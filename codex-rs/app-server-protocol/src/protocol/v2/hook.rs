use super::shared::v2_enum_from_core;
use codex_protocol::protocol::HookEventName as CoreHookEventName;
use codex_protocol::protocol::HookExecutionMode as CoreHookExecutionMode;
use codex_protocol::protocol::HookHandlerType as CoreHookHandlerType;
use codex_protocol::protocol::HookOutputEntry as CoreHookOutputEntry;
use codex_protocol::protocol::HookOutputEntryKind as CoreHookOutputEntryKind;
use codex_protocol::protocol::HookRunEconomy as CoreHookRunEconomy;
use codex_protocol::protocol::HookRunStatus as CoreHookRunStatus;
use codex_protocol::protocol::HookRunSummary as CoreHookRunSummary;
use codex_protocol::protocol::HookScope as CoreHookScope;
use codex_protocol::protocol::HookSource as CoreHookSource;
use codex_protocol::protocol::HookTrustStatus as CoreHookTrustStatus;
use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

v2_enum_from_core!(
    pub enum HookEventName from CoreHookEventName {
        PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SessionStart, UserPromptSubmit, Stop
    }
);

v2_enum_from_core!(
    pub enum HookHandlerType from CoreHookHandlerType {
        Command, Prompt, Agent
    }
);

v2_enum_from_core!(
    pub enum HookExecutionMode from CoreHookExecutionMode {
        Sync, Async
    }
);

v2_enum_from_core!(
    pub enum HookScope from CoreHookScope {
        Thread, Turn
    }
);

v2_enum_from_core!(
    pub enum HookSource from CoreHookSource {
        System,
        User,
        Project,
        Mdm,
        SessionFlags,
        Plugin,
        CloudRequirements,
        LegacyManagedConfigFile,
        LegacyManagedConfigMdm,
        Unknown,
    }
);

v2_enum_from_core!(
    pub enum HookTrustStatus from CoreHookTrustStatus {
        Managed, Untrusted, Trusted, Modified
    }
);

fn default_hook_source() -> HookSource {
    HookSource::Unknown
}

v2_enum_from_core!(
    pub enum HookRunStatus from CoreHookRunStatus {
        Running, Completed, Failed, Blocked, Stopped
    }
);

v2_enum_from_core!(
    pub enum HookOutputEntryKind from CoreHookOutputEntryKind {
        Warning, Stop, Feedback, Context, Error
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct HookOutputEntry {
    pub kind: HookOutputEntryKind,
    pub text: String,
}

impl From<CoreHookOutputEntry> for HookOutputEntry {
    fn from(value: CoreHookOutputEntry) -> Self {
        Self {
            kind: value.kind.into(),
            text: value.text,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct HookRunEconomy {
    pub decision_type: Option<String>,
    pub command_class: Option<String>,
    pub bypass_reason: Option<String>,
    pub exact_output_reason: Option<String>,
    pub original_bytes: Option<u64>,
    pub replacement_bytes: Option<u64>,
    pub model_visible_bytes: Option<u64>,
    pub output_original_bytes: Option<u64>,
    pub output_model_visible_bytes: Option<u64>,
    pub token_budget: Option<u64>,
    pub original_token_count: Option<u64>,
    pub estimated_saved_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_refs: Vec<String>,
}

impl From<CoreHookRunEconomy> for HookRunEconomy {
    fn from(value: CoreHookRunEconomy) -> Self {
        Self {
            decision_type: value.decision_type,
            command_class: value.command_class,
            bypass_reason: value.bypass_reason,
            exact_output_reason: value.exact_output_reason,
            original_bytes: value.original_bytes,
            replacement_bytes: value.replacement_bytes,
            model_visible_bytes: value.model_visible_bytes,
            output_original_bytes: value.output_original_bytes,
            output_model_visible_bytes: value.output_model_visible_bytes,
            token_budget: value.token_budget,
            original_token_count: value.original_token_count,
            estimated_saved_tokens: value.estimated_saved_tokens,
            artifact_refs: value.artifact_refs,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct HookRunSummary {
    pub id: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub execution_mode: HookExecutionMode,
    pub scope: HookScope,
    pub source_path: AbsolutePathBuf,
    #[serde(default = "default_hook_source")]
    pub source: HookSource,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub trust_status: Option<HookTrustStatus>,
    pub display_order: i64,
    pub status: HookRunStatus,
    pub status_message: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub entries: Vec<HookOutputEntry>,
    #[serde(default)]
    pub economy: Option<HookRunEconomy>,
}

impl From<CoreHookRunSummary> for HookRunSummary {
    fn from(value: CoreHookRunSummary) -> Self {
        Self {
            id: value.id,
            event_name: value.event_name.into(),
            handler_type: value.handler_type.into(),
            execution_mode: value.execution_mode.into(),
            scope: value.scope.into(),
            source_path: value.source_path,
            source: value.source.into(),
            key: value.key,
            plugin_id: value.plugin_id,
            trust_status: value.trust_status.map(Into::into),
            display_order: value.display_order,
            status: value.status.into(),
            status_message: value.status_message,
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
            entries: value.entries.into_iter().map(Into::into).collect(),
            economy: value.economy.map(Into::into),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct HookStartedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct HookCompletedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[cfg(test)]
mod tests {
    use codex_protocol::protocol::HookEventName as CoreHookEventName;
    use codex_protocol::protocol::HookExecutionMode as CoreHookExecutionMode;
    use codex_protocol::protocol::HookHandlerType as CoreHookHandlerType;
    use codex_protocol::protocol::HookRunEconomy as CoreHookRunEconomy;
    use codex_protocol::protocol::HookRunStatus as CoreHookRunStatus;
    use codex_protocol::protocol::HookRunSummary as CoreHookRunSummary;
    use codex_protocol::protocol::HookScope as CoreHookScope;
    use codex_protocol::protocol::HookSource as CoreHookSource;
    use codex_protocol::protocol::HookTrustStatus as CoreHookTrustStatus;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;

    use super::HookRunSummary;

    #[test]
    fn hook_run_summary_converts_economy_fields() {
        let core = CoreHookRunSummary {
            id: "run-1".to_string(),
            event_name: CoreHookEventName::PostToolUse,
            handler_type: CoreHookHandlerType::Command,
            execution_mode: CoreHookExecutionMode::Sync,
            scope: CoreHookScope::Turn,
            source_path: test_path_buf("/tmp/hooks.json").abs(),
            source: CoreHookSource::Plugin,
            key: Some("plugin:key".to_string()),
            plugin_id: Some("plugin-id".to_string()),
            trust_status: Some(CoreHookTrustStatus::Trusted),
            display_order: 7,
            status: CoreHookRunStatus::Completed,
            status_message: Some("ok".to_string()),
            started_at: 10,
            completed_at: Some(20),
            duration_ms: Some(10),
            entries: Vec::new(),
            economy: Some(CoreHookRunEconomy {
                decision_type: Some("compact".to_string()),
                command_class: Some("unified_exec".to_string()),
                bypass_reason: Some("none".to_string()),
                exact_output_reason: Some("not-exact-output".to_string()),
                original_bytes: Some(1000),
                replacement_bytes: Some(200),
                model_visible_bytes: Some(200),
                output_original_bytes: Some(20_000),
                output_model_visible_bytes: Some(5_000),
                token_budget: Some(12_000),
                original_token_count: Some(18_000),
                estimated_saved_tokens: Some(1_200),
                artifact_refs: vec!["artifact-1".to_string()],
            }),
        };

        let converted = HookRunSummary::from(core);
        let economy = converted.economy.expect("economy should convert");

        assert_eq!(converted.key.as_deref(), Some("plugin:key"));
        assert_eq!(converted.plugin_id.as_deref(), Some("plugin-id"));
        assert_eq!(
            converted.trust_status,
            Some(super::HookTrustStatus::Trusted)
        );
        assert_eq!(economy.decision_type.as_deref(), Some("compact"));
        assert_eq!(economy.command_class.as_deref(), Some("unified_exec"));
        assert_eq!(economy.bypass_reason.as_deref(), Some("none"));
        assert_eq!(
            economy.exact_output_reason.as_deref(),
            Some("not-exact-output")
        );
        assert_eq!(economy.original_bytes, Some(1000));
        assert_eq!(economy.replacement_bytes, Some(200));
        assert_eq!(economy.model_visible_bytes, Some(200));
        assert_eq!(economy.output_original_bytes, Some(20_000));
        assert_eq!(economy.output_model_visible_bytes, Some(5_000));
        assert_eq!(economy.token_budget, Some(12_000));
        assert_eq!(economy.original_token_count, Some(18_000));
        assert_eq!(economy.estimated_saved_tokens, Some(1_200));
        assert_eq!(economy.artifact_refs, vec!["artifact-1".to_string()]);
    }

    #[test]
    fn hook_run_economy_defaults_missing_artifact_refs() {
        let economy: super::HookRunEconomy =
            serde_json::from_value(serde_json::json!({})).expect("economy should deserialize");

        assert!(economy.artifact_refs.is_empty());
    }
}
