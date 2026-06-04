use agendao_client::PromptPart;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type LocalServerState = Arc<crate::local_server_bridge::CliLocalServerState>;

pub(super) async fn send_prompt(
    state: &LocalServerState,
    session_id: &str,
    content: String,
    parts: Option<Vec<PromptPart>>,
    agent: Option<String>,
    scheduler_profile: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    ingress_source: Option<String>,
    idempotency_key: Option<String>,
    source_origin: Option<agendao_types::MessageSourceOrigin>,
    source_surface: Option<agendao_types::MessageSourceSurface>,
) -> anyhow::Result<agendao_client::PromptResponse> {
    crate::local_server_bridge::local_prompt(
        Arc::clone(state),
        session_id,
        agendao_client::PromptRequest {
            message: (!content.trim().is_empty()).then_some(content),
            parts,
            idempotency_key,
            ingress_source,
            agent,
            scheduler_profile,
            model,
            variant,
            command: None,
            arguments: None,
            source_origin,
            source_surface,
        },
    )
    .await
}

pub(super) async fn send_command_prompt(
    state: &LocalServerState,
    session_id: &str,
    command: String,
    arguments: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    ingress_source: Option<String>,
    idempotency_key: Option<String>,
    source_origin: Option<agendao_types::MessageSourceOrigin>,
    source_surface: Option<agendao_types::MessageSourceSurface>,
) -> anyhow::Result<agendao_client::PromptResponse> {
    crate::local_server_bridge::local_prompt(
        Arc::clone(state),
        session_id,
        agendao_client::PromptRequest {
            message: None,
            parts: None,
            idempotency_key,
            ingress_source,
            agent: None,
            scheduler_profile: None,
            model,
            variant,
            command: Some(command),
            arguments,
            source_origin,
            source_surface,
        },
    )
    .await
}

pub(super) async fn get_session(
    state: &LocalServerState,
    session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    crate::local_server_bridge::local_get_session(Arc::clone(state), session_id).await
}

pub(super) async fn list_messages(
    state: &LocalServerState,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    crate::local_server_bridge::local_list_messages(Arc::clone(state), session_id).await
}

pub(super) async fn list_questions(
    state: &LocalServerState,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    crate::local_server_bridge::local_list_questions(Arc::clone(state)).await
}

pub(super) async fn reply_question(
    state: &LocalServerState,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    crate::local_server_bridge::local_reply_question(Arc::clone(state), question_id, answers).await
}

pub(super) async fn reject_question(
    state: &LocalServerState,
    question_id: &str,
) -> anyhow::Result<()> {
    crate::local_server_bridge::local_reject_question(Arc::clone(state), question_id).await
}

pub(super) async fn reply_permission(
    state: &LocalServerState,
    permission_id: &str,
    reply: String,
    message: Option<String>,
) -> anyhow::Result<()> {
    crate::local_server_bridge::local_reply_permission(
        Arc::clone(state),
        permission_id,
        reply,
        message,
    )
    .await
}

pub(super) async fn get_workspace_context(
    state: &LocalServerState,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    crate::local_server_bridge::local_get_workspace_context(Arc::clone(state)).await
}

pub(super) async fn list_sessions(
    state: &LocalServerState,
    search: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    crate::local_server_bridge::local_list_sessions(Arc::clone(state), search, limit).await
}

pub(super) async fn get_all_providers(
    state: &LocalServerState,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    crate::local_server_bridge::local_get_all_providers(Arc::clone(state)).await
}

pub(super) async fn get_recent_models(
    state: &LocalServerState,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    crate::local_server_bridge::local_get_recent_models(Arc::clone(state)).await
}

pub(super) async fn put_recent_models(
    state: &LocalServerState,
    recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    crate::local_server_bridge::local_put_recent_models(Arc::clone(state), recent_models).await
}

pub(super) async fn list_execution_modes(
    state: &LocalServerState,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    crate::local_server_bridge::local_list_execution_modes(Arc::clone(state)).await
}

pub(super) async fn list_agents(
    state: &LocalServerState,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    crate::local_server_bridge::local_list_agents(Arc::clone(state)).await
}
