use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type LocalServerState = Arc<super::super::local_server_bridge::CliLocalServerState>;

pub(super) async fn get_workspace_context(
    state: &LocalServerState,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    super::super::local_server_bridge::local_get_workspace_context(Arc::clone(state)).await
}

pub(super) async fn get_recent_models(
    state: &LocalServerState,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    super::super::local_server_bridge::local_get_recent_models(Arc::clone(state)).await
}

pub(super) async fn put_recent_models(
    state: &LocalServerState,
    recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    super::super::local_server_bridge::local_put_recent_models(Arc::clone(state), recent_models)
        .await
}
