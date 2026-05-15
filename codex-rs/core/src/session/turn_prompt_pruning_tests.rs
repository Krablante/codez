use super::prune_prompt_history_for_sampling;
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
