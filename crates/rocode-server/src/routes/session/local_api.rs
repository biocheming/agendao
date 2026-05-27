use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;

use crate::routes::session::messages::ListMessagesQuery;
use crate::routes::session::prompt::SessionPromptRequest;
use crate::routes::session::session_crud::{CreateSessionRequest, ListSessionsQuery};
use crate::ServerState;

fn api_error<E: std::fmt::Display>(error: E) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}

pub async fn local_create_session(
    state: Arc<ServerState>,
    request: rocode_api::CreateSessionRequest,
) -> anyhow::Result<rocode_types::SessionInfo> {
    let Json(session) = super::session_crud::create_session(
        State(state),
        Json(CreateSessionRequest {
            parent_id: None,
            scheduler_profile: request.scheduler_profile,
            directory: request.directory,
            project_id: request.project_id,
            title: request.title,
        }),
    )
    .await
    .map_err(api_error)?;
    Ok(session)
}

pub async fn local_get_session(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<rocode_types::SessionInfo> {
    let Json(session) = super::session_crud::get_session(
        State(state),
        Path(session_id.to_string()),
    )
    .await
    .map_err(api_error)?;
    Ok(session)
}

pub async fn local_list_sessions(
    state: Arc<ServerState>,
    search: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<rocode_api::SessionListItem>> {
    let Json(response) = super::session_crud::list_sessions(
        State(state),
        Query(ListSessionsQuery {
            directory: None,
            roots: None,
            start: None,
            search,
            limit,
        }),
    )
    .await
    .map_err(api_error)?;
    Ok(response.items)
}

pub async fn local_list_messages(
    state: Arc<ServerState>,
    session_id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<rocode_api::MessageInfo>> {
    let Json(messages) = super::messages::list_messages(
        State(state),
        Path(session_id.to_string()),
        Query(ListMessagesQuery { after, limit }),
    )
    .await
    .map_err(api_error)?;

    messages
        .into_iter()
        .map(|message| {
            serde_json::from_value(serde_json::to_value(message)?)
                .context("failed to convert local message payload")
        })
        .collect()
}

pub async fn local_prompt(
    state: Arc<ServerState>,
    session_id: &str,
    request: rocode_api::PromptRequest,
) -> anyhow::Result<rocode_api::PromptResponse> {
    let Json(value) = super::prompt::session_prompt(
        State(state.clone()),
        HeaderMap::new(),
        Path(session_id.to_string()),
        Json(SessionPromptRequest {
            message: request.message,
            parts: request.parts,
            idempotency_key: request.idempotency_key,
            ingress_source: request.ingress_source,
            source_origin: request.source_origin,
            source_surface: request.source_surface,
            model: request.model,
            variant: request.variant,
            agent: request.agent,
            scheduler_profile: request.scheduler_profile,
            command: request.command,
            arguments: request.arguments,
            recovery: None,
        }),
    )
    .await
    .map_err(api_error)?;

    // source_origin/source_surface are threaded through SessionPromptRequest
    // → task_ingress_for_prompt → IngressTurnEnvelope. The session runtime
    // consumes them when constructing PromptExecutionOptions for the
    // orchestrator (see prompt_execution.rs).

    serde_json::from_value(value).context("failed to convert local prompt response")
}

pub async fn local_register_provider(
    state: &Arc<ServerState>,
    provider: Arc<dyn rocode_provider::Provider>,
) {
    state.providers.write().await.register_arc(provider);
}
