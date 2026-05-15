use crate::auth::SharedAuthProvider;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::Compression;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestCompression;
use codex_client::RequestTelemetry;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;

const TOOL_SEARCH_NAME: &str = "tool_search";
const DEEPSEEK_MAX_TOOLS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeepSeekToolName {
    namespace: Option<String>,
    name: String,
}

#[derive(Debug, Default)]
struct DeepSeekChatRequest {
    body: Value,
    tool_name_map: BTreeMap<String, DeepSeekToolName>,
}

struct DeepSeekToolSpec {
    tool: Value,
    tool_name_mapping: Option<(String, DeepSeekToolName)>,
}

/// DeepSeek Chat Completions transport that preserves Codex's internal
/// Responses-style agent loop by mapping chat chunks into [`ResponseEvent`]s.
pub struct DeepSeekChatClient<T: HttpTransport> {
    session: EndpointSession<T>,
    sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

#[derive(Default)]
pub struct DeepSeekChatOptions {
    pub extra_headers: HeaderMap,
    pub compression: Compression,
}

impl<T: HttpTransport> DeepSeekChatClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            sse_telemetry: None,
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            sse_telemetry: sse,
        }
    }

    pub async fn stream_request(
        &self,
        request: ResponsesApiRequest,
        options: DeepSeekChatOptions,
    ) -> Result<ResponseStream, ApiError> {
        let request = deepseek_chat_request_from_responses_request(&request);
        let mut headers = options.extra_headers;
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        let request_compression = match options.compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };
        let stream_response = self
            .session
            .stream_with(
                Method::POST,
                "chat/completions",
                headers,
                Some(request.body),
                |req| {
                    req.compression = request_compression;
                },
            )
            .await?;

        let upstream_request_id = stream_response
            .headers
            .get("x-ds-trace-id")
            .or_else(|| stream_response.headers.get("x-request-id"))
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
        tokio::spawn(process_deepseek_sse(
            stream_response.bytes,
            tx_event,
            self.session.provider().stream_idle_timeout,
            self.sse_telemetry.clone(),
            request.tool_name_map,
        ));
        Ok(ResponseStream {
            rx_event,
            upstream_request_id,
        })
    }
}

fn deepseek_chat_request_from_responses_request(
    request: &ResponsesApiRequest,
) -> DeepSeekChatRequest {
    let mut tool_name_map = BTreeMap::new();
    let messages = deepseek_messages_from_response_items(&request.instructions, &request.input);
    let mut tools = Vec::new();
    append_recent_tools_from_tool_search_outputs(&mut tools, &request.input, &mut tool_name_map);
    append_deepseek_tools_from_responses_tools(&mut tools, &request.tools, &mut tool_name_map);
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String(request.tool_choice.clone());
    }

    if let Some(reasoning) = &request.reasoning {
        if let Some(effort) = reasoning.effort
            && let Some(effort) = deepseek_reasoning_effort_value(effort)
        {
            body["reasoning_effort"] = Value::String(effort.to_string());
        }
        body["thinking"] = json!({ "type": "enabled" });
    }

    if request
        .text
        .as_ref()
        .and_then(|text| text.format.as_ref())
        .is_some()
    {
        // DeepSeek supports JSON object mode here, not OpenAI's JSON-schema
        // response_format shape. The schema remains prompt-side guidance.
        body["response_format"] = json!({ "type": "json_object" });
    }

    DeepSeekChatRequest {
        body,
        tool_name_map,
    }
}

fn deepseek_reasoning_effort_value(effort: ReasoningEffortConfig) -> Option<&'static str> {
    match effort {
        ReasoningEffortConfig::XHigh => Some("max"),
        ReasoningEffortConfig::High
        | ReasoningEffortConfig::Medium
        | ReasoningEffortConfig::Low
        | ReasoningEffortConfig::Minimal => Some("high"),
        ReasoningEffortConfig::None => None,
    }
}

fn deepseek_messages_from_response_items(instructions: &str, items: &[ResponseItem]) -> Vec<Value> {
    let mut messages = Vec::new();
    if !instructions.trim().is_empty() {
        messages.push(json!({
            "role": "system",
            "content": instructions,
        }));
    }

    let mut pending_reasoning: Option<String> = None;
    let mut index = 0;
    while index < items.len() {
        let item = &items[index];
        match item {
            ResponseItem::Message { role, content, .. } => {
                let content = content_items_to_text(content);
                if role == "assistant" {
                    let mut tool_calls = Vec::new();
                    let mut next_index = index + 1;
                    while let Some(item) = items.get(next_index) {
                        let Some(tool_call) = response_item_chat_tool_call(item) else {
                            break;
                        };
                        tool_calls.push(tool_call);
                        next_index += 1;
                    }
                    if tool_calls.is_empty() {
                        messages.push(assistant_message(content, pending_reasoning.take(), None));
                    } else {
                        messages.push(assistant_message(
                            content,
                            pending_reasoning.take(),
                            Some(tool_calls),
                        ));
                        index = next_index;
                        continue;
                    }
                } else {
                    flush_pending_reasoning(&mut messages, &mut pending_reasoning);
                    messages.push(json!({
                        "role": deepseek_message_role(role),
                        "content": content,
                    }));
                }
            }
            ResponseItem::Reasoning {
                summary, content, ..
            } => {
                let reasoning = reasoning_text(summary, content);
                if !reasoning.trim().is_empty() {
                    pending_reasoning = Some(reasoning);
                }
            }
            _ if response_item_chat_tool_call(item).is_some() => {
                let mut tool_calls = Vec::new();
                while let Some(item) = items.get(index) {
                    let Some(tool_call) = response_item_chat_tool_call(item) else {
                        break;
                    };
                    tool_calls.push(tool_call);
                    index += 1;
                }
                messages.push(assistant_message(
                    String::new(),
                    pending_reasoning.take(),
                    Some(tool_calls),
                ));
                continue;
            }
            ResponseItem::FunctionCallOutput { call_id, output }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                flush_pending_reasoning(&mut messages, &mut pending_reasoning);
                messages.push(tool_message(call_id, output));
            }
            ResponseItem::ToolSearchOutput { call_id, tools, .. } => {
                flush_pending_reasoning(&mut messages, &mut pending_reasoning);
                if let Some(call_id) = call_id {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": Value::Array(tools.clone()).to_string(),
                    }));
                }
            }
            _ => {}
        }
        index += 1;
    }
    flush_pending_reasoning(&mut messages, &mut pending_reasoning);
    messages
}

fn deepseek_message_role(role: &str) -> &str {
    match role {
        "developer" => "system",
        role => role,
    }
}

fn assistant_message(
    content: String,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<Value>>,
) -> Value {
    let mut message = json!({
        "role": "assistant",
        "content": content,
    });
    if let Some(reasoning_content) = reasoning_content
        && !reasoning_content.trim().is_empty()
    {
        message["reasoning_content"] = Value::String(reasoning_content);
    }
    if let Some(tool_calls) = tool_calls
        && !tool_calls.is_empty()
    {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    message
}

fn response_item_chat_tool_call(item: &ResponseItem) -> Option<Value> {
    match item {
        ResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            ..
        } => Some(chat_tool_call(
            call_id,
            &deepseek_tool_name(namespace.as_deref(), name),
            arguments,
        )),
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            arguments,
            ..
        } => Some(chat_tool_call(
            call_id,
            TOOL_SEARCH_NAME,
            &arguments.to_string(),
        )),
        ResponseItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => Some(chat_tool_call(call_id, name, input)),
        _ => None,
    }
}

fn flush_pending_reasoning(messages: &mut Vec<Value>, pending_reasoning: &mut Option<String>) {
    if let Some(reasoning) = pending_reasoning.take()
        && !reasoning.trim().is_empty()
    {
        messages.push(assistant_message(String::new(), Some(reasoning), None));
    }
}

fn chat_tool_call(call_id: &str, name: &str, arguments: &str) -> Value {
    json!({
        "id": call_id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments,
        }
    })
}

fn tool_message(call_id: &str, output: &FunctionCallOutputPayload) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": call_id,
        "content": output.body.to_text().unwrap_or_default(),
    })
}

fn content_items_to_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => text.as_str(),
            ContentItem::InputImage { image_url, .. } => image_url.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reasoning_text(
    summary: &[ReasoningItemReasoningSummary],
    content: &Option<Vec<ReasoningItemContent>>,
) -> String {
    let raw = content
        .as_ref()
        .map(|content| {
            content
                .iter()
                .map(|item| match item {
                    ReasoningItemContent::ReasoningText { text }
                    | ReasoningItemContent::Text { text } => text.as_str(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if !raw.trim().is_empty() {
        return raw;
    }
    summary
        .iter()
        .map(|item| match item {
            ReasoningItemReasoningSummary::SummaryText { text } => text.as_str(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn deepseek_tool_name(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(namespace) => format!("{namespace}{name}"),
        None => name.to_string(),
    }
}

fn append_recent_tools_from_tool_search_outputs(
    out: &mut Vec<Value>,
    items: &[ResponseItem],
    tool_name_map: &mut BTreeMap<String, DeepSeekToolName>,
) {
    let start = items
        .iter()
        .rposition(|item| {
            matches!(
                item,
                ResponseItem::Message { role, .. } if role == "user"
            )
        })
        .unwrap_or(0);
    for item in &items[start..] {
        if let ResponseItem::ToolSearchOutput { tools, .. } = item {
            append_deepseek_tools_from_responses_tools(out, tools, tool_name_map);
        }
    }
}

fn append_deepseek_tools_from_responses_tools(
    out: &mut Vec<Value>,
    tools: &[Value],
    tool_name_map: &mut BTreeMap<String, DeepSeekToolName>,
) {
    for tool in tools {
        match tool.get("type").and_then(Value::as_str) {
            Some("function") => {
                if let Some(tool) = chat_tool_from_responses_function(None, tool)
                    && push_chat_tool_unique(out, tool.tool)
                    && let Some((name, mapped)) = tool.tool_name_mapping
                {
                    tool_name_map.insert(name, mapped);
                }
            }
            Some("namespace") => {
                if let (Some(namespace), Some(namespace_tools)) = (
                    tool.get("name").and_then(Value::as_str),
                    tool.get("tools").and_then(Value::as_array),
                ) {
                    for namespace_tool in namespace_tools {
                        if let Some(tool) =
                            chat_tool_from_responses_function(Some(namespace), namespace_tool)
                            && push_chat_tool_unique(out, tool.tool)
                            && let Some((name, mapped)) = tool.tool_name_mapping
                        {
                            tool_name_map.insert(name, mapped);
                        }
                    }
                }
            }
            Some("tool_search") => {
                if let Some(tool) = chat_tool_from_responses_tool_search(tool) {
                    push_chat_tool_unique(out, tool);
                }
            }
            _ => {}
        }
    }
}

fn push_chat_tool_unique(out: &mut Vec<Value>, tool: Value) -> bool {
    if out.len() >= DEEPSEEK_MAX_TOOLS {
        return false;
    }
    let Some(name) = tool.pointer("/function/name").and_then(Value::as_str) else {
        return false;
    };
    if out
        .iter()
        .any(|existing| existing.pointer("/function/name").and_then(Value::as_str) == Some(name))
    {
        return false;
    }
    out.push(tool);
    true
}

fn chat_tool_from_responses_function(
    namespace: Option<&str>,
    tool: &Value,
) -> Option<DeepSeekToolSpec> {
    let name = tool.get("name")?.as_str()?;
    let deepseek_name = deepseek_tool_name(namespace, name);
    let tool_name_mapping = namespace.map(|namespace| {
        (
            deepseek_name.clone(),
            DeepSeekToolName {
                namespace: Some(namespace.to_string()),
                name: name.to_string(),
            },
        )
    });
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parameters = tool
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    let chat_tool = json!({
        "type": "function",
        "function": {
            "name": deepseek_name,
            "description": description,
            "parameters": parameters,
        }
    });
    // DeepSeek strict-mode is a beta endpoint/schema mode. Keep the stable
    // provider path on ordinary function calling and do not forward Responses
    // strict hints.
    Some(DeepSeekToolSpec {
        tool: chat_tool,
        tool_name_mapping,
    })
}

fn chat_tool_from_responses_tool_search(tool: &Value) -> Option<Value> {
    let execution = tool
        .get("execution")
        .and_then(Value::as_str)
        .unwrap_or("client");
    if execution != "client" {
        return None;
    }
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("Search the available tools and return matching tool definitions.");
    let parameters = tool
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    Some(json!({
        "type": "function",
        "function": {
            "name": TOOL_SEARCH_NAME,
            "description": description,
            "parameters": parameters,
        }
    }))
}

async fn process_deepseek_sse(
    stream: codex_client::ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: std::time::Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_name_map: BTreeMap<String, DeepSeekToolName>,
) {
    let mut stream = stream.eventsource();
    let mut accumulator = DeepSeekStreamAccumulator::new(tool_name_map);
    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(err))) => {
                debug!("DeepSeek SSE error: {err:#}");
                let _ = tx_event.send(Err(ApiError::Stream(err.to_string()))).await;
                return;
            }
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "DeepSeek stream closed before [DONE]".into(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("DeepSeek stream idle timeout".into())))
                    .await;
                return;
            }
        };

        if sse.data == "[DONE]" {
            for event in accumulator.finish() {
                let _ = tx_event.send(Ok(event)).await;
            }
            return;
        }

        let chunk = match serde_json::from_str::<DeepSeekChunk>(&sse.data) {
            Ok(chunk) => chunk,
            Err(err) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!(
                        "failed to parse DeepSeek chunk: {err}"
                    ))))
                    .await;
                return;
            }
        };
        for event in accumulator.apply_chunk(chunk) {
            let _ = tx_event.send(Ok(event)).await;
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeepSeekChunk {
    id: Option<String>,
    #[serde(default)]
    choices: Vec<DeepSeekChoice>,
    usage: Option<DeepSeekUsage>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    delta: DeepSeekDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct DeepSeekDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeepSeekToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<DeepSeekFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct DeepSeekUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    prompt_tokens_details: Option<DeepSeekPromptTokenDetails>,
    completion_tokens_details: Option<DeepSeekCompletionTokenDetails>,
    prompt_cache_hit_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekPromptTokenDetails {
    cached_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DeepSeekCompletionTokenDetails {
    reasoning_tokens: Option<i64>,
}

impl From<DeepSeekUsage> for TokenUsage {
    fn from(usage: DeepSeekUsage) -> Self {
        TokenUsage {
            input_tokens: usage.prompt_tokens.unwrap_or_default(),
            cached_input_tokens: usage
                .prompt_cache_hit_tokens
                .or_else(|| {
                    usage
                        .prompt_tokens_details
                        .and_then(|details| details.cached_tokens)
                })
                .unwrap_or_default(),
            output_tokens: usage.completion_tokens.unwrap_or_default(),
            reasoning_output_tokens: usage
                .completion_tokens_details
                .and_then(|details| details.reasoning_tokens)
                .unwrap_or_default(),
            total_tokens: usage.total_tokens.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Default)]
struct DeepSeekToolCallState {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct DeepSeekStreamAccumulator {
    tool_name_map: BTreeMap<String, DeepSeekToolName>,
    response_id: Option<String>,
    content: String,
    reasoning: String,
    tool_calls: BTreeMap<usize, DeepSeekToolCallState>,
    usage: Option<TokenUsage>,
    finish_reason: Option<String>,
    reasoning_item_added: bool,
    message_item_added: bool,
}

impl DeepSeekStreamAccumulator {
    fn new(tool_name_map: BTreeMap<String, DeepSeekToolName>) -> Self {
        Self {
            tool_name_map,
            ..Self::default()
        }
    }

    fn apply_chunk(&mut self, chunk: DeepSeekChunk) -> Vec<ResponseEvent> {
        if self.response_id.is_none() {
            self.response_id = chunk.id;
        }
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage.into());
        }

        let mut events = Vec::new();
        for choice in chunk.choices {
            if let Some(reasoning_content) = choice.delta.reasoning_content
                && !reasoning_content.is_empty()
            {
                if !self.reasoning_item_added {
                    events.push(ResponseEvent::OutputItemAdded(self.reasoning_item()));
                    self.reasoning_item_added = true;
                }
                self.reasoning.push_str(&reasoning_content);
                events.push(ResponseEvent::ReasoningContentDelta {
                    delta: reasoning_content,
                    content_index: 0,
                });
            }

            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                if !self.message_item_added {
                    events.push(ResponseEvent::OutputItemAdded(self.message_item()));
                    self.message_item_added = true;
                }
                self.content.push_str(&content);
                events.push(ResponseEvent::OutputTextDelta(content));
            }

            for tool_call in choice.delta.tool_calls {
                let state = self.tool_calls.entry(tool_call.index).or_default();
                if let Some(id) = tool_call.id {
                    state.id = Some(id);
                }
                if let Some(function) = tool_call.function {
                    if let Some(name) = function.name {
                        state.name.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        state.arguments.push_str(&arguments);
                    }
                }
            }

            if let Some(finish_reason) = choice.finish_reason {
                self.finish_reason = Some(finish_reason);
            }
        }
        events
    }

    fn finish(&mut self) -> Vec<ResponseEvent> {
        let mut events = Vec::new();
        if self.reasoning_item_added || !self.reasoning.is_empty() {
            events.push(ResponseEvent::OutputItemDone(self.reasoning_item()));
        }
        if self.message_item_added || !self.content.is_empty() {
            events.push(ResponseEvent::OutputItemDone(self.message_item()));
        }
        for (index, tool_call) in &self.tool_calls {
            if !tool_call.name.trim().is_empty() {
                let call_id = tool_call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("call_{index}"));
                let item = if tool_call.name == TOOL_SEARCH_NAME {
                    ResponseItem::ToolSearchCall {
                        id: None,
                        call_id: Some(call_id),
                        status: None,
                        execution: "client".to_string(),
                        arguments: serde_json::from_str(&tool_call.arguments)
                            .unwrap_or_else(|_| Value::String(tool_call.arguments.clone())),
                    }
                } else {
                    let tool_name = self
                        .tool_name_map
                        .get(&tool_call.name)
                        .cloned()
                        .unwrap_or_else(|| DeepSeekToolName {
                            namespace: None,
                            name: tool_call.name.clone(),
                        });
                    ResponseItem::FunctionCall {
                        id: None,
                        name: tool_name.name,
                        namespace: tool_name.namespace,
                        arguments: tool_call.arguments.clone(),
                        call_id,
                    }
                };
                events.push(ResponseEvent::OutputItemDone(item));
            }
        }
        let end_turn = match self.finish_reason.as_deref() {
            Some("tool_calls") => Some(false),
            Some(_) => Some(true),
            None => None,
        };
        events.push(ResponseEvent::Completed {
            response_id: self
                .response_id
                .clone()
                .unwrap_or_else(|| "deepseek_chat_completion".to_string()),
            token_usage: self.usage.clone(),
            end_turn,
        });
        events
    }

    fn reasoning_item(&self) -> ResponseItem {
        ResponseItem::Reasoning {
            id: self
                .response_id
                .clone()
                .unwrap_or_else(|| "deepseek_reasoning".to_string()),
            summary: Vec::new(),
            content: Some(vec![ReasoningItemContent::Text {
                text: self.reasoning.clone(),
            }]),
            encrypted_content: None,
        }
    }

    fn message_item(&self) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: self.content.clone(),
            }],
            phase: Some(MessagePhase::FinalAnswer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Reasoning;
    use crate::common::TextControls;
    use crate::common::TextFormat;
    use crate::common::TextFormatType;
    use pretty_assertions::assert_eq;

    #[test]
    fn converts_responses_request_to_deepseek_chat_request() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: "system text".to_string(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "hello".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::Reasoning {
                    id: "rs_1".to_string(),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::Text {
                        text: "think".to_string(),
                    }]),
                    encrypted_content: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "developer guidance".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "shell".to_string(),
                    namespace: None,
                    arguments: "{\"cmd\":\"pwd\"}".to_string(),
                    call_id: "call_1".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call_1".to_string(),
                    output: FunctionCallOutputPayload::from_text("ok".to_string()),
                },
                ResponseItem::ToolSearchCall {
                    id: None,
                    call_id: Some("call_search".to_string()),
                    status: None,
                    execution: "client".to_string(),
                    arguments: json!({ "query": "scout resolve", "limit": 2 }),
                },
                ResponseItem::ToolSearchOutput {
                    call_id: Some("call_search".to_string()),
                    status: "completed".to_string(),
                    execution: "client".to_string(),
                    tools: vec![json!({
                        "type": "namespace",
                        "name": "mcp__scout__",
                        "description": "Resolve Codez facts",
                        "tools": [{
                            "type": "function",
                            "name": "resolve",
                            "description": "Resolve Codez facts",
                            "parameters": { "type": "object", "properties": {} }
                        }]
                    })],
                },
            ],
            tools: vec![
                json!({
                    "type": "function",
                    "name": "shell",
                    "description": "Run shell",
                    "strict": true,
                    "parameters": { "type": "object", "properties": {} }
                }),
                json!({
                    "type": "tool_search",
                    "execution": "client",
                    "description": "Search tools",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        },
                        "required": ["query"]
                    }
                }),
            ],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: Some(TextControls {
                verbosity: None,
                format: Some(TextFormat {
                    r#type: TextFormatType::JsonSchema,
                    strict: true,
                    schema: json!({
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"]
                    }),
                    name: "answer_schema".to_string(),
                }),
            }),
            client_metadata: None,
        };

        let request = deepseek_chat_request_from_responses_request(&request);
        let body = request.body;

        assert_eq!(body["model"], "deepseek-v4-pro");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["messages"][2]["reasoning_content"], "think");
        assert_eq!(body["messages"][3]["role"], "system");
        assert_eq!(body["messages"][4]["tool_calls"][0]["id"], "call_1");
        assert_eq!(body["messages"][5]["role"], "tool");
        assert_eq!(
            body["messages"][6]["tool_calls"][0]["function"]["name"],
            "tool_search"
        );
        assert_eq!(body["messages"][7]["tool_call_id"], "call_search");
        let tools = body["tools"].as_array().expect("tools array");
        let shell_tool = tools
            .iter()
            .find(|tool| tool["function"]["name"] == "shell")
            .expect("shell tool should be exposed");
        assert!(shell_tool["function"].get("strict").is_none());
        assert!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "tool_search")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "mcp__scout__resolve")
        );
        assert_eq!(
            request.tool_name_map.get("mcp__scout__resolve"),
            Some(&DeepSeekToolName {
                namespace: Some("mcp__scout__".to_string()),
                name: "resolve".to_string(),
            })
        );
        assert_eq!(body["response_format"], json!({ "type": "json_object" }));
        assert!(body["response_format"].get("schema").is_none());
    }

    #[test]
    fn maps_codex_reasoning_effort_to_deepseek_values() {
        for (effort, expected) in [
            (ReasoningEffortConfig::High, Some("high")),
            (ReasoningEffortConfig::Medium, Some("high")),
            (ReasoningEffortConfig::Low, Some("high")),
            (ReasoningEffortConfig::Minimal, Some("high")),
            (ReasoningEffortConfig::XHigh, Some("max")),
            (ReasoningEffortConfig::None, None),
        ] {
            let request = ResponsesApiRequest {
                model: "deepseek-v4-pro".to_string(),
                instructions: String::new(),
                input: vec![ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "hello".to_string(),
                    }],
                    phase: None,
                }],
                tools: Vec::new(),
                tool_choice: "auto".to_string(),
                parallel_tool_calls: false,
                reasoning: Some(Reasoning {
                    effort: Some(effort),
                    summary: None,
                }),
                store: false,
                stream: true,
                include: Vec::new(),
                service_tier: None,
                prompt_cache_key: None,
                text: None,
                client_metadata: None,
            };

            let body = deepseek_chat_request_from_responses_request(&request).body;
            assert_eq!(body["thinking"], json!({ "type": "enabled" }));
            if let Some(expected) = expected {
                assert_eq!(body["reasoning_effort"], expected);
            } else {
                assert!(body.get("reasoning_effort").is_none());
            }
        }
    }

    #[test]
    fn groups_consecutive_tool_calls_in_one_chat_assistant_message() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "use two tools".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "resolve".to_string(),
                    namespace: Some("mcp__scout__".to_string()),
                    arguments: "{\"query\":\"gateway\"}".to_string(),
                    call_id: "call_scout".to_string(),
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "exec_command".to_string(),
                    namespace: None,
                    arguments: "{\"cmd\":\"pwd\"}".to_string(),
                    call_id: "call_pwd".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call_scout".to_string(),
                    output: FunctionCallOutputPayload::from_text("scout ok".to_string()),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call_pwd".to_string(),
                    output: FunctionCallOutputPayload::from_text("pwd ok".to_string()),
                },
            ],
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };

        let body = deepseek_chat_request_from_responses_request(&request).body;
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["name"],
            "mcp__scout__resolve"
        );
        assert_eq!(
            messages[1]["tool_calls"][1]["function"]["name"],
            "exec_command"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_scout");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_pwd");
    }

    #[test]
    fn groups_assistant_message_and_following_tool_calls() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "use skill then scout".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::Reasoning {
                    id: "rs_1".to_string(),
                    summary: Vec::new(),
                    content: Some(vec![ReasoningItemContent::Text {
                        text: "read skill, then call scout".to_string(),
                    }]),
                    encrypted_content: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "I found the lookup skill.".to_string(),
                    }],
                    phase: Some(MessagePhase::Commentary),
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "resolve".to_string(),
                    namespace: Some("mcp__scout__".to_string()),
                    arguments: "{\"query\":\"gateway\"}".to_string(),
                    call_id: "call_scout".to_string(),
                },
                ResponseItem::FunctionCallOutput {
                    call_id: "call_scout".to_string(),
                    output: FunctionCallOutputPayload::from_text("scout ok".to_string()),
                },
            ],
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };

        let body = deepseek_chat_request_from_responses_request(&request).body;
        let messages = body["messages"].as_array().expect("messages array");

        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "I found the lookup skill.");
        assert_eq!(
            messages[1]["reasoning_content"],
            "read skill, then call scout"
        );
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["name"],
            "mcp__scout__resolve"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_scout");
    }

    #[test]
    fn maps_responses_namespaces_to_flat_deepseek_tools_and_back() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: vec![ResponseItem::FunctionCall {
                id: None,
                name: "resolve".to_string(),
                namespace: Some("mcp__scout__".to_string()),
                arguments: "{\"query\":\"codez\"}".to_string(),
                call_id: "call_1".to_string(),
            }],
            tools: vec![json!({
                "type": "namespace",
                "name": "mcp__scout__",
                "description": "Scout tools",
                "tools": [{
                    "type": "function",
                    "name": "resolve",
                    "description": "Resolve Codez facts",
                    "parameters": { "type": "object", "properties": {} }
                }]
            })],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };
        let request = deepseek_chat_request_from_responses_request(&request);

        assert_eq!(
            request.body["messages"][0]["tool_calls"][0]["function"]["name"],
            "mcp__scout__resolve"
        );
        assert_eq!(
            request.body["tools"][0]["function"]["name"],
            "mcp__scout__resolve"
        );

        let mut state = DeepSeekStreamAccumulator::new(request.tool_name_map);
        state.apply_chunk(DeepSeekChunk {
            id: Some("chatcmpl-mcp".to_string()),
            choices: vec![DeepSeekChoice {
                delta: DeepSeekDelta {
                    tool_calls: vec![DeepSeekToolCallDelta {
                        index: 0,
                        id: Some("call_2".to_string()),
                        function: Some(DeepSeekFunctionDelta {
                            name: Some("mcp__scout__resolve".to_string()),
                            arguments: Some("{\"query\":\"codez\"}".to_string()),
                        }),
                    }],
                    ..DeepSeekDelta::default()
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        });

        let events = state.finish();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                    namespace: Some(namespace),
                    name,
                    call_id,
                    ..
                }) if namespace == "mcp__scout__" && name == "resolve" && call_id == "call_2"
            )
        }));
    }

    #[test]
    fn ignores_tool_search_outputs_from_previous_user_turns() {
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "old".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::ToolSearchOutput {
                    call_id: Some("old_search".to_string()),
                    status: "completed".to_string(),
                    execution: "client".to_string(),
                    tools: vec![json!({
                        "type": "namespace",
                        "name": "mcp__old__",
                        "description": "Old tools",
                        "tools": [{
                            "type": "function",
                            "name": "stale",
                            "description": "Stale tool",
                            "parameters": { "type": "object", "properties": {} }
                        }]
                    })],
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "new".to_string(),
                    }],
                    phase: None,
                },
            ],
            tools: Vec::new(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };

        let request = deepseek_chat_request_from_responses_request(&request);
        assert!(request.body.get("tools").is_none());
        assert!(request.tool_name_map.is_empty());
    }

    #[test]
    fn caps_and_deduplicates_deepseek_tools_with_matching_name_map() {
        let namespace_tools = (0..130)
            .map(|index| {
                json!({
                    "type": "function",
                    "name": format!("tool_{index:03}"),
                    "description": "Bulk test tool",
                    "parameters": { "type": "object", "properties": {} }
                })
            })
            .chain(std::iter::once(json!({
                "type": "function",
                "name": "tool_000",
                "description": "Duplicate bulk tool",
                "parameters": { "type": "object", "properties": {} }
            })))
            .collect::<Vec<_>>();
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: Vec::new(),
            tools: vec![json!({
                "type": "namespace",
                "name": "mcp__bulk__",
                "description": "Bulk namespace",
                "tools": namespace_tools,
            })],
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };

        let request = deepseek_chat_request_from_responses_request(&request);
        let tools = request.body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), DEEPSEEK_MAX_TOOLS);
        assert_eq!(
            tools
                .iter()
                .filter(|tool| tool["function"]["name"] == "mcp__bulk__tool_000")
                .count(),
            1
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "mcp__bulk__tool_127")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool["function"]["name"] == "mcp__bulk__tool_128")
        );
        assert!(request.tool_name_map.contains_key("mcp__bulk__tool_127"));
        assert!(!request.tool_name_map.contains_key("mcp__bulk__tool_128"));
    }

    #[test]
    fn prioritizes_recent_tool_search_results_over_bulk_direct_tools() {
        let direct_tools = (0..130)
            .map(|index| {
                json!({
                    "type": "function",
                    "name": format!("direct_{index:03}"),
                    "description": "Bulk direct tool",
                    "parameters": { "type": "object", "properties": {} }
                })
            })
            .collect::<Vec<_>>();
        let request = ResponsesApiRequest {
            model: "deepseek-v4-pro".to_string(),
            instructions: String::new(),
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "find scout".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::ToolSearchOutput {
                    call_id: Some("search_1".to_string()),
                    status: "completed".to_string(),
                    execution: "client".to_string(),
                    tools: vec![json!({
                        "type": "namespace",
                        "name": "mcp__scout__",
                        "description": "Scout tools",
                        "tools": [{
                            "type": "function",
                            "name": "resolve",
                            "description": "Resolve Codez facts",
                            "parameters": { "type": "object", "properties": {} }
                        }]
                    })],
                },
            ],
            tools: direct_tools,
            tool_choice: "auto".to_string(),
            parallel_tool_calls: false,
            reasoning: None,
            store: false,
            stream: true,
            include: Vec::new(),
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            client_metadata: None,
        };

        let request = deepseek_chat_request_from_responses_request(&request);
        let tools = request.body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), DEEPSEEK_MAX_TOOLS);
        assert_eq!(tools[0]["function"]["name"], "mcp__scout__resolve");
        assert!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "direct_126")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool["function"]["name"] == "direct_127")
        );
        assert_eq!(
            request.tool_name_map.get("mcp__scout__resolve"),
            Some(&DeepSeekToolName {
                namespace: Some("mcp__scout__".to_string()),
                name: "resolve".to_string(),
            })
        );
    }

    #[test]
    fn maps_deepseek_stream_chunks_to_response_events() {
        let mut state = DeepSeekStreamAccumulator::default();
        let events = state.apply_chunk(DeepSeekChunk {
            id: Some("chatcmpl_1".to_string()),
            choices: vec![DeepSeekChoice {
                delta: DeepSeekDelta {
                    reasoning_content: Some("reason".to_string()),
                    ..DeepSeekDelta::default()
                },
                finish_reason: None,
            }],
            usage: None,
        });
        assert!(matches!(
            events.as_slice(),
            [
                ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. }),
                ResponseEvent::ReasoningContentDelta { .. }
            ]
        ));

        let events = state.apply_chunk(DeepSeekChunk {
            id: Some("chatcmpl_1".to_string()),
            choices: vec![DeepSeekChoice {
                delta: DeepSeekDelta {
                    content: Some("hello".to_string()),
                    tool_calls: vec![DeepSeekToolCallDelta {
                        index: 0,
                        id: Some("call_1".to_string()),
                        function: Some(DeepSeekFunctionDelta {
                            name: Some("shell".to_string()),
                            arguments: Some("{\"cmd\":\"pwd\"}".to_string()),
                        }),
                    }],
                    ..DeepSeekDelta::default()
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: Some(DeepSeekUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
                prompt_cache_hit_tokens: Some(7),
                ..DeepSeekUsage::default()
            }),
        });
        assert!(matches!(
            events.as_slice(),
            [
                ResponseEvent::OutputItemAdded(ResponseItem::Message { .. }),
                ResponseEvent::OutputTextDelta(_)
            ]
        ));

        let events = state.finish();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                    name,
                    call_id,
                    ..
                }) if name == "shell" && call_id == "call_1"
            )
        }));
        assert!(matches!(
            events.last(),
            Some(ResponseEvent::Completed {
                token_usage: Some(TokenUsage {
                    total_tokens: 15,
                    cached_input_tokens: 7,
                    ..
                }),
                end_turn: Some(false),
                ..
            })
        ));
    }

    #[test]
    fn maps_split_tool_search_call_to_response_tool_search_item() {
        let mut state = DeepSeekStreamAccumulator::default();
        state.apply_chunk(DeepSeekChunk {
            id: Some("chatcmpl-search".to_string()),
            choices: vec![DeepSeekChoice {
                delta: DeepSeekDelta {
                    tool_calls: vec![DeepSeekToolCallDelta {
                        index: 0,
                        id: Some("call_search".to_string()),
                        function: Some(DeepSeekFunctionDelta {
                            name: Some("tool_".to_string()),
                            arguments: Some("{\"query\":\"sc".to_string()),
                        }),
                    }],
                    ..DeepSeekDelta::default()
                },
                finish_reason: None,
            }],
            usage: None,
        });
        state.apply_chunk(DeepSeekChunk {
            id: Some("chatcmpl-search".to_string()),
            choices: vec![DeepSeekChoice {
                delta: DeepSeekDelta {
                    tool_calls: vec![DeepSeekToolCallDelta {
                        index: 0,
                        id: None,
                        function: Some(DeepSeekFunctionDelta {
                            name: Some("search".to_string()),
                            arguments: Some("out\",\"limit\":1}".to_string()),
                        }),
                    }],
                    ..DeepSeekDelta::default()
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        });

        let events = state.finish();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ResponseEvent::OutputItemDone(ResponseItem::ToolSearchCall {
                    call_id: Some(call_id),
                    execution,
                    arguments,
                    ..
                }) if call_id == "call_search"
                    && execution == "client"
                    && arguments["query"] == "scout"
                    && arguments["limit"] == 1
            )
        }));
        assert!(matches!(
            events.last(),
            Some(ResponseEvent::Completed {
                end_turn: Some(false),
                ..
            })
        ));
    }
}
