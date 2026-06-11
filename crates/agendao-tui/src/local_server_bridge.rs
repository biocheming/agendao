use std::path::PathBuf;
use std::sync::Arc;

use agendao_client::{
    AgentInfo, ConnectProviderRequest, CreateSessionRequest, ExecutionModeInfo,
    FullProviderListResponse, KnownProvidersResponse, MessageInfo, MultimodalCapabilitiesResponse,
    MultimodalPolicyResponse, MultimodalPreflightRequest, MultimodalPreflightResponse,
    PermissionRequestInfo, PromptRequest, PromptResponse, ProviderConnectSchemaResponse,
    ProviderDescriptorResponse, ProviderListResponse, QuestionInfo, RefreshProviderCatalogResponse,
    ResolveProviderConnectResponse, SessionListItem, SessionRuntimeState,
    SessionTelemetrySnapshot,
};
use anyhow::Result;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "local-server")]
pub type LocalServerState = agendao_server_local::LocalServerState;
#[cfg(feature = "local-server")]
pub type LocalServerEvent = agendao_server_local::LocalServerEvent;

#[cfg(feature = "local-server")]
pub async fn new_local_server_for_workspace(
    workspace_root: PathBuf,
) -> Result<Arc<LocalServerState>> {
    agendao_server_local::new_local_server_for_workspace(workspace_root).await
}

#[cfg(not(feature = "local-server"))]
#[derive(Debug, Default)]
pub struct LocalServerState;

#[cfg(not(feature = "local-server"))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalServerEvent {
    SessionBusy {
        session_id: String,
    },
    SessionIdle {
        session_id: String,
    },
    SessionUpdated {
        session_id: String,
    },
    OutputBlock {
        session_id: String,
        block: serde_json::Value,
    },
    QuestionCreated {
        session_id: String,
        request_id: String,
    },
    QuestionResolved {
        session_id: String,
        request_id: String,
    },
    PermissionRequested {
        session_id: String,
        permission_id: String,
        info_json: Option<serde_json::Value>,
    },
    PermissionResolved {
        session_id: String,
        permission_id: String,
    },
    ToolCallStarted {
        session_id: String,
    },
    ToolCallCompleted {
        session_id: String,
    },
    ControlInputTransition {
        session_id: String,
        phase: String,
    },
    TopologyChanged {
        session_id: String,
    },
    DiffUpdated {
        session_id: String,
    },
    SessionTreeChanged {
        session_id: String,
    },
}

#[cfg(not(feature = "local-server"))]
pub async fn new_local_server_for_workspace(
    _workspace_root: PathBuf,
) -> Result<Arc<LocalServerState>> {
    Err(anyhow::anyhow!(
        "agendao-tui was built without the `local-server` feature"
    ))
}

#[cfg(feature = "local-server")]
pub fn spawn_direct_event_loop(
    state: Arc<LocalServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> UnboundedReceiver<LocalServerEvent> {
    agendao_server_local::spawn_direct_event_loop(state, session_id, cancel)
}

#[cfg(not(feature = "local-server"))]
pub fn spawn_direct_event_loop(
    _state: Arc<LocalServerState>,
    _session_id: String,
    _cancel: CancellationToken,
) -> UnboundedReceiver<LocalServerEvent> {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    rx
}

#[cfg(feature = "local-server")]
pub async fn local_list_messages(
    state: Arc<LocalServerState>,
    session_id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<MessageInfo>> {
    agendao_server_local::local_list_messages(state, session_id, after, limit).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_messages(
    _state: Arc<LocalServerState>,
    _session_id: &str,
    _after: Option<String>,
    _limit: Option<usize>,
) -> Result<Vec<MessageInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_create_session(
    state: Arc<LocalServerState>,
    request: CreateSessionRequest,
) -> Result<agendao_types::SessionInfo> {
    agendao_server_local::local_create_session(state, request).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_create_session(
    _state: Arc<LocalServerState>,
    _request: CreateSessionRequest,
) -> Result<agendao_types::SessionInfo> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<agendao_types::SessionInfo> {
    agendao_server_local::local_get_session(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<agendao_types::SessionInfo> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_prompt(
    state: Arc<LocalServerState>,
    session_id: &str,
    request: PromptRequest,
) -> Result<PromptResponse> {
    agendao_server_local::local_prompt(state, session_id, request).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_prompt(
    _state: Arc<LocalServerState>,
    _session_id: &str,
    _request: PromptRequest,
) -> Result<PromptResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_list_sessions(
    state: Arc<LocalServerState>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SessionListItem>> {
    agendao_server_local::local_list_sessions(state, search, limit).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_sessions(
    _state: Arc<LocalServerState>,
    _search: Option<String>,
    _limit: Option<usize>,
) -> Result<Vec<SessionListItem>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session_status(
    state: Arc<LocalServerState>,
) -> Result<std::collections::HashMap<String, agendao_types::SessionStatusInfo>> {
    agendao_server_local::local_get_session_status(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session_status(
    _state: Arc<LocalServerState>,
) -> Result<std::collections::HashMap<String, agendao_types::SessionStatusInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_delete_session(state: Arc<LocalServerState>, session_id: &str) -> Result<bool> {
    agendao_server_local::local_delete_session(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_delete_session(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<bool> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_connect_provider(
    state: Arc<LocalServerState>,
    request: ConnectProviderRequest,
) -> Result<()> {
    agendao_server_local::local_connect_provider(state, request).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_connect_provider(
    _state: Arc<LocalServerState>,
    _request: ConnectProviderRequest,
) -> Result<()> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_provider_descriptor(
    state: Arc<LocalServerState>,
    provider_id: &str,
) -> Result<ProviderDescriptorResponse> {
    agendao_server_local::local_get_provider_descriptor(state, provider_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_provider_descriptor(
    _state: Arc<LocalServerState>,
    _provider_id: &str,
) -> Result<ProviderDescriptorResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_list_questions(state: Arc<LocalServerState>) -> Result<Vec<QuestionInfo>> {
    agendao_server_local::local_list_questions(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_questions(_state: Arc<LocalServerState>) -> Result<Vec<QuestionInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_reply_question(
    state: Arc<LocalServerState>,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> Result<()> {
    agendao_server_local::local_reply_question(state, question_id, answers).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_reply_question(
    _state: Arc<LocalServerState>,
    _question_id: &str,
    _answers: Vec<Vec<String>>,
) -> Result<()> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_reject_question(state: Arc<LocalServerState>, question_id: &str) -> Result<()> {
    agendao_server_local::local_reject_question(state, question_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_reject_question(
    _state: Arc<LocalServerState>,
    _question_id: &str,
) -> Result<()> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_list_permissions(
    state: Arc<LocalServerState>,
) -> Result<Vec<PermissionRequestInfo>> {
    agendao_server_local::local_list_permissions(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_permissions(
    _state: Arc<LocalServerState>,
) -> Result<Vec<PermissionRequestInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_reply_permission(
    state: Arc<LocalServerState>,
    permission_id: &str,
    reply: String,
    message: Option<String>,
) -> Result<()> {
    agendao_server_local::local_reply_permission(state, permission_id, reply, message).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_reply_permission(
    _state: Arc<LocalServerState>,
    _permission_id: &str,
    _reply: String,
    _message: Option<String>,
) -> Result<()> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session_runtime(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<SessionRuntimeState> {
    agendao_server_local::local_get_session_runtime(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session_runtime(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<SessionRuntimeState> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session_telemetry(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<SessionTelemetrySnapshot> {
    agendao_server_local::local_get_session_telemetry(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session_telemetry(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<SessionTelemetrySnapshot> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session_todos(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<Vec<agendao_types::SessionTodoInfo>> {
    agendao_server_local::local_get_session_todos(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session_todos(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<Vec<agendao_types::SessionTodoInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_session_diff(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<Vec<agendao_types::FileDiff>> {
    agendao_server_local::local_get_session_diff(state, session_id).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_session_diff(
    _state: Arc<LocalServerState>,
    _session_id: &str,
) -> Result<Vec<agendao_types::FileDiff>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_config_providers(
    state: Arc<LocalServerState>,
) -> Result<ProviderListResponse> {
    agendao_server_local::local_get_config_providers(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_config_providers(
    _state: Arc<LocalServerState>,
) -> Result<ProviderListResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_config(state: Arc<LocalServerState>) -> Result<agendao_config::Config> {
    agendao_server_local::local_get_config(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_config(_state: Arc<LocalServerState>) -> Result<agendao_config::Config> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_config_validation(
    state: Arc<LocalServerState>,
) -> Result<agendao_types::ConfigPolicyValidationSnapshot> {
    agendao_server_local::local_get_config_validation(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_config_validation(
    _state: Arc<LocalServerState>,
) -> Result<agendao_types::ConfigPolicyValidationSnapshot> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_list_agents(state: Arc<LocalServerState>) -> Result<Vec<AgentInfo>> {
    agendao_server_local::local_list_agents(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_agents(_state: Arc<LocalServerState>) -> Result<Vec<AgentInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_list_execution_modes(
    state: Arc<LocalServerState>,
) -> Result<Vec<ExecutionModeInfo>> {
    agendao_server_local::local_list_execution_modes(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_list_execution_modes(
    _state: Arc<LocalServerState>,
) -> Result<Vec<ExecutionModeInfo>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_workspace_context(
    state: Arc<LocalServerState>,
) -> Result<agendao_runtime_context::ResolvedWorkspaceContext> {
    agendao_server_local::local_get_workspace_context(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_workspace_context(
    _state: Arc<LocalServerState>,
) -> Result<agendao_runtime_context::ResolvedWorkspaceContext> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_multimodal_policy(
    state: Arc<LocalServerState>,
) -> Result<MultimodalPolicyResponse> {
    agendao_server_local::local_get_multimodal_policy(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_multimodal_policy(
    _state: Arc<LocalServerState>,
) -> Result<MultimodalPolicyResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_multimodal_capabilities(
    state: Arc<LocalServerState>,
    model: Option<String>,
) -> Result<MultimodalCapabilitiesResponse> {
    agendao_server_local::local_get_multimodal_capabilities(state, model).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_multimodal_capabilities(
    _state: Arc<LocalServerState>,
    _model: Option<String>,
) -> Result<MultimodalCapabilitiesResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_preflight_multimodal(
    state: Arc<LocalServerState>,
    request: MultimodalPreflightRequest,
) -> Result<MultimodalPreflightResponse> {
    agendao_server_local::local_preflight_multimodal(state, request).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_preflight_multimodal(
    _state: Arc<LocalServerState>,
    _request: MultimodalPreflightRequest,
) -> Result<MultimodalPreflightResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_recent_models(
    state: Arc<LocalServerState>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    agendao_server_local::local_get_recent_models(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_recent_models(
    _state: Arc<LocalServerState>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_put_recent_models(
    state: Arc<LocalServerState>,
    recent_models: Vec<agendao_state::RecentModelEntry>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    agendao_server_local::local_put_recent_models(state, recent_models).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_put_recent_models(
    _state: Arc<LocalServerState>,
    _recent_models: Vec<agendao_state::RecentModelEntry>,
) -> Result<Vec<agendao_state::RecentModelEntry>> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_all_providers(
    state: Arc<LocalServerState>,
) -> Result<FullProviderListResponse> {
    agendao_server_local::local_get_all_providers(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_all_providers(
    _state: Arc<LocalServerState>,
) -> Result<FullProviderListResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_known_providers(
    state: Arc<LocalServerState>,
) -> Result<KnownProvidersResponse> {
    agendao_server_local::local_get_known_providers(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_known_providers(
    _state: Arc<LocalServerState>,
) -> Result<KnownProvidersResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_get_provider_connect_schema(
    state: Arc<LocalServerState>,
) -> Result<ProviderConnectSchemaResponse> {
    agendao_server_local::local_get_provider_connect_schema(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_get_provider_connect_schema(
    _state: Arc<LocalServerState>,
) -> Result<ProviderConnectSchemaResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_resolve_provider_connect(
    state: Arc<LocalServerState>,
    query: String,
) -> Result<ResolveProviderConnectResponse> {
    agendao_server_local::local_resolve_provider_connect(state, query).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_resolve_provider_connect(
    _state: Arc<LocalServerState>,
    _query: String,
) -> Result<ResolveProviderConnectResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(feature = "local-server")]
pub async fn local_refresh_provider_catalog(
    state: Arc<LocalServerState>,
) -> Result<RefreshProviderCatalogResponse> {
    agendao_server_local::local_refresh_provider_catalog(state).await
}

#[cfg(not(feature = "local-server"))]
pub async fn local_refresh_provider_catalog(
    _state: Arc<LocalServerState>,
) -> Result<RefreshProviderCatalogResponse> {
    Err(anyhow::anyhow!("local server bridge unavailable"))
}

#[cfg(all(test, feature = "local-server"))]
pub async fn local_register_provider(
    state: &Arc<LocalServerState>,
    provider: Arc<dyn agendao_provider::Provider>,
) {
    agendao_server_local::local_register_provider(state, provider).await
}

#[cfg(all(test, not(feature = "local-server")))]
pub async fn local_register_provider(
    _state: &Arc<LocalServerState>,
    _provider: Arc<dyn agendao_provider::Provider>,
) {
    panic!("local-server feature is required for local provider registration tests");
}
