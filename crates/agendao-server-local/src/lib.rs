use std::path::PathBuf;
use std::sync::Arc;

use agendao_client::{
    AgentInfo, ConnectProviderRequest, CreateSessionRequest, ExecutionModeInfo,
    FullProviderListResponse, KnownProvidersResponse, MessageInfo, MultimodalCapabilitiesResponse,
    MultimodalPolicyResponse, MultimodalPreflightRequest, MultimodalPreflightResponse,
    PermissionRequestInfo, PromptRequest, PromptResponse, ProviderConnectSchemaResponse,
    ProviderDescriptorResponse, ProviderListResponse, QuestionInfo, RefreshProviderCatalogResponse,
    ResolveProviderConnectResponse, SessionListItem,
};
use anyhow::Result;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

pub type LocalServerState = agendao_server::ServerState;
pub type LocalServerEvent = agendao_server::DirectEvent;

pub async fn new_local_server_for_workspace(
    workspace_root: PathBuf,
) -> Result<Arc<LocalServerState>> {
    Ok(Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(
            "http://127.0.0.1:0".to_string(),
            workspace_root,
        )
        .await?,
    ))
}

pub fn spawn_direct_event_loop(
    state: Arc<LocalServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> UnboundedReceiver<LocalServerEvent> {
    agendao_server::spawn_direct_event_loop(state, session_id, cancel)
}

pub async fn local_list_messages(
    state: Arc<LocalServerState>,
    session_id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MessageInfo>> {
    agendao_server::local_list_messages(state, session_id, after, limit).await
}

pub async fn local_create_session(
    state: Arc<LocalServerState>,
    request: CreateSessionRequest,
) -> Result<agendao_types::SessionInfo> {
    agendao_server::local_create_session(state, request).await
}

pub async fn local_get_session(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<agendao_types::SessionInfo> {
    agendao_server::local_get_session(state, session_id).await
}

pub async fn local_prompt(
    state: Arc<LocalServerState>,
    session_id: &str,
    request: PromptRequest,
) -> Result<PromptResponse> {
    agendao_server::local_prompt(state, session_id, request).await
}

pub async fn local_list_sessions(
    state: Arc<LocalServerState>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SessionListItem>> {
    agendao_server::local_list_sessions(state, search, limit).await
}

pub async fn local_connect_provider(
    state: Arc<LocalServerState>,
    request: ConnectProviderRequest,
) -> Result<()> {
    agendao_server::local_connect_provider(state, request).await
}

pub async fn local_get_provider_descriptor(
    state: Arc<LocalServerState>,
    provider_id: &str,
) -> Result<ProviderDescriptorResponse> {
    agendao_server::local_get_provider_descriptor(state, provider_id).await
}

pub async fn local_list_questions(state: Arc<LocalServerState>) -> Result<Vec<QuestionInfo>> {
    agendao_server::local_list_questions(state).await
}

pub async fn local_reply_question(
    state: Arc<LocalServerState>,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> Result<()> {
    agendao_server::local_reply_question(state, question_id, answers).await
}

pub async fn local_reject_question(state: Arc<LocalServerState>, question_id: &str) -> Result<()> {
    agendao_server::local_reject_question(state, question_id).await
}

pub async fn local_list_permissions(
    state: Arc<LocalServerState>,
) -> Result<Vec<PermissionRequestInfo>> {
    agendao_server::local_list_permissions(state).await
}

pub async fn local_reply_permission(
    state: Arc<LocalServerState>,
    permission_id: &str,
    reply: String,
    message: Option<String>,
) -> Result<()> {
    agendao_server::local_reply_permission(state, permission_id, reply, message).await
}

pub async fn local_get_config_providers(
    state: Arc<LocalServerState>,
) -> Result<ProviderListResponse> {
    agendao_server::local_get_config_providers(state).await
}

pub async fn local_get_config(state: Arc<LocalServerState>) -> Result<agendao_config::Config> {
    agendao_server::local_get_config(state).await
}

pub async fn local_get_config_validation(
    state: Arc<LocalServerState>,
) -> Result<agendao_types::ConfigPolicyValidationSnapshot> {
    agendao_server::local_get_config_validation(state).await
}

pub async fn local_list_agents(state: Arc<LocalServerState>) -> Result<Vec<AgentInfo>> {
    agendao_server::local_list_agents(state).await
}

pub async fn local_list_execution_modes(
    state: Arc<LocalServerState>,
) -> Result<Vec<ExecutionModeInfo>> {
    agendao_server::local_list_execution_modes(state).await
}

pub async fn local_get_workspace_context(
    state: Arc<LocalServerState>,
) -> Result<agendao_runtime_context::ResolvedWorkspaceContext> {
    agendao_server::local_get_workspace_context(state).await
}

pub async fn local_get_multimodal_policy(
    state: Arc<LocalServerState>,
) -> Result<MultimodalPolicyResponse> {
    agendao_server::local_get_multimodal_policy(state).await
}

pub async fn local_get_multimodal_capabilities(
    state: Arc<LocalServerState>,
    model: Option<String>,
) -> Result<MultimodalCapabilitiesResponse> {
    agendao_server::local_get_multimodal_capabilities(state, model).await
}

pub async fn local_preflight_multimodal(
    state: Arc<LocalServerState>,
    request: MultimodalPreflightRequest,
) -> Result<MultimodalPreflightResponse> {
    agendao_server::local_preflight_multimodal(state, request).await
}

pub async fn local_get_recent_models(
    state: Arc<LocalServerState>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    agendao_server::local_get_recent_models(state).await
}

pub async fn local_put_recent_models(
    state: Arc<LocalServerState>,
    recent_models: Vec<agendao_state::RecentModelEntry>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    agendao_server::local_put_recent_models(state, recent_models).await
}

pub async fn local_get_all_providers(
    state: Arc<LocalServerState>,
) -> Result<FullProviderListResponse> {
    agendao_server::local_get_all_providers(state).await
}

pub async fn local_get_known_providers(
    state: Arc<LocalServerState>,
) -> Result<KnownProvidersResponse> {
    agendao_server::local_get_known_providers(state).await
}

pub async fn local_get_provider_connect_schema(
    state: Arc<LocalServerState>,
) -> Result<ProviderConnectSchemaResponse> {
    agendao_server::local_get_provider_connect_schema(state).await
}

pub async fn local_resolve_provider_connect(
    state: Arc<LocalServerState>,
    query: String,
) -> Result<ResolveProviderConnectResponse> {
    agendao_server::local_resolve_provider_connect(state, query).await
}

pub async fn local_refresh_provider_catalog(
    state: Arc<LocalServerState>,
) -> Result<RefreshProviderCatalogResponse> {
    agendao_server::local_refresh_provider_catalog(state).await
}

pub async fn local_register_provider(
    state: &Arc<LocalServerState>,
    provider: Arc<dyn agendao_provider::Provider>,
) {
    agendao_server::local_register_provider(state, provider).await
}
