use super::*;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[test]
fn failed_turn_does_not_overwrite_output_last_message_file() {
    let tempdir = tempdir().expect("create tempdir");
    let output_path = tempdir.path().join("last-message.txt");
    std::fs::write(&output_path, "keep existing contents").expect("seed output file");

    let mut processor = EventProcessorWithJsonOutput::new(Some(output_path.clone()));

    let collected = processor.collect_thread_events(ServerNotification::ItemCompleted(
        codex_app_server_protocol::ItemCompletedNotification {
            item: ThreadItem::AgentMessage {
                id: "msg-1".to_string(),
                text: "partial answer".to_string(),
                phase: None,
                memory_citation: None,
            },
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            completed_at_ms: 0,
        },
    ));

    assert_eq!(collected.status, CodexStatus::Running);
    assert_eq!(processor.final_message(), Some("partial answer"));

    let status = processor.process_server_notification(ServerNotification::TurnCompleted(
        codex_app_server_protocol::TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: codex_app_server_protocol::Turn {
                id: "turn-1".to_string(),
                items_view: codex_app_server_protocol::TurnItemsView::Full,
                items: Vec::new(),
                status: TurnStatus::Failed,
                error: Some(codex_app_server_protocol::TurnError {
                    message: "turn failed".to_string(),
                    additional_details: None,
                    codex_error_info: None,
                }),
                started_at: None,
                completed_at: Some(0),
                duration_ms: None,
            },
        },
    ));

    assert_eq!(status, CodexStatus::InitiateShutdown);
    assert_eq!(processor.final_message(), None);

    EventProcessor::print_final_output(&mut processor);

    assert_eq!(
        std::fs::read_to_string(&output_path).expect("read output file"),
        "keep existing contents"
    );
}

#[test]
fn turn_completed_includes_active_usage_separate_from_total_usage() {
    let mut processor = EventProcessorWithJsonOutput::new(None);

    let collected = processor.collect_thread_events(ServerNotification::ThreadTokenUsageUpdated(
        codex_app_server_protocol::ThreadTokenUsageUpdatedNotification {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            token_usage: codex_app_server_protocol::ThreadTokenUsage {
                total: codex_app_server_protocol::TokenUsageBreakdown {
                    total_tokens: 115_787,
                    input_tokens: 114_902,
                    cached_input_tokens: 74_240,
                    output_tokens: 885,
                    reasoning_output_tokens: 230,
                },
                last: codex_app_server_protocol::TokenUsageBreakdown {
                    total_tokens: 22_934,
                    input_tokens: 22_517,
                    cached_input_tokens: 21_376,
                    output_tokens: 417,
                    reasoning_output_tokens: 21,
                },
                model_context_window: Some(1_000_000),
            },
        },
    ));
    assert_eq!(collected.status, CodexStatus::Running);

    let collected = processor.collect_thread_events(ServerNotification::TurnCompleted(
        codex_app_server_protocol::TurnCompletedNotification {
            thread_id: "thread-1".to_string(),
            turn: codex_app_server_protocol::Turn {
                id: "turn-1".to_string(),
                items: Vec::new(),
                items_view: codex_app_server_protocol::TurnItemsView::Full,
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: Some(0),
                duration_ms: None,
            },
        },
    ));

    assert_eq!(collected.status, CodexStatus::InitiateShutdown);
    assert_eq!(
        collected.events,
        vec![ThreadEvent::TurnCompleted(TurnCompletedEvent {
            usage: Usage {
                input_tokens: 114_902,
                cached_input_tokens: 74_240,
                output_tokens: 885,
                reasoning_output_tokens: 230,
            },
            active_usage: Some(Usage {
                input_tokens: 22_517,
                cached_input_tokens: 21_376,
                output_tokens: 417,
                reasoning_output_tokens: 21,
            }),
        })]
    );
}
