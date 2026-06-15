use std::path::PathBuf;
use std::sync::Arc;

use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;

#[cfg(feature = "local-server")]
pub(super) type CliLocalServerState = agendao_server::ServerState;

#[cfg(not(feature = "local-server"))]
#[derive(Debug)]
pub(super) enum CliLocalServerState {}

pub(super) fn direct_mode_available() -> bool {
    cfg!(feature = "local-server")
}

#[cfg(feature = "local-server")]
pub(super) async fn create_local_server_state(
    base_url: String,
    working_dir: PathBuf,
) -> anyhow::Result<Arc<CliLocalServerState>> {
    let state = Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(base_url, working_dir)
            .await?,
    );
    state.ensure_frontend_projector();
    Ok(state)
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn create_local_server_state(
    _base_url: String,
    _working_dir: PathBuf,
) -> anyhow::Result<Arc<CliLocalServerState>> {
    anyhow::bail!("direct mode requires the `local-server` CLI feature")
}

#[cfg(feature = "local-server")]
pub(super) async fn local_create_session(
    state: Arc<CliLocalServerState>,
    request: agendao_client::CreateSessionRequest,
) -> anyhow::Result<agendao_types::SessionInfo> {
    agendao_server::local_create_session(state, request).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_prompt(
    state: Arc<CliLocalServerState>,
    session_id: &str,
    request: agendao_client::PromptRequest,
) -> anyhow::Result<agendao_client::PromptResponse> {
    agendao_server::local_prompt(state, session_id, request).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_list_messages(
    state: Arc<CliLocalServerState>,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    agendao_server::local_list_messages(state, session_id, None, None).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_get_workspace_context(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    agendao_server::local_get_workspace_context(state).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_list_sessions(
    state: Arc<CliLocalServerState>,
    search: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    agendao_server::local_list_sessions(state, None, search, limit).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_fork_session(
    state: Arc<CliLocalServerState>,
    session_id: &str,
) -> anyhow::Result<agendao_types::SessionInfo> {
    agendao_server::local_fork_session(state, session_id, None).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_get_recent_models(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    agendao_server::local_get_recent_models(state).await
}

#[cfg(feature = "local-server")]
pub(super) async fn local_put_recent_models(
    state: Arc<CliLocalServerState>,
    recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    agendao_server::local_put_recent_models(state, recent_models).await
}

#[cfg(not(feature = "local-server"))]
macro_rules! local_server_unavailable {
    () => {
        anyhow::bail!("direct mode requires the `local-server` CLI feature")
    };
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_create_session(
    _state: Arc<CliLocalServerState>,
    _request: agendao_client::CreateSessionRequest,
) -> anyhow::Result<agendao_types::SessionInfo> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_prompt(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
    _request: agendao_client::PromptRequest,
) -> anyhow::Result<agendao_client::PromptResponse> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_list_messages(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_get_workspace_context(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_list_sessions(
    _state: Arc<CliLocalServerState>,
    _search: Option<String>,
    _limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_fork_session(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
) -> anyhow::Result<agendao_types::SessionInfo> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_get_recent_models(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(super) async fn local_put_recent_models(
    _state: Arc<CliLocalServerState>,
    _recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    local_server_unavailable!()
}
