#![cfg(not(target_os = "windows"))]

use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use codex_config::types::McpServerConfig;
use codex_config::types::McpServerTransportConfig;
use codex_core::config::Config;
use codex_features::Feature;
use codex_model_provider_info::WireApi;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::stdio_server_bin;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use wiremock::Match;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::Respond;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path_regex;

#[derive(Debug, Clone, Default)]
struct ChatRequestLog {
    requests: Arc<Mutex<Vec<Value>>>,
}

impl ChatRequestLog {
    fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Match for ChatRequestLog {
    fn matches(&self, request: &wiremock::Request) -> bool {
        let body = serde_json::from_slice::<Value>(&request.body)
            .unwrap_or_else(|err| panic!("DeepSeek chat request body should be JSON: {err}"));
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(body);
        true
    }
}

struct ChatSseSequence {
    calls: std::sync::atomic::AtomicUsize,
    bodies: Vec<String>,
}

fn configure_deepseek_chat_provider(config: &mut Config, base_url: String) {
    config.model_provider.name = "DeepSeek test".to_string();
    config.model_provider.base_url = Some(base_url);
    config.model_provider.env_key = Some("PATH".to_string());
    config.model_provider.experimental_bearer_token = None;
    config.model_provider.requires_openai_auth = false;
    config.model_provider.supports_websockets = false;
    config.model_provider.wire_api = WireApi::DeepSeekChat;
    config.model_provider.request_max_retries = Some(0);
    config.model_provider.stream_max_retries = Some(0);
}

fn insert_stdio_mcp_server(config: &mut Config, server_name: &str, command: String) {
    let mut servers = config.mcp_servers.get().clone();
    servers.insert(
        server_name.to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command,
                args: Vec::new(),
                env: Some(HashMap::from([(
                    "MCP_TEST_VALUE".to_string(),
                    "deepseek-mcp-ok".to_string(),
                )])),
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: Some(Duration::from_secs(10)),
            tool_timeout_sec: Some(Duration::from_secs(10)),
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    );
    if let Err(err) = config.mcp_servers.set(servers) {
        panic!("test MCP server config should be accepted: {err}");
    }
}

impl Respond for ChatSseSequence {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let call_num = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = self
            .bodies
            .get(call_num)
            .unwrap_or_else(|| panic!("no DeepSeek chat response for request {call_num}"));
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body.clone())
    }
}

async fn mount_deepseek_chat_sequence(server: &MockServer, bodies: Vec<String>) -> ChatRequestLog {
    let log = ChatRequestLog::default();
    let num_calls = bodies.len();
    let responder = ChatSseSequence {
        calls: std::sync::atomic::AtomicUsize::new(0),
        bodies,
    };
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .and(log.clone())
        .respond_with(responder)
        .up_to_n_times(num_calls as u64)
        .expect(num_calls as u64)
        .mount(server)
        .await;
    log
}

async fn mount_deepseek_chat_sequence_unverified(
    server: &MockServer,
    bodies: Vec<String>,
) -> ChatRequestLog {
    let log = ChatRequestLog::default();
    let num_calls = bodies.len();
    let responder = ChatSseSequence {
        calls: std::sync::atomic::AtomicUsize::new(0),
        bodies,
    };
    Mock::given(method("POST"))
        .and(path_regex(".*/chat/completions$"))
        .and(log.clone())
        .respond_with(responder)
        .up_to_n_times(num_calls as u64)
        .mount(server)
        .await;
    log
}

fn deepseek_sse(events: Vec<Value>) -> String {
    let mut out = String::new();
    for event in events {
        out.push_str("data: ");
        out.push_str(&event.to_string());
        out.push_str("\n\n");
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_preserves_codex_tool_loop() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let command_args = json!({
        "cmd": "/bin/echo deepseek codex",
    });
    let first_response = deepseek_sse(vec![
        json!({
            "id": "chatcmpl-1",
            "choices": [{
                "delta": { "reasoning_content": "Need to run the shell tool." },
                "finish_reason": null
            }]
        }),
        json!({
            "id": "chatcmpl-1",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_deepseek_shell",
                        "function": {
                            "name": "exec_command",
                            "arguments": command_args.to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }),
    ]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-2",
        "choices": [{
            "delta": {
                "reasoning_content": "The command output is available.",
                "content": "done"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 4,
            "total_tokens": 24,
            "prompt_cache_hit_tokens": 10,
            "completion_tokens_details": { "reasoning_tokens": 2 }
        }
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("gpt-5.4").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "run the DeepSeek Codex shell smoke",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["model"], "gpt-5.4");
    assert_eq!(requests[0]["stream"], true);
    assert_eq!(requests[0]["stream_options"]["include_usage"], true);
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "exec_command"),
        "first DeepSeek chat request should expose the Codex exec_command tool: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );

    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "assistant"
                && message["reasoning_content"] == "Need to run the shell tool."
                && message["tool_calls"][0]["id"] == "call_deepseek_shell"
        }),
        "follow-up request should preserve DeepSeek reasoning and assistant tool call"
    );
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == "call_deepseek_shell"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("deepseek codex"))
        }),
        "follow-up request should send the real shell output back as a tool message"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_returns_malformed_tool_arguments_to_model() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "call_deepseek_bad_plan";
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-bad-args-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": "update_plan",
                        "arguments": "{not-json"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-bad-args-2",
        "choices": [{
            "delta": { "content": "recovered" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-flash").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "simulate malformed tool arguments",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == call_id
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("failed to parse function arguments"))
        }),
        "malformed arguments should be returned to DeepSeek as a tool error message: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_handles_large_tool_output() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "call_deepseek_large_stdout";
    let command_args = json!({
        "cmd": "/bin/sh -c 'i=0; while [ $i -lt 12000 ]; do printf X; i=$((i+1)); done; printf LARGE_END'",
    });
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-large-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": "exec_command",
                        "arguments": command_args.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-large-2",
        "choices": [{
            "delta": { "content": "large output handled" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-flash").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "simulate large shell output",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    let tool_content = second_messages
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    assert!(
        tool_content.len() > 8_000,
        "large tool output should not disappear before the DeepSeek follow-up request; got {} bytes",
        tool_content.len()
    );
    assert!(
        tool_content.contains("LARGE_END"),
        "large tool output should preserve the tail marker: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_handles_multiple_tool_calls_in_one_turn() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let first_call_id = "call_deepseek_multi_one";
    let second_call_id = "call_deepseek_multi_two";
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-multi-1",
        "choices": [{
            "delta": {
                "tool_calls": [
                    {
                        "index": 0,
                        "id": first_call_id,
                        "function": {
                            "name": "exec_command",
                            "arguments": json!({ "cmd": "/bin/echo multi-one" }).to_string()
                        }
                    },
                    {
                        "index": 1,
                        "id": second_call_id,
                        "function": {
                            "name": "exec_command",
                            "arguments": json!({ "cmd": "/bin/echo multi-two" }).to_string()
                        }
                    }
                ]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-multi-2",
        "choices": [{
            "delta": { "content": "multi handled" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-pro").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "simulate multiple DeepSeek tool calls",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    let assistant_tool_index = second_messages
        .iter()
        .position(|message| {
            message["role"] == "assistant"
                && message["tool_calls"]
                    .as_array()
                    .is_some_and(|tool_calls| tool_calls.len() == 2)
        })
        .expect("parallel-style tool calls should be grouped in one assistant message");
    assert_eq!(
        second_messages[assistant_tool_index + 1]["tool_call_id"],
        first_call_id
    );
    assert_eq!(
        second_messages[assistant_tool_index + 2]["tool_call_id"],
        second_call_id
    );
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == first_call_id
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("multi-one"))
        }),
        "first parallel-style tool output should return to DeepSeek: {}",
        serde_json::to_string_pretty(second_messages)?
    );
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == second_call_id
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("multi-two"))
        }),
        "second parallel-style tool output should return to DeepSeek: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_retries_stream_closed_before_done() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let partial_response_without_done = "data: {\"id\":\"chatcmpl-retry-1\",\"choices\":[{\"delta\":{\"reasoning_content\":\"start\"},\"finish_reason\":null}]}\n\n".to_string();
    let recovered_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-retry-2",
        "choices": [{
            "delta": { "content": "retry recovered" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 15,
            "completion_tokens": 3,
            "total_tokens": 18
        }
    })]);
    let chat_log = mount_deepseek_chat_sequence(
        &server,
        vec![partial_response_without_done, recovered_response],
    )
    .await;

    let mut builder = test_codex().with_model("deepseek-v4-pro").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
            config.model_provider.stream_max_retries = Some(1);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "simulate dropped DeepSeek stream",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["model"], "deepseek-v4-pro");
    assert_eq!(requests[1]["model"], "deepseek-v4-pro");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_handles_unknown_mcp_placeholder() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "call_deepseek_unknown_mcp";
    let tool_name = "mcp__missing__resolve";
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-missing-mcp-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": tool_name,
                        "arguments": json!({ "query": "codez" }).to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-missing-mcp-2",
        "choices": [{
            "delta": { "content": "placeholder handled" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-flash").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
            config
                .features
                .enable(Feature::UnavailableDummyTools)
                .unwrap_or_else(|err| panic!("unavailable dummy tools should enable: {err}"));
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "simulate unknown MCP tool call",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == tool_name),
        "unknown MCP history should expose a placeholder tool in the follow-up request: {}",
        serde_json::to_string_pretty(&requests[1]["tools"])?
    );
    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == call_id
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("unsupported call"))
        }),
        "first unknown MCP call should return an actionable tool error: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_routes_tool_search_and_injects_found_tool() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let search_args = json!({
        "query": "automation update",
        "limit": 1,
    });
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-search-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_deepseek_tool_search",
                    "function": {
                        "name": "tool_search",
                        "arguments": search_args.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-search-2",
        "choices": [{
            "delta": { "content": "done" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-flash").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let mut test = builder.build(&server).await?;
    let dynamic_tools = vec![DynamicToolSpec {
        namespace: None,
        name: "automation_update".to_string(),
        description: "Create, update, view, or delete recurring automations.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string" }
            },
            "required": ["mode"],
            "additionalProperties": false
        }),
        defer_loading: true,
    }];
    let new_thread = test
        .thread_manager
        .start_thread_with_tools(
            test.config.clone(),
            dynamic_tools,
            /*persist_extended_history*/ false,
        )
        .await?;
    test.codex = new_thread.thread;
    test.session_configured = new_thread.session_configured;

    test.submit_turn_with_permission_profile(
        "find the automation update tool",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "tool_search"),
        "first request should expose tool_search: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| {
                tool["type"] == "function"
                    && tool["function"]["name"] == "apply_patch"
                    && tool["function"]["parameters"]["properties"]["input"]["type"] == "string"
            }),
        "DeepSeek must receive function-shaped apply_patch, not Responses custom/freeform: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );
    assert!(
        !requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "automation_update"),
        "deferred tool should be hidden before search: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );

    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == "call_deepseek_tool_search"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("automation_update"))
        }),
        "follow-up request should include tool_search output"
    );
    assert!(
        requests[1]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "automation_update"),
        "DeepSeek follow-up must expose the discovered tool as a chat function: {}",
        serde_json::to_string_pretty(&requests[1]["tools"])?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_routes_namespaced_mcp_tool() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let server_name = "rmcp_deepseek";
    let namespace = format!("mcp__{server_name}__");
    let call_id = "call_deepseek_mcp_echo";
    let tool_args = json!({
        "message": "ping",
        "env_var": "MCP_TEST_VALUE",
    });
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-mcp-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": format!("{namespace}echo"),
                        "arguments": tool_args.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-mcp-2",
        "choices": [{
            "delta": { "content": "mcp done" },
            "finish_reason": "stop"
        }]
    })]);
    let rmcp_test_server_bin = match stdio_server_bin() {
        Ok(bin) => bin,
        Err(err) => {
            eprintln!(
                "skipping DeepSeek MCP routing test; test_stdio_server is unavailable: {err}"
            );
            return Ok(());
        }
    };
    let chat_log =
        mount_deepseek_chat_sequence_unverified(&server, vec![first_response, second_response])
            .await;

    let mut builder = test_codex().with_model("deepseek-v4-pro").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
            insert_stdio_mcp_server(config, server_name, rmcp_test_server_bin);
        }
    });
    let test = builder.build(&server).await?;

    let submit_result = test
        .submit_turn_with_permission_profile(
            "call the Codez-style namespaced MCP echo tool",
            PermissionProfile::Disabled,
        )
        .await;
    assert!(
        submit_result.is_ok(),
        "DeepSeek MCP turn should complete: {submit_result:?}"
    );

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == format!("{namespace}echo")),
        "first DeepSeek chat request should expose namespaced MCP as a flat function: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );

    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == call_id
                && message["content"].as_str().is_some_and(|content| {
                    content.contains("ECHOING: ping") && content.contains("deepseek-mcp-ok")
                })
        }),
        "follow-up request should send the real MCP output back to DeepSeek: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_discovers_deferred_mcp_tool_then_routes_it() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let server_name = "rmcp_deepseek_deferred";
    let namespace = format!("mcp__{server_name}__");
    let search_call_id = "call_deepseek_mcp_search";
    let mcp_call_id = "call_deepseek_mcp_deferred_echo";
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-mcp-search-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": search_call_id,
                    "function": {
                        "name": "tool_search",
                        "arguments": json!({ "query": "echo rmcp_deepseek_deferred", "limit": 2 }).to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-mcp-search-2",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": mcp_call_id,
                    "function": {
                        "name": format!("{namespace}echo"),
                        "arguments": json!({
                            "message": "deferred-ping",
                            "env_var": "MCP_TEST_VALUE",
                        }).to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let third_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-mcp-search-3",
        "choices": [{
            "delta": { "content": "deferred mcp done" },
            "finish_reason": "stop"
        }]
    })]);
    let rmcp_test_server_bin = match stdio_server_bin() {
        Ok(bin) => bin,
        Err(err) => {
            eprintln!(
                "skipping deferred DeepSeek MCP routing test; test_stdio_server is unavailable: {err}"
            );
            return Ok(());
        }
    };
    let chat_log = mount_deepseek_chat_sequence_unverified(
        &server,
        vec![first_response, second_response, third_response],
    )
    .await;

    let mut builder = test_codex().with_model("deepseek-v4-pro").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
            config
                .features
                .enable(Feature::ToolSearchAlwaysDeferMcpTools)
                .unwrap_or_else(|err| panic!("tool-search MCP deferral should enable: {err}"));
            insert_stdio_mcp_server(config, server_name, rmcp_test_server_bin);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn_with_permission_profile(
        "find and call the deferred Codez-style MCP echo tool",
        PermissionProfile::Disabled,
    )
    .await?;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "tool_search"),
        "first request should expose tool_search: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );
    assert!(
        !requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == format!("{namespace}echo")),
        "deferred MCP tool should not be directly exposed before search: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );
    assert!(
        requests[1]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == format!("{namespace}echo")),
        "DeepSeek follow-up should expose MCP tool found through tool_search: {}",
        serde_json::to_string_pretty(&requests[1]["tools"])?
    );
    let third_messages = requests[2]["messages"]
        .as_array()
        .expect("third request messages");
    assert!(
        third_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == mcp_call_id
                && message["content"].as_str().is_some_and(|content| {
                    content.contains("ECHOING: deferred-ping")
                        && content.contains("deepseek-mcp-ok")
                })
        }),
        "MCP output should return to DeepSeek after deferred discovery: {}",
        serde_json::to_string_pretty(third_messages)?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_chat_wire_api_routes_namespaced_dynamic_tool() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let call_id = "call_deepseek_dynamic";
    let dynamic_args = json!({
        "project": "codez",
    });
    let first_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-dynamic-1",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": "codex_appcodez_status",
                        "arguments": dynamic_args.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })]);
    let second_response = deepseek_sse(vec![json!({
        "id": "chatcmpl-dynamic-2",
        "choices": [{
            "delta": { "content": "dynamic done" },
            "finish_reason": "stop"
        }]
    })]);
    let chat_log =
        mount_deepseek_chat_sequence(&server, vec![first_response, second_response]).await;

    let mut builder = test_codex().with_model("deepseek-v4-flash").with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            configure_deepseek_chat_provider(config, base_url);
        }
    });
    let base_test = builder.build(&server).await?;
    let new_thread = base_test
        .thread_manager
        .start_thread_with_tools(
            base_test.config.clone(),
            vec![DynamicToolSpec {
                namespace: Some("codex_app".to_string()),
                name: "codez_status".to_string(),
                description: "Return Codez topic status for Telegram UI.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "project": { "type": "string" }
                    },
                    "required": ["project"],
                    "additionalProperties": false
                }),
                defer_loading: false,
            }],
            /*persist_extended_history*/ false,
        )
        .await?;
    let mut test = base_test;
    test.codex = new_thread.thread;
    test.session_configured = new_thread.session_configured;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![codex_protocol::user_input::UserInput::Text {
                text: "simulate Telegram custom tool call".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;

    let EventMsg::DynamicToolCallRequest(request) = wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::DynamicToolCallRequest(_))
    })
    .await
    else {
        unreachable!("event guard guarantees DynamicToolCallRequest");
    };
    assert_eq!(request.call_id, call_id);
    assert_eq!(request.namespace.as_deref(), Some("codex_app"));
    assert_eq!(request.tool, "codez_status");
    assert_eq!(request.arguments, dynamic_args);

    test.codex
        .submit(Op::DynamicToolResponse {
            id: request.call_id,
            response: DynamicToolResponse {
                content_items: vec![DynamicToolCallOutputContentItem::InputText {
                    text: "dynamic-codez-ok".to_string(),
                }],
                success: true,
            },
        })
        .await?;

    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let requests = chat_log.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["function"]["name"] == "codex_appcodez_status"),
        "first DeepSeek request should expose the namespaced custom tool: {}",
        serde_json::to_string_pretty(&requests[0]["tools"])?
    );
    let second_messages = requests[1]["messages"]
        .as_array()
        .expect("second request messages");
    assert!(
        second_messages.iter().any(|message| {
            message["role"] == "tool"
                && message["tool_call_id"] == call_id
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("dynamic-codez-ok"))
        }),
        "follow-up request should include dynamic tool output: {}",
        serde_json::to_string_pretty(second_messages)?
    );

    Ok(())
}
