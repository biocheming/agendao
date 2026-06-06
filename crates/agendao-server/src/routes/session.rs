mod autoresearch_target;
mod cancel;
mod effective_policy;
mod events;
mod executions;
mod local_api;
mod messages;
pub(crate) mod prompt;
mod recovery;
mod repair;
mod scheduler;
mod session_crud;
mod stages;
mod steering;
mod telemetry;

use std::sync::Arc;

use axum::{
    routing::{delete, get, patch, post},
    Router,
};

use crate::ServerState;

// ─── Re-exports for sibling route modules (e.g. stream.rs) ─────────────────
pub use self::local_api::{
    local_connect_provider, local_create_session, local_delete_session, local_fork_session,
    local_get_all_providers, local_get_config, local_get_config_providers,
    local_get_config_validation, local_get_known_providers, local_get_multimodal_capabilities,
    local_get_multimodal_policy, local_get_provider_connect_schema, local_get_provider_descriptor,
    local_get_recent_models, local_get_session, local_get_workspace_context, local_list_agents,
    local_list_execution_modes, local_list_messages, local_list_permissions, local_list_questions,
    local_list_sessions, local_preflight_multimodal, local_prompt, local_put_recent_models,
    local_refresh_provider_catalog, local_register_provider, local_reject_question,
    local_reply_permission, local_reply_question, local_resolve_provider_connect,
};
pub(crate) use self::messages::SendMessageRequest;
pub(crate) use self::scheduler::{
    apply_scheduler_selection_session_metadata, collect_skill_tree_validation,
    resolve_prompt_request_config, to_task_agent_info, PromptRequestConfigInput,
    PromptRequestSchedulerProfileSource,
};
pub(crate) use self::session_crud::resolved_session_directory;

// ─── Re-exports for external crates (pub) ──────────────────────────────────
pub use self::scheduler::{
    abort_local_session_execution, run_local_scheduler_prompt, LocalSchedulerPromptOutcome,
    LocalSchedulerPromptRequest,
};

// ─── Imports used only by session_routes() ─────────────────────────────────
use self::cancel::{abort_prompt, abort_scheduler_stage, abort_session};
use self::events::{get_session_event_stages, get_session_events};
use self::executions::{cancel_session_execution, get_session_executions, list_all_executions};
use self::messages::{add_message_part, delete_message, delete_part, list_messages, send_message};
use self::prompt::session_prompt;
use self::recovery::{execute_session_recovery, get_session_recovery};
use self::repair::{get_session_repair_summary, query_session_repair};
use self::session_crud::{
    archive_session, cancel_tool_call, clear_session_revert, create_session, delete_session,
    execute_command, execute_shell, fork_session, get_message, get_session,
    get_session_attached_sessions, get_session_diff, get_session_runtime, get_session_summary,
    get_session_todos, list_sessions, prompt_async, session_revert, session_status,
    session_unrevert, set_session_permission, set_session_summary, set_session_title,
    share_session, start_compaction, unshare_session, update_part, update_session,
};
pub(crate) use self::session_crud::{create_session_from_spec, session_to_info, CreateSessionSpec};
use self::session_crud::{recheck_blocked_session, wake_sleeping_session};
use self::stages::get_session_stages;
use self::steering::submit_session_steering;
use self::telemetry::{get_session_insights, get_session_telemetry};

use super::stream::stream_message;

pub(crate) fn session_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/status", get(session_status))
        .route("/executions", get(list_all_executions))
        .route(
            "/{id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/{id}/attached", get(get_session_attached_sessions))
        .route("/{id}/runtime", get(get_session_runtime))
        .route("/{id}/telemetry", get(get_session_telemetry))
        .route("/{id}/insights", get(get_session_insights))
        .route("/{id}/repair/summary", get(get_session_repair_summary))
        .route("/{id}/repair/query", get(query_session_repair))
        .route("/{id}/steer", post(submit_session_steering))
        .route("/{id}/stages", get(get_session_stages))
        .route("/{id}/executions", get(get_session_executions))
        .route(
            "/{id}/executions/{execution_id}/cancel",
            post(cancel_session_execution),
        )
        .route("/{id}/recovery", get(get_session_recovery))
        .route("/{id}/recovery/execute", post(execute_session_recovery))
        .route("/{id}/todo", get(get_session_todos))
        .route("/{id}/fork", post(fork_session))
        .route("/{id}/abort", post(abort_session))
        .route("/{id}/scheduler/stage/abort", post(abort_scheduler_stage))
        .route("/{id}/share", post(share_session).delete(unshare_session))
        .route("/{id}/archive", post(archive_session))
        .route("/{id}/title", patch(set_session_title))
        .route("/{id}/permission", patch(set_session_permission))
        .route(
            "/{id}/summary",
            get(get_session_summary).patch(set_session_summary),
        )
        .route(
            "/{id}/revert",
            post(session_revert).delete(clear_session_revert),
        )
        .route("/{id}/unrevert", post(session_unrevert))
        .route("/{id}/compact", post(start_compaction))
        .route("/{id}/compaction", post(start_compaction))
        .route("/{id}/command", post(execute_command))
        .route("/{id}/shell", post(execute_shell))
        .route("/{id}/message", post(send_message).get(list_messages))
        .route(
            "/{id}/message/{msgID}",
            get(get_message).delete(delete_message),
        )
        .route("/{id}/message/{msgID}/part", post(add_message_part))
        .route(
            "/{id}/message/{msgID}/part/{partID}",
            delete(delete_part).patch(update_part),
        )
        .route("/{id}/tool/{tool_call_id}/cancel", post(cancel_tool_call))
        .route("/{id}/stream", post(stream_message))
        .route("/{id}/prompt", post(session_prompt))
        .route("/{id}/prompt/abort", post(abort_prompt))
        .route("/{id}/prompt_async", post(prompt_async))
        .route("/{id}/recheck", post(recheck_blocked_session))
        .route("/{id}/wake", post(wake_sleeping_session))
        .route("/{id}/diff", get(get_session_diff))
        .route("/{id}/events", get(get_session_events))
        .route("/{id}/events/stages", get(get_session_event_stages))
}

#[cfg(test)]
mod tests {
    use crate::ApiError;
    use agendao_config::Config as AppConfig;
    use agendao_core::agent_task_registry::{global_task_registry, AgentTaskStatus};
    use agendao_orchestrator::{ModelRef as OrchestratorModelRef, SchedulerProfileConfig};
    use agendao_session::Session;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use self::executions::{
        collect_active_agent_task_execution_records, collect_active_tool_execution_records,
    };

    use super::scheduler::{
        resolve_request_model_inputs, resolve_scheduler_profile_config,
        resolve_scheduler_request_defaults, resolve_scheduler_request_defaults_validated,
        scheduler_mode_kind, scheduler_system_prompt_preview,
    };
    use super::*;

    #[test]
    fn scheduler_model_inputs_prefer_agent_override() {
        let profile = SchedulerProfileConfig {
            model: Some(OrchestratorModelRef {
                provider_id: "ethnopic".to_string(),
                model_id: "test-model-reasoning".to_string(),
            }),
            ..Default::default()
        };

        let (request_model, config_model, config_provider) = resolve_request_model_inputs(
            true,
            Some("openai/gpt-5"),
            Some(&profile),
            Some("ethnopic/test-model-large"),
            Some("ethnopic/test-model-fast"),
        );

        assert_eq!(request_model, None);
        assert_eq!(config_model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(config_provider, None);
    }

    #[test]
    fn scheduler_model_inputs_prefer_profile_override_over_request_model() {
        let profile = SchedulerProfileConfig {
            model: Some(OrchestratorModelRef {
                provider_id: "ethnopic".to_string(),
                model_id: "test-model-reasoning".to_string(),
            }),
            ..Default::default()
        };

        let (request_model, config_model, config_provider) = resolve_request_model_inputs(
            true,
            None,
            Some(&profile),
            Some("openai/gpt-5"),
            Some("ethnopic/test-model-fast"),
        );

        assert_eq!(request_model, None);
        assert_eq!(config_model.as_deref(), Some("test-model-reasoning"));
        assert_eq!(config_provider.as_deref(), Some("ethnopic"));
    }

    #[test]
    fn scheduler_model_inputs_fall_back_to_request_model_when_no_overrides_exist() {
        let (request_model, config_model, config_provider) = resolve_request_model_inputs(
            true,
            None,
            None,
            Some("openai/gpt-5"),
            Some("ethnopic/test-model-fast"),
        );

        assert_eq!(request_model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(config_model.as_deref(), Some("ethnopic/test-model-fast"));
        assert_eq!(config_provider, None);
    }

    #[test]
    fn non_scheduler_model_inputs_keep_request_then_agent_precedence() {
        let (request_model, config_model, config_provider) = resolve_request_model_inputs(
            false,
            Some("ethnopic/test-model-reasoning"),
            None,
            Some("openai/gpt-5"),
            Some("ethnopic/test-model-fast"),
        );

        assert_eq!(request_model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(
            config_model.as_deref(),
            Some("ethnopic/test-model-reasoning")
        );
        assert_eq!(config_provider, None);
    }

    #[test]
    fn builtin_preset_defaults_resolve_without_external_scheduler_file() {
        for name in &["auto", "sisyphus", "verifier"] {
            let defaults = resolve_scheduler_request_defaults(&AppConfig::default(), Some(name))
                .expect("builtin preset should resolve without schedulerPath");
            assert_eq!(defaults.profile_name.as_deref(), Some(*name));
        }
    }

    #[test]
    fn builtin_presets_resolve_as_preset_modes() {
        for name in &["auto", "sisyphus", "verifier"] {
            let defaults = resolve_scheduler_request_defaults(&AppConfig::default(), Some(name))
                .expect("builtin preset should resolve without schedulerPath");
            assert_eq!(defaults.profile_name.as_deref(), Some(*name));
            assert_eq!(scheduler_mode_kind(name), "preset");
        }
    }

    #[test]
    fn builtin_autoresearch_profile_resolves_without_external_scheduler_file() {
        let defaults =
            resolve_scheduler_request_defaults(&AppConfig::default(), Some("autoresearch-run"))
                .expect("built-in autoresearch profile should resolve without schedulerPath");

        assert_eq!(defaults.profile_name.as_deref(), Some("autoresearch-run"));
        assert_eq!(scheduler_mode_kind("autoresearch-run"), "profile");
    }

    #[test]
    fn builtin_autoresearch_profile_config_resolves_without_external_scheduler_file() {
        let (profile_name, profile) =
            resolve_scheduler_profile_config(&AppConfig::default(), Some("autoresearch-run"))
                .expect("built-in autoresearch profile config should resolve");

        assert_eq!(profile_name, "autoresearch-run");
        assert_eq!(profile.orchestrator.as_deref(), Some("hephaestus"));
    }

    #[test]
    fn builtin_auto_profile_config_resolves_without_external_scheduler_file() {
        let (profile_name, profile) =
            resolve_scheduler_profile_config(&AppConfig::default(), Some("auto"))
                .expect("built-in auto profile config should resolve");

        assert_eq!(profile_name, "auto");
        assert_eq!(profile.orchestrator.as_deref(), Some("sisyphus"));
        assert!(profile
            .stage_kinds()
            .contains(&agendao_orchestrator::SchedulerStageKind::Route));
    }

    #[test]
    fn workspace_autoresearch_profile_overrides_bundled_defaults() {
        let temp = std::env::temp_dir().join(format!(
            "agendao_server_autoresearch_override_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        let scheduler_path = temp.join("autoresearch.jsonc");
        fs::write(
            &scheduler_path,
            r#"{
  "defaults": { "profile": "autoresearch-run" },
  "profiles": {
    "autoresearch-run": {
      "orchestrator": "atlas",
      "workflow": {
        "workflow": { "kind": "autoresearch", "mode": "run" },
        "objective": {
          "goal": "demo goal",
          "scope": { "include": ["book/**"] },
          "direction": "higher-is-better",
          "metric": { "kind": "numeric-extract", "pattern": "score=([0-9]+)" },
          "verify": { "command": "bash ./scripts/verify-autoresearch.sh" }
        },
        "iterationPolicy": { "mode": "bounded", "maxIterations": 30 },
        "decisionPolicy": { "baselineStrategy": "capture-before-first-iteration" },
        "workspacePolicy": { "snapshotStrategy": "patch-file" }
      }
    }
  }
}"#,
        )
        .expect("write scheduler");

        let config = AppConfig {
            scheduler_path: Some(scheduler_path.display().to_string()),
            ..AppConfig::default()
        };

        let defaults = resolve_scheduler_request_defaults(&config, Some("autoresearch-run"))
            .expect("workspace profile should resolve");
        assert_eq!(defaults.profile_name.as_deref(), Some("autoresearch-run"));

        let (_, profile) = resolve_scheduler_profile_config(&config, Some("autoresearch-run"))
            .expect("workspace profile config should resolve");
        assert_eq!(profile.orchestrator.as_deref(), Some("atlas"));

        let _ = fs::remove_dir_all(PathBuf::from(&temp));
    }

    #[test]
    fn explicit_unknown_scheduler_profile_returns_bad_request_instead_of_falling_back() {
        let error =
            resolve_scheduler_request_defaults_validated(&AppConfig::default(), Some("missing"))
                .expect_err("explicitly requested unknown scheduler profile should fail");

        match error {
            ApiError::BadRequest(message) => {
                assert!(message.contains("Scheduler profile could not be resolved"));
                assert!(message.contains("missing"));
            }
            other => panic!("expected bad request, got {other:?}"),
        }
    }

    #[test]
    fn preset_preview_dispatches_to_orchestrator_and_returns_nonempty_third_person() {
        // Server layer only validates dispatch behaviour:
        // - known preset names resolve to a non-empty preview from the orchestrator
        // - the preview uses third-person "You are" framing (not "I'm")
        // Exact prompt wording is owned by agendao-orchestrator presets.
        for name in &["atlas", "prometheus", "sisyphus", "hephaestus", "verifier"] {
            let profile = SchedulerProfileConfig {
                orchestrator: Some(name.to_string()),
                ..Default::default()
            };
            let preview = scheduler_system_prompt_preview(name, &profile);
            assert!(
                !preview.is_empty(),
                "preview for preset '{name}' should not be empty"
            );
            assert!(
                preview.starts_with("You are"),
                "preview for preset '{name}' should use third-person framing, got: {preview}"
            );
            assert!(
                !preview.contains(&format!("I'm {}", capitalize_first(name))),
                "preview for preset '{name}' should not contain first-person intro"
            );
        }
    }

    #[test]
    fn unknown_profile_preview_returns_generic_fallback() {
        let profile = SchedulerProfileConfig::default();
        let preview = scheduler_system_prompt_preview("custom-profile", &profile);
        assert!(
            preview.contains("custom-profile"),
            "fallback preview should mention the profile name"
        );
        assert!(!preview.is_empty(), "fallback preview should not be empty");
    }

    #[test]
    fn active_tool_execution_records_attach_to_active_stage() {
        let mut session = Session::new("proj", "/tmp");
        let session_id = session.id.clone();
        let mut assistant = agendao_session::SessionMessage::assistant(session_id.clone());
        assistant.add_tool_call("call_1", "bash", serde_json::json!({"command": "echo hi"}));
        session.push_message(assistant);

        let records = vec![
            agendao_server_core::runtime_control::ExecutionRecord {
                id: format!("prompt:{session_id}"),
                session_id: session_id.clone(),
                kind: agendao_server_core::runtime_control::ExecutionKind::PromptRun,
                status: agendao_server_core::runtime_control::ExecutionStatus::Running,
                label: Some("Prompt run".to_string()),
                parent_id: None,
                stage_id: None,
                waiting_on: None,
                recent_event: None,
                started_at: 1,
                updated_at: 1,
                metadata: None,
            },
            agendao_server_core::runtime_control::ExecutionRecord {
                id: format!("scheduler:{session_id}"),
                session_id: session_id.clone(),
                kind: agendao_server_core::runtime_control::ExecutionKind::SchedulerRun,
                status: agendao_server_core::runtime_control::ExecutionStatus::Running,
                label: Some("Scheduler run".to_string()),
                parent_id: Some(format!("prompt:{session_id}")),
                stage_id: None,
                waiting_on: None,
                recent_event: None,
                started_at: 2,
                updated_at: 2,
                metadata: None,
            },
            agendao_server_core::runtime_control::ExecutionRecord {
                id: "msg_stage_1".to_string(),
                session_id: session.id.clone(),
                kind: agendao_server_core::runtime_control::ExecutionKind::SchedulerStage,
                status: agendao_server_core::runtime_control::ExecutionStatus::Running,
                label: Some("Plan".to_string()),
                parent_id: Some("scheduler:ses_tools".to_string()),
                stage_id: Some("msg_stage_1".to_string()),
                waiting_on: None,
                recent_event: None,
                started_at: 3,
                updated_at: 3,
                metadata: None,
            },
        ];

        let tool_records = collect_active_tool_execution_records(&session, &records);
        assert_eq!(tool_records.len(), 1);
        let tool = &tool_records[0];
        assert!(matches!(
            tool.kind,
            agendao_server_core::runtime_control::ExecutionKind::ToolCall
        ));
        assert_eq!(tool.parent_id.as_deref(), Some("msg_stage_1"));
        assert_eq!(tool.label.as_deref(), Some("Tool: bash"));
    }

    #[test]
    fn active_agent_task_execution_records_are_session_scoped() {
        let session_id = "ses_agent_tasks";
        let task_id = global_task_registry().register(
            Some(session_id.to_string()),
            "atlas".to_string(),
            "Verify implementation".to_string(),
            Some(4),
            Arc::new(|| {}),
        );
        let other_id = global_task_registry().register(
            Some("ses_other".to_string()),
            "atlas".to_string(),
            "Ignore me".to_string(),
            Some(2),
            Arc::new(|| {}),
        );

        let records = vec![agendao_server_core::runtime_control::ExecutionRecord {
            id: format!("prompt:{session_id}"),
            session_id: session_id.to_string(),
            kind: agendao_server_core::runtime_control::ExecutionKind::PromptRun,
            status: agendao_server_core::runtime_control::ExecutionStatus::Running,
            label: Some("Prompt run".to_string()),
            parent_id: None,
            stage_id: None,
            waiting_on: None,
            recent_event: None,
            started_at: 1,
            updated_at: 1,
            metadata: None,
        }];

        let task_records = collect_active_agent_task_execution_records(session_id, &records);
        assert_eq!(task_records.len(), 1);
        let task = &task_records[0];
        assert!(matches!(
            task.kind,
            agendao_server_core::runtime_control::ExecutionKind::AgentTask
        ));
        assert_eq!(
            task.parent_id.as_deref(),
            Some(format!("prompt:{session_id}").as_str())
        );
        assert_eq!(task.label.as_deref(), Some("Agent task: atlas"));

        global_task_registry().complete(&task_id, AgentTaskStatus::Cancelled);
        global_task_registry().complete(&other_id, AgentTaskStatus::Cancelled);
    }

    fn capitalize_first(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().to_string() + c.as_str(),
        }
    }

    #[tokio::test]
    async fn event_query_filters_by_stage_id_via_stage_event_log() {
        use agendao_stage_protocol::{EventScope, StageEvent};

        let log = agendao_server_core::stage_event_log::StageEventLog::new();
        let session_id = "ses_event_test";

        log.record(
            session_id,
            StageEvent {
                event_id: "evt_1".into(),
                scope: EventScope::Stage,
                stage_id: Some("stg_alpha".into()),
                execution_id: Some("ex_1".into()),
                event_type: "execution.topology.changed".into(),
                ts: 1000,
                payload: serde_json::json!({}),
            },
        )
        .await;
        log.record(
            session_id,
            StageEvent {
                event_id: "evt_2".into(),
                scope: EventScope::Stage,
                stage_id: Some("stg_beta".into()),
                execution_id: Some("ex_2".into()),
                event_type: "execution.topology.changed".into(),
                ts: 2000,
                payload: serde_json::json!({}),
            },
        )
        .await;
        log.record(
            session_id,
            StageEvent {
                event_id: "evt_3".into(),
                scope: EventScope::Stage,
                stage_id: Some("stg_alpha".into()),
                execution_id: Some("ex_3".into()),
                event_type: "agent.started".into(),
                ts: 3000,
                payload: serde_json::json!({}),
            },
        )
        .await;

        // Filter by stage_id
        let filter_stage = agendao_server_core::stage_event_log::EventFilter {
            stage_id: Some("stg_alpha".into()),
            ..Default::default()
        };
        let results = log.query(session_id, &filter_stage).await;
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|e| e.stage_id.as_deref() == Some("stg_alpha")));

        // Filter by stage_id + event_type
        let filter_combined = agendao_server_core::stage_event_log::EventFilter {
            stage_id: Some("stg_alpha".into()),
            event_type: Some("agent.started".into()),
            ..Default::default()
        };
        let results = log.query(session_id, &filter_combined).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].event_id, "evt_3");

        // stage_ids lists distinct stages
        let stage_ids = log.stage_ids(session_id).await;
        assert_eq!(stage_ids, vec!["stg_alpha", "stg_beta"]);
    }
}
