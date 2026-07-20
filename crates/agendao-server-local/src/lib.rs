use std::path::PathBuf;
use std::sync::Arc;

use agendao_server_core::frontend_events::FrontendEvent;
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
pub type LocalServerEvent = FrontendEvent;

pub async fn new_local_server_for_workspace(
    workspace_root: PathBuf,
) -> Result<Arc<LocalServerState>> {
    let state = Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(
            "http://127.0.0.1:0".to_string(),
            workspace_root,
        )
        .await?,
    );
    state.ensure_frontend_projector();
    Ok(state)
}

pub fn spawn_direct_event_loop(
    state: Arc<LocalServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> UnboundedReceiver<LocalServerEvent> {
    agendao_server::spawn_direct_event_loop(state, session_id, cancel)
}

pub fn spawn_direct_event_bus(
    state: Arc<LocalServerState>,
    cancel: CancellationToken,
) -> UnboundedReceiver<LocalServerEvent> {
    agendao_server::spawn_direct_event_bus(state, cancel)
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
    local_list_sessions_in_directory(state, None, search, limit).await
}

/// List sessions filtered by exact directory match (canonical path).
///
/// `directory = None` returns all sessions across the workspace,
/// matching legacy behaviour. `directory = Some(path)` scopes the
/// result to sessions whose `record().directory == path`.
pub async fn local_list_sessions_in_directory(
    state: Arc<LocalServerState>,
    directory: Option<String>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SessionListItem>> {
    agendao_server::local_list_sessions(state, directory, search, limit).await
}

pub async fn local_get_session_status(
    state: Arc<LocalServerState>,
) -> Result<std::collections::HashMap<String, agendao_types::SessionStatusInfo>> {
    agendao_server::local_get_session_status(state).await
}

pub async fn local_delete_session(state: Arc<LocalServerState>, session_id: &str) -> Result<bool> {
    agendao_server::local_delete_session(state, session_id).await
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

pub async fn local_update_provider(
    state: Arc<LocalServerState>,
    provider_id: &str,
    name: Option<String>,
    base_url: Option<String>,
    protocol: Option<String>,
) -> Result<bool> {
    agendao_server::local_update_provider(state, provider_id, name, base_url, protocol).await
}

pub async fn local_set_provider_disabled(
    state: Arc<LocalServerState>,
    provider_id: &str,
    disabled: bool,
) -> Result<bool> {
    agendao_server::local_set_provider_disabled(state, provider_id, disabled).await
}

pub async fn local_test_provider_connection(
    state: Arc<LocalServerState>,
    provider_id: &str,
) -> Result<agendao_api::TestProviderConnectionResponse> {
    agendao_server::local_test_provider_connection(state, provider_id).await
}

pub async fn local_delete_provider(
    state: Arc<LocalServerState>,
    provider_id: &str,
) -> Result<bool> {
    agendao_server::local_delete_provider(state, provider_id).await
}

pub async fn local_get_provider_model_config(
    state: Arc<LocalServerState>,
    provider_id: &str,
    model_key: &str,
) -> Result<agendao_config::ModelConfig> {
    agendao_server::local_get_provider_model_config(state, provider_id, model_key).await
}

pub async fn local_put_provider_model_config(
    state: Arc<LocalServerState>,
    provider_id: &str,
    model_key: &str,
    model: agendao_config::ModelConfig,
) -> Result<agendao_config::Config> {
    agendao_server::local_put_provider_model_config(state, provider_id, model_key, model).await
}

pub async fn local_delete_provider_model_config(
    state: Arc<LocalServerState>,
    provider_id: &str,
    model_key: &str,
) -> Result<agendao_config::Config> {
    agendao_server::local_delete_provider_model_config(state, provider_id, model_key).await
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

pub async fn local_get_session_runtime(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<agendao_client::SessionRuntimeState> {
    let runtime = agendao_server::local_get_session_runtime(state, session_id).await?;
    Ok(serde_json::from_value(serde_json::to_value(runtime)?)?)
}

pub async fn local_get_session_telemetry(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<agendao_client::SessionTelemetrySnapshot> {
    let snapshot = agendao_server::local_get_session_telemetry(state, session_id).await?;
    Ok(serde_json::from_value(serde_json::to_value(snapshot)?)?)
}

pub async fn local_get_session_todos(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<Vec<agendao_types::SessionTodoInfo>> {
    agendao_server::local_get_session_todos(state, session_id).await
}

pub async fn local_get_session_diff(
    state: Arc<LocalServerState>,
    session_id: &str,
) -> Result<Vec<agendao_types::FileDiff>> {
    agendao_server::local_get_session_diff(state, session_id).await
}

pub async fn local_get_config_providers(
    state: Arc<LocalServerState>,
) -> Result<ProviderListResponse> {
    agendao_server::local_get_config_providers(state).await
}

pub async fn local_get_config(state: Arc<LocalServerState>) -> Result<agendao_config::Config> {
    agendao_server::local_get_config(state).await
}

pub async fn local_patch_config(
    state: Arc<LocalServerState>,
    patch: serde_json::Value,
) -> Result<agendao_config::Config> {
    agendao_server::local_patch_config(state, patch).await
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

/// GET `/skill/catalog` 的 local-direct 短路：TUI Settings skills 读通路。
pub async fn local_list_skills(
    state: Arc<LocalServerState>,
    query: agendao_client::SkillCatalogQuery,
) -> Result<Vec<agendao_client::SkillCatalogEntry>> {
    agendao_server::local_list_skills(state, query).await
}

/// GET `/tool/catalog` 的 local-direct 短路：TUI Settings→Tools 读通路。
pub async fn local_list_tools(
    state: Arc<LocalServerState>,
) -> Result<Vec<agendao_client::ToolListEntry>> {
    agendao_server::local_list_tools(state).await
}

/// PUT `/config/disabled` 的 local-direct 短路：TUI Settings skills/tools
/// 启停写通路（`Some(vec)` 整体替换，允许空 vec 清空）。
pub async fn local_put_disabled_config(
    state: Arc<LocalServerState>,
    update: agendao_client::DisabledConfigUpdate,
) -> Result<agendao_config::Config> {
    agendao_server::local_put_disabled_config(state, update).await
}

/// GET `/skill/proposal/?status=` 的 local-direct 短路：TUI Settings 提案读通路。
pub async fn local_list_skill_proposals(
    state: Arc<LocalServerState>,
    status: &str,
) -> Result<Vec<agendao_client::SkillEvolutionProposal>> {
    agendao_server::local_list_skill_proposals(state, status).await
}

/// POST `/skill/manage` 的 local-direct 短路：TUI Settings skills 管理写通路
/// （本阶段用于 Delete）。
pub async fn local_manage_skill(
    state: Arc<LocalServerState>,
    request: agendao_client::SkillManageRequest,
) -> Result<agendao_client::SkillManageResponse> {
    agendao_server::local_manage_skill(state, request).await
}

/// POST `/skill/proposal/{id}/status` 的 local-direct 短路（approve/reject）。
pub async fn local_update_skill_proposal_status(
    state: Arc<LocalServerState>,
    id: &str,
    status: &str,
) -> Result<agendao_client::SkillEvolutionProposal> {
    agendao_server::local_update_skill_proposal_status(state, id, status).await
}

/// GET `/mcp` 的 local-direct 短路：返回已按 name 排序的 Vec（同 HTTP client 语义）。
pub async fn local_get_mcp_status(
    state: Arc<LocalServerState>,
) -> Result<Vec<agendao_client::McpStatusInfo>> {
    agendao_server::local_get_mcp_status(state).await
}

/// POST `/mcp/{name}/connect` 的 local-direct 短路。
pub async fn local_connect_mcp(state: Arc<LocalServerState>, name: &str) -> Result<bool> {
    agendao_server::local_connect_mcp(state, name).await
}

/// POST `/mcp/{name}/disconnect` 的 local-direct 短路。
pub async fn local_disconnect_mcp(state: Arc<LocalServerState>, name: &str) -> Result<bool> {
    agendao_server::local_disconnect_mcp(state, name).await
}

/// PUT `/config/mcp/{key}` 的 local-direct 短路（TUI Settings→MCP 增/改/启停）。
pub async fn local_put_mcp_config(
    state: Arc<LocalServerState>,
    key: &str,
    mcp: agendao_config::McpServerConfig,
) -> Result<agendao_config::Config> {
    agendao_server::local_put_mcp_config(state, key, mcp).await
}

/// DELETE `/config/mcp/{key}` 的 local-direct 短路。
pub async fn local_delete_mcp_config(
    state: Arc<LocalServerState>,
    key: &str,
) -> Result<agendao_config::Config> {
    agendao_server::local_delete_mcp_config(state, key).await
}

/// PUT `/config/plugin/{key}` 的 local-direct 短路（TUI Settings→Plugins 安装）。
pub async fn local_put_plugin_config(
    state: Arc<LocalServerState>,
    key: &str,
    plugin: agendao_config::PluginConfig,
) -> Result<agendao_config::Config> {
    agendao_server::local_put_plugin_config(state, key, plugin).await
}

/// DELETE `/config/plugin/{key}` 的 local-direct 短路（managed 条目删除）。
pub async fn local_delete_plugin_config(
    state: Arc<LocalServerState>,
    key: &str,
) -> Result<agendao_config::Config> {
    agendao_server::local_delete_plugin_config(state, key).await
}

/// GET `/config/plugins` 的 local-direct 短路：TUI Settings→Plugins 读通路。
pub async fn local_list_plugins(
    state: Arc<LocalServerState>,
) -> Result<Vec<agendao_client::PluginListEntry>> {
    agendao_server::local_list_plugins(state).await
}
