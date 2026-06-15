#![allow(dead_code)]

use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;

use crate::routes::session::messages::ListMessagesQuery;
use crate::routes::session::prompt::SessionPromptRequest;
use crate::routes::session::session_crud::{
    CreateSessionRequest, ForkSessionRequest, ListSessionsQuery,
};
use crate::ServerState;

fn api_error<E: std::fmt::Display>(error: E) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}

pub async fn local_create_session(
    state: Arc<ServerState>,
    request: agendao_api::CreateSessionRequest,
) -> anyhow::Result<agendao_types::SessionInfo> {
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
) -> anyhow::Result<agendao_types::SessionInfo> {
    let Json(session) =
        super::session_crud::get_session(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(session)
}

pub async fn local_list_sessions(
    state: Arc<ServerState>,
    directory: Option<String>,
    search: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_api::SessionListItem>> {
    let Json(response) = super::session_crud::list_sessions(
        State(state),
        Query(ListSessionsQuery {
            directory,
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

pub async fn local_get_session_status(
    state: Arc<ServerState>,
) -> anyhow::Result<std::collections::HashMap<String, agendao_types::SessionStatusInfo>> {
    let Json(status) = super::session_crud::session_status(State(state))
        .await
        .map_err(api_error)?;
    Ok(status)
}

pub async fn local_delete_session(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<bool> {
    let Json(response) =
        super::session_crud::delete_session(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(response
        .get("deleted")
        .and_then(|value| value.as_bool())
        .unwrap_or(true))
}

pub async fn local_list_messages(
    state: Arc<ServerState>,
    session_id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_api::MessageInfo>> {
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

pub async fn local_list_questions(
    state: Arc<ServerState>,
) -> anyhow::Result<Vec<agendao_api::QuestionInfo>> {
    let Json(questions) = super::super::tui::list_questions(State(state)).await;
    serde_json::from_value(serde_json::to_value(questions)?)
        .context("failed to convert local questions payload")
}

pub async fn local_reply_question(
    state: Arc<ServerState>,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    let Json(_ok) = super::super::tui::reply_question(
        State(state),
        Path(question_id.to_string()),
        Json(super::super::tui::ReplyQuestionRequest { answers }),
    )
    .await
    .map_err(api_error)?;
    Ok(())
}

pub async fn local_reject_question(
    state: Arc<ServerState>,
    question_id: &str,
) -> anyhow::Result<()> {
    let Json(_ok) = super::super::tui::reject_question(State(state), Path(question_id.to_string()))
        .await
        .map_err(api_error)?;
    Ok(())
}

pub async fn local_list_permissions(
    _state: Arc<ServerState>,
) -> anyhow::Result<Vec<agendao_api::PermissionRequestInfo>> {
    let Json(permissions) = super::super::permission::list_permissions().await;
    serde_json::from_value(serde_json::to_value(permissions)?)
        .context("failed to convert local permissions payload")
}

pub async fn local_reply_permission(
    state: Arc<ServerState>,
    permission_id: &str,
    reply: String,
    message: Option<String>,
) -> anyhow::Result<()> {
    let Json(_ok) = super::super::permission::reply_permission(
        State(state),
        Path(permission_id.to_string()),
        Json(super::super::permission::ReplyPermissionRequest { reply, message }),
    )
    .await
    .map_err(api_error)?;
    Ok(())
}

pub async fn local_get_session_runtime(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<agendao_server_core::runtime_state::SessionRuntimeState> {
    let Json(runtime) =
        super::session_crud::get_session_runtime(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(runtime)
}

pub async fn local_get_session_telemetry(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<super::telemetry::SessionTelemetrySnapshot> {
    let Json(snapshot) =
        super::telemetry::get_session_telemetry(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(snapshot)
}

pub async fn local_get_session_todos(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_types::SessionTodoInfo>> {
    let Json(todos) =
        super::session_crud::get_session_todos(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(todos)
}

pub async fn local_get_session_diff(
    state: Arc<ServerState>,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_types::FileDiff>> {
    let Json(diffs) =
        super::session_crud::get_session_diff(State(state), Path(session_id.to_string()))
            .await
            .map_err(api_error)?;
    Ok(diffs)
}

pub async fn local_fork_session(
    state: Arc<ServerState>,
    session_id: &str,
    message_id: Option<String>,
) -> anyhow::Result<agendao_types::SessionInfo> {
    let Json(session) = super::session_crud::fork_session(
        State(state),
        Path(session_id.to_string()),
        Json(ForkSessionRequest {
            message_id,
            history_mode: None,
            history_message_limit: None,
        }),
    )
    .await
    .map_err(api_error)?;
    Ok(session)
}

pub async fn local_prompt(
    state: Arc<ServerState>,
    session_id: &str,
    request: agendao_api::PromptRequest,
) -> anyhow::Result<agendao_api::PromptResponse> {
    let parts = request.parts.map(|parts| {
        parts
            .into_iter()
            .map(|part| match part {
                agendao_api::PromptPart::Text { text } => {
                    agendao_session::prompt::PartInput::Text { text }
                }
                agendao_api::PromptPart::File {
                    url,
                    filename,
                    mime,
                } => agendao_session::prompt::PartInput::File {
                    url,
                    filename,
                    mime,
                },
                agendao_api::PromptPart::Agent { name } => {
                    agendao_session::prompt::PartInput::Agent { name }
                }
                agendao_api::PromptPart::Subtask {
                    prompt,
                    description,
                    agent,
                } => agendao_session::prompt::PartInput::Subtask {
                    prompt,
                    description,
                    agent,
                },
            })
            .collect()
    });

    let Json(value) = super::prompt::session_prompt(
        State(state.clone()),
        HeaderMap::new(),
        Path(session_id.to_string()),
        Json(SessionPromptRequest {
            message: request.message,
            parts,
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
    provider: Arc<dyn agendao_provider::Provider>,
) {
    state.providers.write().await.register_arc(provider);
}

pub async fn local_get_config(state: Arc<ServerState>) -> anyhow::Result<agendao_config::Config> {
    let Json(config) = super::super::config::get_config(State(state))
        .await
        .map_err(api_error)?;
    Ok(config)
}

pub async fn local_get_config_validation(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_types::ConfigPolicyValidationSnapshot> {
    let Json(snapshot) = super::super::config::get_config_validation(State(state))
        .await
        .map_err(api_error)?;
    Ok(snapshot)
}

pub async fn local_get_config_providers(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::ProviderListResponse> {
    let Json(response) = super::super::config::get_config_providers(State(state)).await;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local config providers payload")
}

pub async fn local_get_workspace_context(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_runtime_context::ResolvedWorkspaceContext> {
    let Json(context) = super::super::workspace::get_workspace_context(State(state))
        .await
        .map_err(api_error)?;
    Ok(context)
}

pub async fn local_get_recent_models(
    state: Arc<ServerState>,
) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
    let Json(payload) = super::super::workspace::get_workspace_recent_models(State(state))
        .await
        .map_err(api_error)?;
    Ok(payload.recent_models)
}

pub async fn local_put_recent_models(
    state: Arc<ServerState>,
    recent_models: Vec<agendao_state::RecentModelEntry>,
) -> anyhow::Result<Vec<agendao_state::RecentModelEntry>> {
    let Json(payload) = super::super::workspace::put_workspace_recent_models(
        State(state),
        Json(super::super::workspace::RecentModelsPayload { recent_models }),
    )
    .await
    .map_err(api_error)?;
    Ok(payload.recent_models)
}

pub async fn local_get_multimodal_policy(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::MultimodalPolicyResponse> {
    let Json(response) = super::super::multimodal::get_multimodal_policy(State(state))
        .await
        .map_err(api_error)?;
    Ok(response)
}

pub async fn local_get_multimodal_capabilities(
    state: Arc<ServerState>,
    model: Option<String>,
) -> anyhow::Result<agendao_api::MultimodalCapabilitiesResponse> {
    let Json(response) = super::super::multimodal::get_multimodal_capabilities(
        State(state),
        Query(super::super::multimodal::MultimodalCapabilitiesQuery { model }),
    )
    .await
    .map_err(api_error)?;
    Ok(response)
}

pub async fn local_preflight_multimodal(
    state: Arc<ServerState>,
    request: agendao_api::MultimodalPreflightRequest,
) -> anyhow::Result<agendao_api::MultimodalPreflightResponse> {
    let Json(response) =
        super::super::multimodal::post_multimodal_preflight(State(state), Json(request))
            .await
            .map_err(api_error)?;
    Ok(response)
}

pub async fn local_get_all_providers(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::FullProviderListResponse> {
    let Json(response) = super::super::provider::list_providers(State(state)).await;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local providers payload")
}

pub async fn local_get_known_providers(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::KnownProvidersResponse> {
    let Json(response) = super::super::provider::list_known_providers(State(state)).await;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local known providers payload")
}

pub async fn local_get_provider_connect_schema(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::ProviderConnectSchemaResponse> {
    let Json(response) = super::super::provider::get_provider_connect_schema(State(state)).await;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local provider connect schema payload")
}

pub async fn local_get_provider_descriptor(
    state: Arc<ServerState>,
    provider_id: &str,
) -> anyhow::Result<agendao_api::ProviderDescriptorResponse> {
    let Json(response) = super::super::provider::get_provider_descriptor(
        State(state),
        Path(provider_id.to_string()),
    )
    .await
    .map_err(api_error)?;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local provider descriptor payload")
}

pub async fn local_resolve_provider_connect(
    state: Arc<ServerState>,
    query: String,
) -> anyhow::Result<agendao_api::ResolveProviderConnectResponse> {
    let Json(response) = super::super::provider::resolve_provider_connect(
        State(state),
        Json(super::super::provider::ResolveProviderConnectRequest { query }),
    )
    .await;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local provider connect resolution payload")
}

pub async fn local_refresh_provider_catalog(
    state: Arc<ServerState>,
) -> anyhow::Result<agendao_api::RefreshProviderCatalogResponse> {
    let Json(response) = super::super::provider::refresh_provider_catalog(State(state))
        .await
        .map_err(api_error)?;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local provider catalog refresh payload")
}

pub async fn local_connect_provider(
    state: Arc<ServerState>,
    request: agendao_api::ConnectProviderRequest,
) -> anyhow::Result<()> {
    let Json(connected) = super::super::provider::connect_provider(
        State(state),
        Json(super::super::provider::ConnectProviderRequest {
            provider_id: request.provider_id,
            api_key: request.api_key,
            base_url: request.base_url,
            protocol: request.protocol,
        }),
    )
    .await
    .map_err(api_error)?;
    if connected {
        Ok(())
    } else {
        Err(anyhow::anyhow!("local provider connect returned false"))
    }
}

pub async fn local_list_agents(
    state: Arc<ServerState>,
) -> anyhow::Result<Vec<agendao_api::AgentInfo>> {
    let Json(response) = super::super::list_agents(State(state), HeaderMap::new())
        .await
        .map_err(api_error)?;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local agents payload")
}

pub async fn local_list_execution_modes(
    state: Arc<ServerState>,
) -> anyhow::Result<Vec<agendao_api::ExecutionModeInfo>> {
    let Json(response) = super::super::list_execution_modes(State(state), HeaderMap::new())
        .await
        .map_err(api_error)?;
    serde_json::from_value(serde_json::to_value(response)?)
        .context("failed to convert local execution modes payload")
}
