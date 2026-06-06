mod direct_dispatch;
mod transport_dispatch;

use crate::api_client::CliApiClient;
use agendao_client::FrontendTransport;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type LocalServer = Option<Arc<super::local_server_bridge::CliLocalServerState>>;
type Transport = Option<Arc<FrontendTransport>>;

pub async fn get_workspace_context(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    if let Some(state) = local_server {
        direct_dispatch::get_workspace_context(state).await
    } else {
        transport_dispatch::get_workspace_context(transport, api_client).await
    }
}

pub async fn get_recent_models(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(state) = local_server {
        direct_dispatch::get_recent_models(state).await
    } else {
        transport_dispatch::get_recent_models(transport, api_client).await
    }
}

pub async fn put_recent_models(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
    recent_models: &[RecentModelEntry],
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(state) = local_server {
        direct_dispatch::put_recent_models(state, recent_models.to_vec()).await
    } else {
        transport_dispatch::put_recent_models(transport, api_client, recent_models).await
    }
}
