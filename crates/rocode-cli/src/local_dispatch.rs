// Direct-mode dispatch helpers — mirror the TUI RuntimeApiClient pattern.
// When `local_server` is Some, use rocode_server::local_* functions;
// otherwise delegate to the AsyncApiClient over HTTP.

use std::sync::Arc;
use crate::api_client::CliApiClient;
use rocode_client::CreateSessionRequest;

/// Create a session via local or HTTP path.
#[allow(dead_code)] // will be used when threading local_server through call sites
pub async fn create_session(
    local_server: &Option<Arc<rocode_server::ServerState>>,
    api_client: &CliApiClient,
    scheduler_profile: Option<String>,
    directory: Option<String>,
) -> anyhow::Result<rocode_types::SessionInfo> {
    if let Some(state) = local_server {
        rocode_server::local_create_session(
            Arc::clone(state),
            CreateSessionRequest {
                scheduler_profile,
                directory,
                project_id: None,
                title: None,
            },
        )
        .await
    } else {
        api_client.create_session(scheduler_profile, directory).await
    }
}

/// Send a prompt via local or HTTP path.
#[allow(dead_code)]
pub async fn send_prompt(
    local_server: &Option<Arc<rocode_server::ServerState>>,
    api_client: &CliApiClient,
    session_id: &str,
    content: String,
    parts: Option<Vec<rocode_client::PromptPart>>,
    agent: Option<String>,
    scheduler_profile: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    ingress_source: Option<String>,
    idempotency_key: Option<String>,
    source_origin: Option<rocode_types::MessageSourceOrigin>,
    source_surface: Option<rocode_types::MessageSourceSurface>,
) -> anyhow::Result<rocode_client::PromptResponse> {
    if let Some(state) = local_server {
        rocode_server::local_prompt(
            Arc::clone(state),
            session_id,
            rocode_client::PromptRequest {
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
    } else {
        api_client
            .send_prompt(
                session_id, content, parts, agent, scheduler_profile,
                model, variant, ingress_source, idempotency_key,
                source_origin, source_surface,
            )
            .await
    }
}
