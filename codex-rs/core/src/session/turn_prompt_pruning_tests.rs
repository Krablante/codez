use super::prune_prompt_history_for_sampling;
use super::prune_prompt_history_preserving_current_turn_for_sampling;
use super::prune_prompt_history_preserving_current_turn_for_sampling_with_boundary;
use codex_protocol::AgentPath;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ImageDetail;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::LocalShellExecAction;
use codex_protocol::models::LocalShellStatus;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::InterAgentCommunication;
use pretty_assertions::assert_eq;

fn input_text(text: &str) -> ContentItem {
    ContentItem::InputText {
        text: text.to_string(),
    }
}

fn output_text(text: &str) -> ContentItem {
    ContentItem::OutputText {
        text: text.to_string(),
    }
}

fn user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![input_text(text)],
        phase: None,
    }
}

fn with_message_id(mut item: ResponseItem, message_id: &str) -> ResponseItem {
    if let ResponseItem::Message { id, .. } = &mut item {
        *id = Some(message_id.to_string());
    }
    item
}

fn assistant_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![output_text(text)],
        phase: Some(MessagePhase::Commentary),
    }
}

fn developer_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![input_text(text)],
        phase: None,
    }
}

fn developer_message_sections(texts: &[&str]) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: texts.iter().map(|text| input_text(text)).collect(),
        phase: None,
    }
}

fn inter_agent_assistant_message(text: &str) -> ResponseItem {
    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::root().join("worker").unwrap(),
        Vec::new(),
        text.to_string(),
        true,
    );
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![output_text(&serde_json::to_string(&communication).unwrap())],
        phase: None,
    }
}

fn shell_function_call(call_id: &str, command: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: "shell_command".to_string(),
        namespace: None,
        arguments: format!("{{\"command\":\"{command}\"}}"),
        call_id: call_id.to_string(),
    }
}

fn function_call_output(call_id: &str, output: &str) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        call_id: call_id.to_string(),
        output: FunctionCallOutputPayload::from_text(output.to_string()),
    }
}

fn reasoning_item(id: &str, text: &str) -> ResponseItem {
    ResponseItem::Reasoning {
        id: id.to_string(),
        summary: vec![ReasoningItemReasoningSummary::SummaryText {
            text: text.to_string(),
        }],
        content: None,
        encrypted_content: None,
    }
}

#[test]
fn prune_prompt_history_drops_stale_context_messages_but_keeps_current_context_window() {
    let mixed_developer_message = developer_message_sections(&[
        "<permissions instructions>\nstale permissions",
        "persistent developer policy",
    ]);
    let first_prompt = user_message("first prompt");
    let first_answer = assistant_message("first answer");
    let current_developer_context = developer_message("<permissions instructions>\ncurrent");
    let current_user_context =
        user_message("<environment_context>\ncurrent\n</environment_context>");
    let second_prompt = user_message("second prompt");

    let pruned = prune_prompt_history_for_sampling(vec![
        developer_message("<permissions instructions>\nstale permissions"),
        user_message("<environment_context>\nstale env\n</environment_context>"),
        mixed_developer_message.clone(),
        first_prompt.clone(),
        first_answer.clone(),
        current_developer_context.clone(),
        current_user_context.clone(),
        second_prompt.clone(),
    ]);

    assert_eq!(
        pruned,
        vec![
            mixed_developer_message,
            first_prompt,
            first_answer,
            current_developer_context,
            current_user_context,
            second_prompt,
        ]
    );
}

#[test]
fn prune_prompt_history_keeps_latest_turn_window_and_drops_older_ephemeral_items() {
    let pruned = prune_prompt_history_for_sampling(vec![
        user_message("first prompt"),
        ResponseItem::FunctionCall {
            id: None,
            name: "shell_command".to_string(),
            namespace: None,
            arguments: "{\"command\":\"printf old\"}".to_string(),
            call_id: "call-1".to_string(),
        },
        ResponseItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: FunctionCallOutputPayload::from_text("old tool output".to_string()),
        },
        ResponseItem::Reasoning {
            id: "rs_1".to_string(),
            summary: vec![ReasoningItemReasoningSummary::SummaryText {
                text: "old reasoning".to_string(),
            }],
            content: None,
            encrypted_content: None,
        },
        ResponseItem::ImageGenerationCall {
            id: "img_1".to_string(),
            status: "completed".to_string(),
            revised_prompt: Some("old image".to_string()),
            result: "base64".to_string(),
        },
        ResponseItem::Compaction {
            encrypted_content: "keep summary".to_string(),
        },
        assistant_message("first answer"),
        developer_message("<permissions instructions>\ncurrent permissions"),
        user_message("<environment_context>\ncurrent env\n</environment_context>"),
        user_message("second prompt"),
        ResponseItem::CustomToolCallOutput {
            call_id: "call-2".to_string(),
            name: Some("custom_tool".to_string()),
            output: FunctionCallOutputPayload::from_text("current output".to_string()),
        },
    ]);

    assert_eq!(pruned.len(), 7);
    assert!(matches!(
        &pruned[0],
        ResponseItem::Message { role, .. } if role == "user"
    ));
    assert!(matches!(
        &pruned[1],
        ResponseItem::Compaction { encrypted_content } if encrypted_content == "keep summary"
    ));
    assert!(matches!(
        &pruned[2],
        ResponseItem::Message { role, .. } if role == "assistant"
    ));
    assert!(matches!(
        &pruned[3],
        ResponseItem::Message { role, .. } if role == "developer"
    ));
    assert!(matches!(
        &pruned[4],
        ResponseItem::Message { role, content, .. }
            if role == "user"
                && content.iter().any(|item| matches!(
                    item,
                    ContentItem::InputText { text }
                        if text.contains("<environment_context>")
                ))
    ));
    assert!(matches!(
        &pruned[5],
        ResponseItem::Message { role, content, .. }
            if role == "user"
                && content.iter().any(|item| matches!(
                    item,
                    ContentItem::InputText { text } if text == "second prompt"
                ))
    ));
    assert!(matches!(
        &pruned[6],
        ResponseItem::CustomToolCallOutput { call_id, .. } if call_id == "call-2"
    ));
}

#[test]
fn prune_prompt_history_keeps_goal_continuation_prompt_and_drops_stale_tools() {
    let goal_continuation_prompt = developer_message(
        "Continue working toward the active thread goal.\n\n<untrusted_objective>\nkeep going\n</untrusted_objective>",
    );

    let pruned = prune_prompt_history_for_sampling(vec![
        user_message("first prompt"),
        ResponseItem::FunctionCall {
            id: None,
            name: "shell_command".to_string(),
            namespace: None,
            arguments: "{\"command\":\"printf old\"}".to_string(),
            call_id: "old-call".to_string(),
        },
        ResponseItem::FunctionCallOutput {
            call_id: "old-call".to_string(),
            output: FunctionCallOutputPayload::from_text("old tool output".to_string()),
        },
        assistant_message("first answer"),
        goal_continuation_prompt.clone(),
    ]);

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(pruned.contains(&user_message("first prompt")));
    assert!(pruned.contains(&goal_continuation_prompt));
}

#[test]
fn prune_prompt_history_leaves_items_without_turn_boundary_unchanged() {
    let items = vec![
        developer_message("<permissions instructions>\npermissions"),
        ResponseItem::FunctionCall {
            id: None,
            name: "shell_command".to_string(),
            namespace: None,
            arguments: "{\"command\":\"printf old\"}".to_string(),
            call_id: "call-1".to_string(),
        },
        ResponseItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: FunctionCallOutputPayload::from_text("old tool output".to_string()),
        },
    ];

    assert_eq!(prune_prompt_history_for_sampling(items.clone()), items);
}

#[test]
fn prune_prompt_history_uses_inter_agent_boundary_and_drops_other_prunable_variants() {
    let pruned = prune_prompt_history_for_sampling(vec![
        user_message("first prompt"),
        ResponseItem::LocalShellCall {
            id: None,
            call_id: Some("shell-1".to_string()),
            status: LocalShellStatus::Completed,
            action: LocalShellAction::Exec(LocalShellExecAction {
                command: vec!["echo".to_string(), "old".to_string()],
                timeout_ms: None,
                working_directory: None,
                env: None,
                user: None,
            }),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: Some("completed".to_string()),
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "calendar"}),
        },
        ResponseItem::ToolSearchOutput {
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: Vec::new(),
        },
        ResponseItem::WebSearchCall {
            id: None,
            status: Some("completed".to_string()),
            action: None,
        },
        inter_agent_assistant_message("delegate this"),
        ResponseItem::LocalShellCall {
            id: None,
            call_id: Some("shell-2".to_string()),
            status: LocalShellStatus::Completed,
            action: LocalShellAction::Exec(LocalShellExecAction {
                command: vec!["echo".to_string(), "current".to_string()],
                timeout_ms: None,
                working_directory: None,
                env: None,
                user: None,
            }),
        },
    ]);

    assert_eq!(pruned.len(), 3);
    assert!(matches!(
        &pruned[0],
        ResponseItem::Message { role, content, .. }
            if role == "user"
                && content.iter().any(|item| matches!(
                    item,
                    ContentItem::InputText { text } if text == "first prompt"
                ))
    ));
    assert!(matches!(
        &pruned[1],
        ResponseItem::Message { role, .. } if role == "assistant"
    ));
    assert!(matches!(
        &pruned[2],
        ResponseItem::LocalShellCall { call_id, .. }
            if call_id.as_deref() == Some("shell-2")
    ));
}

#[test]
fn prune_prompt_history_keeps_image_bearing_tool_outputs_from_older_turns() {
    let image_url = "data:image/webp;base64,AAAA";
    let pruned = prune_prompt_history_for_sampling(vec![
        user_message("first prompt"),
        ResponseItem::FunctionCall {
            id: None,
            name: "view_image".to_string(),
            namespace: None,
            arguments: "{\"path\":\"/tmp/example.webp\"}".to_string(),
            call_id: "view-image-call".to_string(),
        },
        ResponseItem::FunctionCallOutput {
            call_id: "view-image-call".to_string(),
            output: FunctionCallOutputPayload::from_content_items(vec![
                FunctionCallOutputContentItem::InputImage {
                    image_url: image_url.to_string(),
                    detail: Some(ImageDetail::Original),
                },
            ]),
        },
        ResponseItem::CustomToolCall {
            id: None,
            status: Some("completed".to_string()),
            call_id: "js-repl-call".to_string(),
            name: "js_repl".to_string(),
            input: "console.log('image flow')".to_string(),
        },
        ResponseItem::CustomToolCallOutput {
            call_id: "js-repl-call".to_string(),
            name: Some("js_repl".to_string()),
            output: FunctionCallOutputPayload::from_content_items(vec![
                FunctionCallOutputContentItem::InputImage {
                    image_url: image_url.to_string(),
                    detail: Some(ImageDetail::Original),
                },
            ]),
        },
        user_message("second prompt"),
    ]);

    assert_eq!(pruned.len(), 6);
    assert!(matches!(
        &pruned[1],
        ResponseItem::FunctionCall { call_id, .. } if call_id == "view-image-call"
    ));
    assert!(matches!(
        &pruned[2],
        ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "view-image-call"
    ));
    assert!(matches!(
        &pruned[3],
        ResponseItem::CustomToolCall { call_id, .. } if call_id == "js-repl-call"
    ));
    assert!(matches!(
        &pruned[4],
        ResponseItem::CustomToolCallOutput { call_id, .. } if call_id == "js-repl-call"
    ));
}

#[test]
fn prune_prompt_history_preserving_current_turn_drops_stale_history_before_live_steer() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let stale_env = user_message("<environment_context>\nstale\n</environment_context>");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_context = developer_message("<permissions instructions>\ncurrent");
    let active_env = user_message("<environment_context>\ncurrent\n</environment_context>");
    let active_prompt = user_message("current prompt");
    let active_call = shell_function_call("current-call", "printf current");
    let active_output = function_call_output("current-call", "current tool output");
    let active_reasoning = reasoning_item("current-reasoning", "current reasoning");
    let live_steer = user_message("also center the title");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling(vec![
        stale_context,
        stale_env,
        first_prompt.clone(),
        first_call,
        first_output,
        first_reasoning,
        assistant_message("first answer"),
        active_context.clone(),
        active_env.clone(),
        active_prompt.clone(),
        active_call.clone(),
        active_output.clone(),
        active_reasoning.clone(),
        live_steer.clone(),
    ]);

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));
    assert!(!pruned.contains(&user_message(
        "<environment_context>\nstale\n</environment_context>"
    )));

    assert!(pruned.contains(&first_prompt));
    assert!(pruned.contains(&active_context));
    assert!(pruned.contains(&active_env));
    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&active_call));
    assert!(pruned.contains(&active_output));
    assert!(pruned.contains(&active_reasoning));
    assert!(pruned.contains(&live_steer));
}

#[test]
fn prune_prompt_history_preserving_current_turn_keeps_active_chain_with_steer_and_subagent() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let stale_env = user_message("<environment_context>\nstale\n</environment_context>");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_context = developer_message("<permissions instructions>\ncurrent");
    let active_env = user_message("<environment_context>\ncurrent\n</environment_context>");
    let active_prompt = user_message("current prompt");
    let active_call = shell_function_call("current-call", "printf current");
    let active_output = function_call_output("current-call", "current tool output");
    let active_reasoning = reasoning_item("current-reasoning", "current reasoning");
    let live_steer = user_message("also center the title");
    let subagent_mail = inter_agent_assistant_message("queued child update");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling(vec![
        stale_context,
        stale_env,
        first_prompt.clone(),
        first_call,
        first_output,
        first_reasoning,
        assistant_message("first answer"),
        active_context.clone(),
        active_env.clone(),
        active_prompt.clone(),
        active_call.clone(),
        active_output.clone(),
        active_reasoning.clone(),
        live_steer.clone(),
        subagent_mail.clone(),
    ]);

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));
    assert!(!pruned.contains(&user_message(
        "<environment_context>\nstale\n</environment_context>"
    )));

    assert!(pruned.contains(&first_prompt));
    assert!(pruned.contains(&active_context));
    assert!(pruned.contains(&active_env));
    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&active_call));
    assert!(pruned.contains(&active_output));
    assert!(pruned.contains(&active_reasoning));
    assert!(pruned.contains(&live_steer));
    assert!(pruned.contains(&subagent_mail));
}

#[test]
fn prune_prompt_history_preserving_current_turn_keeps_chain_before_repeated_live_steer() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let stale_env = user_message("<environment_context>\nstale\n</environment_context>");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_context = developer_message("<permissions instructions>\ncurrent");
    let active_env = user_message("<environment_context>\ncurrent\n</environment_context>");
    let active_prompt = user_message("current prompt");
    let active_call = shell_function_call("current-call", "printf current");
    let active_output = function_call_output("current-call", "current tool output");
    let active_reasoning = reasoning_item("current-reasoning", "current reasoning");
    let first_live_steer = user_message("also center the title");
    let post_steer_call = shell_function_call("post-steer-call", "printf post-steer");
    let post_steer_output = function_call_output("post-steer-call", "post-steer tool output");
    let post_steer_reasoning = reasoning_item("post-steer-reasoning", "post-steer reasoning");
    let second_live_steer = user_message("make it tighter too");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling(vec![
        stale_context,
        stale_env,
        first_prompt.clone(),
        first_call,
        first_output,
        first_reasoning,
        assistant_message("first answer"),
        active_context.clone(),
        active_env.clone(),
        active_prompt.clone(),
        active_call.clone(),
        active_output.clone(),
        active_reasoning.clone(),
        first_live_steer.clone(),
        post_steer_call.clone(),
        post_steer_output.clone(),
        post_steer_reasoning.clone(),
        second_live_steer.clone(),
    ]);

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));
    assert!(!pruned.contains(&user_message(
        "<environment_context>\nstale\n</environment_context>"
    )));

    assert!(pruned.contains(&first_prompt));
    assert!(pruned.contains(&active_context));
    assert!(pruned.contains(&active_env));
    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&active_call));
    assert!(pruned.contains(&active_output));
    assert!(pruned.contains(&active_reasoning));
    assert!(pruned.contains(&first_live_steer));
    assert!(pruned.contains(&post_steer_call));
    assert!(pruned.contains(&post_steer_output));
    assert!(pruned.contains(&post_steer_reasoning));
    assert!(pruned.contains(&second_live_steer));
}

#[test]
fn prune_prompt_history_preserving_current_turn_uses_marker_for_repeated_text_collision() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_prompt = with_message_id(user_message("repeat this exact text"), "active-boundary");
    let active_call = shell_function_call("current-call", "printf current");
    let active_output = function_call_output("current-call", "current tool output");
    let active_reasoning = reasoning_item("current-reasoning", "current reasoning");
    let duplicate_text_live_steer = with_message_id(
        user_message("repeat this exact text"),
        "live-steer-boundary",
    );

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling_with_boundary(
        vec![
            stale_context,
            first_prompt,
            first_call,
            first_output,
            first_reasoning,
            assistant_message("first answer"),
            active_prompt.clone(),
            active_call.clone(),
            active_output.clone(),
            active_reasoning.clone(),
            duplicate_text_live_steer.clone(),
        ],
        active_prompt.clone(),
    );

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );

    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&active_call));
    assert!(pruned.contains(&active_output));
    assert!(pruned.contains(&active_reasoning));
    assert!(pruned.contains(&duplicate_text_live_steer));
}

#[test]
fn prune_prompt_history_preserving_current_turn_uses_adjacent_marked_boundary() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_prompt = with_message_id(user_message("current prompt"), "active-boundary");
    let live_steer = with_message_id(user_message("also center the title"), "live-steer-boundary");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling_with_boundary(
        vec![
            stale_context,
            first_prompt,
            first_call,
            first_output,
            first_reasoning,
            assistant_message("first answer"),
            active_prompt.clone(),
            live_steer.clone(),
        ],
        active_prompt.clone(),
    );

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));

    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&live_steer));
}

#[test]
fn prune_prompt_history_preserving_current_turn_keeps_chain_before_interleaved_context() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let stale_env = user_message("<environment_context>\nstale\n</environment_context>");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let active_context = developer_message("<permissions instructions>\ncurrent");
    let active_env = user_message("<environment_context>\ncurrent\n</environment_context>");
    let active_prompt = user_message("current prompt");
    let active_call = shell_function_call("current-call", "printf current");
    let active_output = function_call_output("current-call", "current tool output");
    let active_reasoning = reasoning_item("current-reasoning", "current reasoning");
    let live_steer = user_message("also center the title");
    let hook_context = developer_message("hook additional context");
    let subagent_mail = inter_agent_assistant_message("queued child update");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling(vec![
        stale_context,
        stale_env,
        first_prompt.clone(),
        first_call,
        first_output,
        first_reasoning,
        assistant_message("first answer"),
        active_context.clone(),
        active_env.clone(),
        active_prompt.clone(),
        active_call.clone(),
        active_output.clone(),
        active_reasoning.clone(),
        live_steer.clone(),
        hook_context.clone(),
        subagent_mail.clone(),
    ]);

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));
    assert!(!pruned.contains(&user_message(
        "<environment_context>\nstale\n</environment_context>"
    )));

    assert!(pruned.contains(&first_prompt));
    assert!(pruned.contains(&active_context));
    assert!(pruned.contains(&active_env));
    assert!(pruned.contains(&active_prompt));
    assert!(pruned.contains(&active_call));
    assert!(pruned.contains(&active_output));
    assert!(pruned.contains(&active_reasoning));
    assert!(pruned.contains(&live_steer));
    assert!(pruned.contains(&hook_context));
    assert!(pruned.contains(&subagent_mail));
}

#[test]
fn prune_prompt_history_preserving_current_turn_uses_synthetic_boundary_without_context() {
    let stale_context = developer_message("<permissions instructions>\nstale");
    let stale_env = user_message("<environment_context>\nstale\n</environment_context>");
    let first_prompt = user_message("first prompt");
    let first_call = shell_function_call("old-call", "printf old");
    let first_output = function_call_output("old-call", "old tool output");
    let first_reasoning = reasoning_item("old-reasoning", "old reasoning");
    let goal_prompt = with_message_id(
        developer_message(
            "Continue working toward the active thread goal.\n\n<untrusted_objective>\nwrite a benchmark note\n</untrusted_objective>",
        ),
        "goal-boundary",
    );
    let goal_call = shell_function_call("goal-call", "printf goal");
    let goal_output = function_call_output("goal-call", "goal tool output");
    let goal_reasoning = reasoning_item("goal-reasoning", "goal reasoning");
    let live_steer = user_message("also include the graph");

    let pruned = prune_prompt_history_preserving_current_turn_for_sampling_with_boundary(
        vec![
            stale_context,
            stale_env,
            first_prompt.clone(),
            first_call,
            first_output,
            first_reasoning,
            assistant_message("first answer"),
            goal_prompt.clone(),
            goal_call.clone(),
            goal_output.clone(),
            goal_reasoning.clone(),
            live_steer.clone(),
        ],
        goal_prompt.clone(),
    );

    assert!(!pruned.iter().any(
        |item| matches!(item, ResponseItem::FunctionCall { call_id, .. } if call_id == "old-call")
    ));
    assert!(
        !pruned
            .iter()
            .any(|item| matches!(item, ResponseItem::FunctionCallOutput { call_id, .. } if call_id == "old-call"))
    );
    assert!(
        !pruned.iter().any(
            |item| matches!(item, ResponseItem::Reasoning { id, .. } if id == "old-reasoning")
        )
    );
    assert!(!pruned.contains(&developer_message("<permissions instructions>\nstale")));
    assert!(!pruned.contains(&user_message(
        "<environment_context>\nstale\n</environment_context>"
    )));

    assert!(pruned.contains(&first_prompt));
    assert!(pruned.contains(&goal_prompt));
    assert!(pruned.contains(&goal_call));
    assert!(pruned.contains(&goal_output));
    assert!(pruned.contains(&goal_reasoning));
    assert!(pruned.contains(&live_steer));
}
