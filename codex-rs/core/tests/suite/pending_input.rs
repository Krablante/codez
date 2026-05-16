use std::sync::Arc;

use codex_core::CodexThread;
use codex_protocol::AgentPath;
use codex_protocol::items::TurnItem;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::context_snapshot;
use core_test_support::context_snapshot::ContextSnapshotOptions;
use core_test_support::responses;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_message_item_added;
use core_test_support::responses::ev_output_text_delta;
use core_test_support::responses::ev_reasoning_item;
use core_test_support::responses::ev_reasoning_item_added;
use core_test_support::responses::ev_response_created;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::StreamingSseServer;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::from_slice;
use serde_json::json;
use tokio::sync::oneshot;

fn ev_message_item_done(id: &str, text: &str) -> Value {
    serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "id": id,
            "content": [{"type": "output_text", "text": text}]
        }
    })
}

fn sse_event(event: Value) -> String {
    responses::sse(vec![event])
}

fn message_input_texts(body: &Value, role: &str) -> Vec<String> {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter(|item| item.get("role").and_then(Value::as_str) == Some(role))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|span| span.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|span| span.get("text").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn message_output_texts(body: &Value, role: &str) -> Vec<String> {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter(|item| item.get("role").and_then(Value::as_str) == Some(role))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|span| span.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|span| span.get("text").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn input_has_reasoning_summary(body: &Value, summary_text: &str) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| {
            item.get("type").and_then(Value::as_str) == Some("reasoning")
                && item
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|summary| {
                        summary.get("text").and_then(Value::as_str) == Some(summary_text)
                    })
        })
}

fn input_has_function_call(body: &Value, call_id: &str) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("call_id").and_then(Value::as_str) == Some(call_id)
        })
}

fn input_has_function_call_output(body: &Value, call_id: &str) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some(call_id)
        })
}

fn chunk(event: Value) -> StreamingSseChunk {
    StreamingSseChunk {
        gate: None,
        body: responses::sse(vec![event]),
    }
}

fn gated_chunk(gate: oneshot::Receiver<()>, events: Vec<Value>) -> StreamingSseChunk {
    StreamingSseChunk {
        gate: Some(gate),
        body: responses::sse(events),
    }
}

fn response_completed_chunks(response_id: &str) -> Vec<StreamingSseChunk> {
    vec![
        chunk(ev_response_created(response_id)),
        chunk(ev_completed(response_id)),
    ]
}

async fn build_codex(server: &StreamingSseServer) -> Arc<CodexThread> {
    test_codex()
        .with_model("gpt-5.4")
        .build_with_streaming_server(server)
        .await
        .unwrap_or_else(|err| panic!("build streaming Codex test session: {err}"))
        .codex
}

async fn submit_user_input(codex: &CodexThread, text: &str) {
    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .unwrap_or_else(|err| panic!("submit user input: {err}"));
}

async fn submit_danger_full_access_user_turn(test: &TestCodex, text: &str) {
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());
    test.codex
        .submit(Op::UserTurn {
            environments: None,
            items: vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            cwd: test.config.cwd.to_path_buf(),
            approval_policy: AskForApproval::Never,
            approvals_reviewer: None,
            sandbox_policy,
            permission_profile,
            model: test.session_configured.model.clone(),
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        })
        .await
        .unwrap_or_else(|err| panic!("submit user turn: {err}"));
}

async fn steer_user_input(codex: &CodexThread, text: &str) {
    codex
        .steer_input(
            vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            /*expected_turn_id*/ None,
            /*responsesapi_client_metadata*/ None,
        )
        .await
        .unwrap_or_else(|err| panic!("steer user input: {err:?}"));
}

async fn submit_queue_only_agent_mail(codex: &CodexThread, text: &str) {
    codex
        .submit(Op::InterAgentCommunication {
            communication: InterAgentCommunication::new(
                AgentPath::try_from("/root/worker")
                    .unwrap_or_else(|err| panic!("worker path should parse: {err}")),
                AgentPath::root(),
                Vec::new(),
                text.to_string(),
                /*trigger_turn*/ false,
            ),
        })
        .await
        .unwrap_or_else(|err| panic!("submit queue-only agent mail: {err}"));
    codex
        .submit(Op::RealtimeConversationListVoices)
        .await
        .unwrap_or_else(|err| panic!("submit list-voices barrier: {err}"));
    wait_for_event(codex, |event| {
        matches!(event, EventMsg::RealtimeConversationListVoicesResponse(_))
    })
    .await;
}

async fn submit_trigger_turn_agent_mail(codex: &CodexThread, text: &str) {
    codex
        .submit(Op::InterAgentCommunication {
            communication: InterAgentCommunication::new(
                AgentPath::try_from("/root/worker")
                    .unwrap_or_else(|err| panic!("worker path should parse: {err}")),
                AgentPath::root(),
                Vec::new(),
                text.to_string(),
                /*trigger_turn*/ true,
            ),
        })
        .await
        .unwrap_or_else(|err| panic!("submit trigger-turn agent mail: {err}"));
}

async fn wait_for_reasoning_item_started(codex: &CodexThread) {
    wait_for_event(codex, |event| {
        matches!(
            event,
            EventMsg::ItemStarted(item_started)
                if matches!(&item_started.item, TurnItem::Reasoning(_))
        )
    })
    .await;
}

async fn wait_for_agent_message(codex: &CodexThread, text: &str) {
    let final_message = wait_for_event(
        codex,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == text),
    )
    .await;
    assert!(matches!(final_message, EventMsg::AgentMessage(_)));
}

async fn wait_for_turn_complete(codex: &CodexThread) {
    wait_for_event(codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
}

fn assert_two_responses_input_snapshot(snapshot_name: &str, requests: &[Vec<u8>]) {
    assert_eq!(requests.len(), 2);
    let options = ContextSnapshotOptions::default().strip_capability_instructions();
    let first: Value =
        from_slice(&requests[0]).unwrap_or_else(|err| panic!("parse first request: {err}"));
    let second: Value =
        from_slice(&requests[1]).unwrap_or_else(|err| panic!("parse second request: {err}"));
    let first_items = first["input"]
        .as_array()
        .unwrap_or_else(|| panic!("first request input"))
        .clone();
    let second_items = second["input"]
        .as_array()
        .unwrap_or_else(|| panic!("second request input"))
        .clone();
    let snapshot = context_snapshot::format_labeled_items_snapshot(
        "/responses POST bodies (input only, redacted like other suite snapshots)",
        &[
            ("First request", first_items.as_slice()),
            ("Second request", second_items.as_slice()),
        ],
        &options,
    );
    insta::assert_snapshot!(snapshot_name, snapshot);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "TODO(aibrahim): flaky"]
async fn injected_user_input_triggers_follow_up_request_with_deltas() {
    let (gate_completed_tx, gate_completed_rx) = oneshot::channel();

    let first_chunks = vec![
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_response_created("resp-1")),
        },
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_message_item_added("msg-1", "")),
        },
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_output_text_delta("first ")),
        },
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_output_text_delta("turn")),
        },
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_message_item_done("msg-1", "first turn")),
        },
        StreamingSseChunk {
            gate: Some(gate_completed_rx),
            body: sse_event(ev_completed("resp-1")),
        },
    ];

    let second_chunks = vec![
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_response_created("resp-2")),
        },
        StreamingSseChunk {
            gate: None,
            body: sse_event(ev_completed("resp-2")),
        },
    ];

    let (server, _completions) =
        start_streaming_sse_server(vec![first_chunks, second_chunks]).await;

    let codex = test_codex()
        .with_model("gpt-5.4")
        .build_with_streaming_server(&server)
        .await
        .unwrap()
        .codex;

    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "first prompt".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .unwrap();

    wait_for_event(&codex, |event| {
        matches!(event, EventMsg::AgentMessageContentDelta(_))
    })
    .await;

    codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "second prompt".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await
        .unwrap();

    let _ = gate_completed_tx.send(());

    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);

    let first_body: Value = serde_json::from_slice(&requests[0]).expect("parse first request");
    let second_body: Value = serde_json::from_slice(&requests[1]).expect("parse second request");

    let first_texts = message_input_texts(&first_body, "user");
    assert!(first_texts.iter().any(|text| text == "first prompt"));
    assert!(!first_texts.iter().any(|text| text == "second prompt"));

    let second_texts = message_input_texts(&second_body, "user");
    assert!(second_texts.iter().any(|text| text == "first prompt"));
    assert!(second_texts.iter().any(|text| text == "second prompt"));

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_inter_agent_mail_triggers_follow_up_after_reasoning_item() {
    let (gate_reasoning_done_tx, gate_reasoning_done_rx) = oneshot::channel();

    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_reasoning_item_added("reason-1", &["thinking"])),
        gated_chunk(
            gate_reasoning_done_rx,
            vec![
                ev_reasoning_item("reason-1", &["thinking"], &[]),
                ev_function_call(
                    "call-stale",
                    "shell",
                    r#"{"command":"echo stale tool call"}"#,
                ),
                ev_message_item_added("msg-stale", ""),
                ev_output_text_delta("stale final"),
                ev_message_item_done("msg-stale", "stale final"),
                ev_completed("resp-1"),
            ],
        ),
    ];

    let (server, _completions) =
        start_streaming_sse_server(vec![first_chunks, response_completed_chunks("resp-2")]).await;

    let codex = build_codex(&server).await;

    submit_user_input(&codex, "first prompt").await;

    wait_for_reasoning_item_started(&codex).await;

    submit_queue_only_agent_mail(&codex, "queued child update").await;

    let _ = gate_reasoning_done_tx.send(());

    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_two_responses_input_snapshot("pending_input_queued_mail_after_reasoning", &requests);

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_inter_agent_mail_triggers_follow_up_after_commentary_message_item() {
    let (gate_message_done_tx, gate_message_done_rx) = oneshot::channel();

    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_message_item_added("msg-1", "")),
        gated_chunk(
            gate_message_done_rx,
            vec![
                ev_output_text_delta("first answer"),
                json!({
                    "type": "response.output_item.done",
                    "item": {
                        "type": "message",
                        "role": "assistant",
                        "id": "msg-1",
                        "content": [{"type": "output_text", "text": "first answer"}],
                        "phase": "commentary",
                    }
                }),
                ev_function_call(
                    "call-stale",
                    "shell",
                    r#"{"command":"echo stale tool call"}"#,
                ),
                ev_message_item_added("msg-stale", ""),
                ev_output_text_delta("stale final"),
                ev_message_item_done("msg-stale", "stale final"),
                ev_completed("resp-1"),
            ],
        ),
    ];

    let (server, _completions) =
        start_streaming_sse_server(vec![first_chunks, response_completed_chunks("resp-2")]).await;

    let codex = build_codex(&server).await;

    submit_user_input(&codex, "first prompt").await;

    wait_for_event(&codex, |event| {
        matches!(
            event,
            EventMsg::ItemStarted(item_started)
                if matches!(&item_started.item, TurnItem::AgentMessage(_))
        )
    })
    .await;

    submit_queue_only_agent_mail(&codex, "queued child update").await;

    let _ = gate_message_done_tx.send(());

    wait_for_agent_message(&codex, "first answer").await;

    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_two_responses_input_snapshot("pending_input_queued_mail_after_commentary", &requests);

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_input_does_not_preempt_after_reasoning_item() {
    let (gate_reasoning_done_tx, gate_reasoning_done_rx) = oneshot::channel();

    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_reasoning_item_added("reason-1", &["thinking"])),
        gated_chunk(
            gate_reasoning_done_rx,
            vec![
                ev_reasoning_item("reason-1", &["thinking"], &[]),
                ev_function_call(
                    "call-preserved",
                    "shell",
                    r#"{"command":"echo preserved tool call"}"#,
                ),
                ev_message_item_added("msg-1", ""),
                ev_output_text_delta("first answer"),
                ev_message_item_done("msg-1", "first answer"),
                ev_completed("resp-1"),
            ],
        ),
    ];

    let (server, _completions) =
        start_streaming_sse_server(vec![first_chunks, response_completed_chunks("resp-2")]).await;

    let codex = build_codex(&server).await;

    submit_user_input(&codex, "first prompt").await;

    wait_for_reasoning_item_started(&codex).await;

    steer_user_input(&codex, "second prompt").await;

    let _ = gate_reasoning_done_tx.send(());

    wait_for_agent_message(&codex, "first answer").await;

    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_two_responses_input_snapshot(
        "pending_input_user_input_no_preempt_after_reasoning",
        &requests,
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steered_user_input_prunes_stale_previous_turn_items_but_keeps_active_chain() {
    let (gate_current_done_tx, gate_current_done_rx) = oneshot::channel();

    let stale_chunks = vec![
        chunk(ev_response_created("resp-stale")),
        chunk(ev_reasoning_item("old-reasoning", &["old thinking"], &[])),
        chunk(ev_function_call(
            "old-call",
            "shell",
            r#"{"command":"echo stale tool call"}"#,
        )),
        chunk(ev_completed("resp-stale")),
    ];

    let stale_followup_chunks = vec![
        chunk(ev_response_created("resp-stale-followup")),
        chunk(ev_message_item_added("msg-stale", "")),
        chunk(ev_output_text_delta("stale answer")),
        chunk(ev_message_item_done("msg-stale", "stale answer")),
        chunk(ev_completed("resp-stale-followup")),
    ];

    let active_chunks = vec![
        chunk(ev_response_created("resp-current")),
        chunk(ev_reasoning_item_added(
            "current-reasoning",
            &["current thinking"],
        )),
        gated_chunk(
            gate_current_done_rx,
            vec![
                ev_reasoning_item("current-reasoning", &["current thinking"], &[]),
                ev_function_call(
                    "current-call",
                    "shell",
                    r#"{"command":"echo current tool call"}"#,
                ),
                ev_message_item_added("msg-current", ""),
                ev_output_text_delta("current answer"),
                ev_message_item_done("msg-current", "current answer"),
                ev_completed("resp-current"),
            ],
        ),
    ];

    let (server, _completions) = start_streaming_sse_server(vec![
        stale_chunks,
        stale_followup_chunks,
        active_chunks,
        response_completed_chunks("resp-steered"),
    ])
    .await;

    let codex = build_codex(&server).await;

    submit_user_input(&codex, "stale prompt").await;
    wait_for_agent_message(&codex, "stale answer").await;
    wait_for_turn_complete(&codex).await;

    submit_user_input(&codex, "current prompt").await;
    wait_for_reasoning_item_started(&codex).await;
    steer_user_input(&codex, "tiny live steer").await;
    let _ = gate_current_done_tx.send(());

    wait_for_agent_message(&codex, "current answer").await;
    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);

    let steered_body: Value =
        from_slice(&requests[3]).unwrap_or_else(|err| panic!("parse steered request: {err}"));

    let steered_user_texts = message_input_texts(&steered_body, "user");
    assert!(
        steered_user_texts
            .iter()
            .any(|text| text == "current prompt"),
        "active turn prompt should remain visible"
    );
    assert!(
        steered_user_texts
            .iter()
            .any(|text| text == "tiny live steer"),
        "live steer should remain visible"
    );

    assert!(
        !input_has_reasoning_summary(&steered_body, "old thinking"),
        "stale previous-turn reasoning should be pruned"
    );
    assert!(
        !input_has_function_call(&steered_body, "old-call"),
        "stale previous-turn tool call should be pruned"
    );
    assert!(
        !input_has_function_call_output(&steered_body, "old-call"),
        "stale previous-turn tool output should be pruned"
    );
    assert!(
        input_has_reasoning_summary(&steered_body, "current thinking"),
        "current-turn reasoning should be preserved"
    );
    assert!(
        input_has_function_call(&steered_body, "current-call"),
        "current-turn tool call should be preserved"
    );
    assert!(
        input_has_function_call_output(&steered_body, "current-call"),
        "current-turn tool output should be preserved"
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trigger_turn_agent_mail_prunes_stale_history_on_synthetic_turn() {
    let stale_chunks = vec![
        chunk(ev_response_created("resp-stale")),
        chunk(ev_reasoning_item("old-reasoning", &["old thinking"], &[])),
        chunk(ev_function_call(
            "old-call",
            "shell",
            r#"{"command":"echo stale tool call"}"#,
        )),
        chunk(ev_completed("resp-stale")),
    ];

    let stale_followup_chunks = vec![
        chunk(ev_response_created("resp-stale-followup")),
        chunk(ev_message_item_added("msg-stale", "")),
        chunk(ev_output_text_delta("stale answer")),
        chunk(ev_message_item_done("msg-stale", "stale answer")),
        chunk(ev_completed("resp-stale-followup")),
    ];

    let mail_chunks = vec![
        chunk(ev_response_created("resp-mail")),
        chunk(ev_reasoning_item("mail-reasoning", &["mail thinking"], &[])),
        chunk(ev_function_call(
            "mail-call",
            "shell",
            r#"{"command":"echo mail tool call"}"#,
        )),
        chunk(ev_completed("resp-mail")),
    ];

    let mail_followup_chunks = vec![
        chunk(ev_response_created("resp-mail-followup")),
        chunk(ev_message_item_added("msg-mail", "")),
        chunk(ev_output_text_delta("mail answer")),
        chunk(ev_message_item_done("msg-mail", "mail answer")),
        chunk(ev_completed("resp-mail-followup")),
    ];

    let (server, _completions) = start_streaming_sse_server(vec![
        stale_chunks,
        stale_followup_chunks,
        mail_chunks,
        mail_followup_chunks,
    ])
    .await;

    let codex = build_codex(&server).await;

    submit_user_input(&codex, "stale prompt").await;
    wait_for_agent_message(&codex, "stale answer").await;
    wait_for_turn_complete(&codex).await;

    submit_trigger_turn_agent_mail(&codex, "queued child update").await;
    wait_for_agent_message(&codex, "mail answer").await;
    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);

    let mail_body: Value =
        from_slice(&requests[2]).unwrap_or_else(|err| panic!("parse mail request: {err}"));
    let mail_followup_body: Value = from_slice(&requests[3])
        .unwrap_or_else(|err| panic!("parse mail follow-up request: {err}"));

    for (body, label) in [
        (&mail_body, "mail request"),
        (&mail_followup_body, "mail follow-up request"),
    ] {
        assert!(
            !input_has_reasoning_summary(body, "old thinking"),
            "stale previous-turn reasoning should be pruned from {label}"
        );
        assert!(
            !input_has_function_call(body, "old-call"),
            "stale previous-turn tool call should be pruned from {label}"
        );
        assert!(
            !input_has_function_call_output(body, "old-call"),
            "stale previous-turn tool output should be pruned from {label}"
        );
        assert!(
            message_output_texts(body, "assistant")
                .iter()
                .any(|text| text.contains("queued child update")),
            "trigger-turn agent mail should remain visible in {label}"
        );
    }

    assert!(
        input_has_reasoning_summary(&mail_followup_body, "mail thinking"),
        "synthetic turn reasoning should remain visible in follow-up"
    );
    assert!(
        input_has_function_call(&mail_followup_body, "mail-call"),
        "synthetic turn tool call should remain visible in follow-up"
    );
    assert!(
        input_has_function_call_output(&mail_followup_body, "mail-call"),
        "synthetic turn tool output should remain visible in follow-up"
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steered_user_input_waits_for_model_continuation_after_mid_turn_compact() {
    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_function_call("call-1", "test_tool", "{}")),
        chunk(ev_completed_with_tokens(
            "resp-1", /*total_tokens*/ 500,
        )),
    ];

    let compact_chunks = vec![
        chunk(ev_response_created("resp-compact")),
        chunk(ev_message_item_done("msg-compact", "AUTO_COMPACT_SUMMARY")),
        chunk(ev_completed_with_tokens(
            "resp-compact",
            /*total_tokens*/ 50,
        )),
    ];

    let post_compact_continuation_chunks = vec![
        chunk(ev_response_created("resp-post-compact")),
        chunk(ev_message_item_added("msg-post-compact", "")),
        chunk(ev_output_text_delta("resumed old task")),
        chunk(ev_message_item_done("msg-post-compact", "resumed old task")),
        chunk(ev_completed_with_tokens(
            "resp-post-compact",
            /*total_tokens*/ 60,
        )),
    ];

    let steered_follow_up_chunks = vec![
        chunk(ev_response_created("resp-steered")),
        chunk(ev_message_item_done(
            "msg-steered",
            "processed steered prompt",
        )),
        chunk(ev_completed_with_tokens(
            "resp-steered",
            /*total_tokens*/ 70,
        )),
    ];

    let (server, _completions) = start_streaming_sse_server(vec![
        first_chunks,
        compact_chunks,
        post_compact_continuation_chunks,
        steered_follow_up_chunks,
    ])
    .await;

    let codex = test_codex()
        .with_model("gpt-5.4")
        .with_config(|config| {
            config.model_provider.name = "OpenAI (test)".to_string();
            config.model_provider.supports_websockets = false;
            config.model_auto_compact_token_limit = Some(200);
        })
        .build_with_streaming_server(&server)
        .await
        .unwrap_or_else(|err| panic!("build streaming Codex test session: {err}"))
        .codex;

    submit_user_input(&codex, "first prompt").await;
    submit_user_input(&codex, "second prompt").await;

    wait_for_agent_message(&codex, "resumed old task").await;
    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);

    let post_compact_body: Value =
        from_slice(&requests[2]).unwrap_or_else(|err| panic!("parse post-compact request: {err}"));
    let steered_body: Value =
        from_slice(&requests[3]).unwrap_or_else(|err| panic!("parse steered request: {err}"));

    let post_compact_user_texts = message_input_texts(&post_compact_body, "user");
    assert!(
        !post_compact_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should stay pending until the model resumes after compaction"
    );

    let steered_user_texts = message_input_texts(&steered_body, "user");
    assert!(
        steered_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should be recorded on the request after the post-compact continuation"
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steered_user_input_follows_compact_when_only_the_steer_needs_follow_up() {
    let (gate_first_completed_tx, gate_first_completed_rx) = oneshot::channel();

    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_message_item_added("msg-1", "")),
        chunk(ev_output_text_delta("first answer")),
        chunk(ev_message_item_done("msg-1", "first answer")),
        gated_chunk(
            gate_first_completed_rx,
            vec![ev_completed_with_tokens(
                "resp-1", /*total_tokens*/ 500,
            )],
        ),
    ];

    let compact_chunks = vec![
        chunk(ev_response_created("resp-compact")),
        chunk(ev_message_item_done("msg-compact", "AUTO_COMPACT_SUMMARY")),
        chunk(ev_completed_with_tokens(
            "resp-compact",
            /*total_tokens*/ 50,
        )),
    ];

    let steered_follow_up_chunks = vec![
        chunk(ev_response_created("resp-steered")),
        chunk(ev_message_item_done(
            "msg-steered",
            "processed steered prompt",
        )),
        chunk(ev_completed_with_tokens(
            "resp-steered",
            /*total_tokens*/ 70,
        )),
    ];

    let (server, _completions) =
        start_streaming_sse_server(vec![first_chunks, compact_chunks, steered_follow_up_chunks])
            .await;

    let codex = test_codex()
        .with_model("gpt-5.4")
        .with_config(|config| {
            config.model_provider.name = "OpenAI (test)".to_string();
            config.model_provider.supports_websockets = false;
            config.model_auto_compact_token_limit = Some(200);
        })
        .build_with_streaming_server(&server)
        .await
        .unwrap_or_else(|err| panic!("build streaming Codex test session: {err}"))
        .codex;

    submit_user_input(&codex, "first prompt").await;
    wait_for_agent_message(&codex, "first answer").await;
    steer_user_input(&codex, "second prompt").await;
    let _ = gate_first_completed_tx.send(());

    wait_for_agent_message(&codex, "processed steered prompt").await;
    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 3);

    let compact_body: Value =
        from_slice(&requests[1]).unwrap_or_else(|err| panic!("parse compact request: {err}"));
    let steered_body: Value =
        from_slice(&requests[2]).unwrap_or_else(|err| panic!("parse steered request: {err}"));

    let compact_user_texts = message_input_texts(&compact_body, "user");
    assert!(
        !compact_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should not be included in the compaction request"
    );

    let steered_user_texts = message_input_texts(&steered_body, "user");
    assert!(
        steered_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should follow compaction without an empty resume request when the model was already done"
    );

    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steered_user_input_waits_when_tool_output_triggers_compact_before_next_request() {
    let (gate_first_completed_tx, gate_first_completed_rx) = oneshot::channel();

    let large_output_command = if cfg!(windows) {
        "[Console]::Out.Write([string]::new([char]'0', 4000))"
    } else {
        "printf '%04000d' 0"
    };
    let large_output_args = json!({
        "command": large_output_command,
        "login": false,
        "timeout_ms": 2000,
    })
    .to_string();

    let first_chunks = vec![
        chunk(ev_response_created("resp-1")),
        chunk(ev_function_call(
            "call-1",
            "shell_command",
            &large_output_args,
        )),
        gated_chunk(
            gate_first_completed_rx,
            vec![ev_completed_with_tokens(
                "resp-1", /*total_tokens*/ 100,
            )],
        ),
    ];

    let compact_chunks = vec![
        chunk(ev_response_created("resp-compact")),
        chunk(ev_message_item_done("msg-compact", "TOOL_OUTPUT_SUMMARY")),
        chunk(ev_completed_with_tokens(
            "resp-compact",
            /*total_tokens*/ 50,
        )),
    ];

    let post_compact_continuation_chunks = vec![
        chunk(ev_response_created("resp-post-compact")),
        chunk(ev_message_item_done(
            "msg-post-compact",
            "resumed after compacting tool output",
        )),
        chunk(ev_completed_with_tokens(
            "resp-post-compact",
            /*total_tokens*/ 60,
        )),
    ];

    let steered_follow_up_chunks = vec![
        chunk(ev_response_created("resp-steered")),
        chunk(ev_message_item_done(
            "msg-steered",
            "processed steered prompt",
        )),
        chunk(ev_completed_with_tokens(
            "resp-steered",
            /*total_tokens*/ 70,
        )),
    ];

    let (server, _completions) = start_streaming_sse_server(vec![
        first_chunks,
        compact_chunks,
        post_compact_continuation_chunks,
        steered_follow_up_chunks,
    ])
    .await;

    let test = test_codex()
        .with_model("gpt-5.4")
        .with_config(|config| {
            config.model_provider.name = "OpenAI (test)".to_string();
            config.model_provider.supports_websockets = false;
            config.model_auto_compact_token_limit = Some(200);
        })
        .build_with_streaming_server(&server)
        .await
        .unwrap_or_else(|err| panic!("build streaming Codex test session: {err}"));
    let codex = test.codex.clone();

    submit_danger_full_access_user_turn(&test, "first prompt").await;
    wait_for_event(&codex, |event| matches!(event, EventMsg::TurnStarted(_))).await;
    steer_user_input(&codex, "second prompt").await;
    let _ = gate_first_completed_tx.send(());

    wait_for_turn_complete(&codex).await;

    let requests = server.requests().await;
    assert_eq!(requests.len(), 4);

    let compact_body: Value =
        from_slice(&requests[1]).unwrap_or_else(|err| panic!("parse compact request: {err}"));
    let post_compact_body: Value =
        from_slice(&requests[2]).unwrap_or_else(|err| panic!("parse post-compact request: {err}"));
    let steered_body: Value =
        from_slice(&requests[3]).unwrap_or_else(|err| panic!("parse steered request: {err}"));

    let compact_user_texts = message_input_texts(&compact_body, "user");
    assert!(
        !compact_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should not be included in the compaction request"
    );

    let post_compact_user_texts = message_input_texts(&post_compact_body, "user");
    assert!(
        !post_compact_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should stay pending until after the compacted continuation"
    );

    let steered_user_texts = message_input_texts(&steered_body, "user");
    assert!(
        steered_user_texts
            .iter()
            .any(|text| text == "second prompt"),
        "steered input should be recorded on the request after the post-compact continuation"
    );

    server.shutdown().await;
}
