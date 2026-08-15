mod blueprint;
mod cancel;
mod effective_policy;
mod executions;
mod local_api;
mod messages;
pub(crate) mod prompt;
mod recovery;
mod repair;
mod scheduler;
mod session_crud;
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
    local_abort_session, local_authenticate_mcp, local_cancel_tool_call, local_compact_session,
    local_connect_mcp, local_connect_provider, local_create_session, local_delete_mcp_config,
    local_delete_plugin_config, local_delete_provider, local_delete_provider_model_config,
    local_delete_session, local_disconnect_mcp, local_execute_session_recovery,
    local_execute_shell, local_fork_session, local_get_all_providers, local_get_config,
    local_get_config_providers, local_get_config_validation, local_get_known_providers,
    local_get_mcp_status, local_get_multimodal_capabilities, local_get_multimodal_policy,
    local_get_provider_connect_schema, local_get_provider_descriptor,
    local_get_provider_model_config, local_get_recent_models, local_get_session,
    local_get_session_diff, local_get_session_recovery, local_get_session_runtime,
    local_get_session_status, local_get_session_telemetry, local_get_session_todos,
    local_get_skill_detail, local_get_workspace_context, local_list_agents,
    local_list_execution_modes, local_list_messages, local_list_permissions, local_list_plugins,
    local_list_questions, local_list_sessions, local_list_skill_proposals, local_list_skills,
    local_list_tools, local_manage_skill, local_patch_config, local_preflight_multimodal,
    local_prompt, local_put_disabled_config, local_put_mcp_config, local_put_plugin_config,
    local_put_provider_model_config, local_put_recent_models, local_refresh_provider_catalog,
    local_register_provider, local_reject_question, local_reload_config, local_remove_mcp_auth,
    local_reply_permission, local_reply_question, local_resolve_provider_connect,
    local_set_provider_disabled, local_start_mcp_auth, local_test_provider_connection,
    local_update_provider, local_update_session_title, local_update_skill_proposal_status,
};
pub(crate) use self::scheduler::{
    scheduler_host_tool_definitions, SessionSchedulerToolExecutor,
    SessionSchedulerToolExecutorInput,
};

// ─── Re-exports for external crates (pub) ──────────────────────────────────

// ─── Imports used only by session_routes() ─────────────────────────────────
use self::blueprint::{get_session_blueprint, reject_session_blueprint, set_session_blueprint};
use self::cancel::{abort_prompt, abort_session};
use self::executions::{cancel_session_execution, get_session_executions, list_all_executions};
use self::messages::{add_message_part, delete_message, delete_part, list_messages, send_message};
use self::prompt::session_prompt;
use self::recovery::{execute_session_recovery, get_session_recovery};
use self::repair::{get_session_repair_summary, query_session_repair};
use self::session_crud::{
    archive_session, cancel_tool_call, clear_session_revert, create_session, delete_session,
    execute_command, execute_shell, fork_session, get_message, get_session, get_session_diff,
    get_session_runtime, get_session_summary, get_session_todos, list_sessions, prompt_async,
    session_revert, session_status, session_unrevert, set_session_permission, set_session_summary,
    set_session_title, share_session, start_compaction, unshare_session, update_part,
    update_session,
};
pub(crate) use self::session_crud::{create_session_from_spec, session_to_info, CreateSessionSpec};
use self::session_crud::{recheck_blocked_session, wake_sleeping_session};
use self::steering::submit_session_steering;
use self::telemetry::{get_session_insights, get_session_telemetry};

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
        .route("/{id}/runtime", get(get_session_runtime))
        .route(
            "/{id}/blueprint",
            get(get_session_blueprint)
                .put(set_session_blueprint)
                .delete(reject_session_blueprint),
        )
        .route("/{id}/telemetry", get(get_session_telemetry))
        .route("/{id}/insights", get(get_session_insights))
        .route("/{id}/repair/summary", get(get_session_repair_summary))
        .route("/{id}/repair/query", get(query_session_repair))
        .route("/{id}/steer", post(submit_session_steering))
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
        .route("/{id}/prompt", post(session_prompt))
        .route("/{id}/prompt/abort", post(abort_prompt))
        .route("/{id}/prompt_async", post(prompt_async))
        .route("/{id}/recheck", post(recheck_blocked_session))
        .route("/{id}/wake", post(wake_sleeping_session))
        .route("/{id}/diff", get(get_session_diff))
}

#[cfg(test)]
mod tests {
    use agendao_session::Session;

    use self::executions::collect_active_tool_execution_records;

    use super::*;

    #[test]
    fn active_tool_execution_records_attach_to_active_scheduler_node() {
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
                id: "scheduler_node:test:root/plan".to_string(),
                session_id: session.id.clone(),
                kind: agendao_server_core::runtime_control::ExecutionKind::SchedulerNode,
                status: agendao_server_core::runtime_control::ExecutionStatus::Running,
                label: Some("Plan".to_string()),
                parent_id: Some("scheduler:ses_tools".to_string()),
                stage_id: None,
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
        assert_eq!(
            tool.parent_id.as_deref(),
            Some("scheduler_node:test:root/plan")
        );
        assert_eq!(tool.label.as_deref(), Some("Tool: bash"));
    }
}
