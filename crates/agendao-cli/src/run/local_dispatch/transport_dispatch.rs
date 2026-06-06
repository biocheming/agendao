use crate::api_client::CliApiClient;
use agendao_client::FrontendTransport;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type Transport = Option<Arc<FrontendTransport>>;

pub(super) async fn get_workspace_context(
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    if let Some(transport) = transport {
        if let Ok(context) = transport.get_workspace_context().await {
            return Ok(context);
        }
    }
    api_client.get_workspace_context().await
}

pub(super) async fn get_recent_models(
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(transport) = transport {
        if let Ok(response) = transport.get_recent_models().await {
            return Ok(response);
        }
    }
    api_client.get_recent_models().await
}

pub(super) async fn put_recent_models(
    transport: &Transport,
    api_client: &CliApiClient,
    recent_models: &[RecentModelEntry],
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(transport) = transport {
        if let Ok(response) = transport.put_recent_models(recent_models).await {
            return Ok(response);
        }
    }
    api_client.put_recent_models(recent_models).await
}
