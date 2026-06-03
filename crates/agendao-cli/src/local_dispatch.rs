// Direct-mode dispatch helpers — mirror the TUI RuntimeApiClient pattern.
// When `local_server` is Some, use agendao_server::local_* functions;
// otherwise delegate to the AsyncApiClient over HTTP.

use std::sync::Arc;
use crate::api_client::CliApiClient;
use agendao_client::CreateSessionRequest;
use agendao_client::FrontendTransport;
use agendao_client::PromptPart;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;

/// Create a session via local or HTTP path.
#[allow(dead_code)] // will be used when threading local_server through call sites
pub async fn create_session(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    scheduler_profile: Option<String>,
    directory: Option<String>,
) -> anyhow::Result<agendao_types::SessionInfo> {
    if let Some(state) = local_server {
        agendao_server::local_create_session(
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
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
    session_id: &str,
    content: String,
    parts: Option<Vec<PromptPart>>,
    agent: Option<String>,
    scheduler_profile: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    ingress_source: Option<String>,
    idempotency_key: Option<String>,
    source_origin: Option<agendao_types::MessageSourceOrigin>,
    source_surface: Option<agendao_types::MessageSourceSurface>,
) -> anyhow::Result<agendao_client::PromptResponse> {
    if let Some(state) = local_server {
        agendao_server::local_prompt(
            Arc::clone(state),
            session_id,
            agendao_client::PromptRequest {
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
    } else if let Some(transport) = transport {
        let response = transport
            .prompt(
                session_id,
                &content,
                agendao_client::transport::PromptOptions {
                    agent_id: agent,
                    scheduler_profile,
                    model,
                    variant,
                    continue_last: false,
                    source_origin,
                    source_surface,
                    ingress_source,
                    idempotency_key,
                },
            )
            .await?;
        Ok(agendao_client::PromptResponse {
            status: "accepted".to_string(),
            ok: Some(true),
            session_id: Some(response.session_id),
            queued_count: None,
            pending_question_id: None,
            command: None,
            missing_fields: Vec::new(),
        })
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

pub async fn send_command_prompt(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    session_id: &str,
    command: String,
    arguments: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    ingress_source: Option<String>,
    idempotency_key: Option<String>,
    source_origin: Option<agendao_types::MessageSourceOrigin>,
    source_surface: Option<agendao_types::MessageSourceSurface>,
) -> anyhow::Result<agendao_client::PromptResponse> {
    if let Some(state) = local_server {
        return agendao_server::local_prompt(
            Arc::clone(state),
            session_id,
            agendao_client::PromptRequest {
                message: None,
                parts: None,
                idempotency_key,
                ingress_source,
                agent: None,
                scheduler_profile: None,
                model,
                variant,
                command: Some(command),
                arguments,
                source_origin,
                source_surface,
            },
        )
        .await;
    }
    api_client
        .send_command_prompt(
            session_id,
            command,
            arguments,
            model,
            variant,
            ingress_source,
            idempotency_key,
            source_origin,
            source_surface,
        )
        .await
}

pub async fn get_session(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    if let Some(state) = local_server {
        return agendao_server::local_get_session(Arc::clone(state), session_id).await;
    }
    api_client.get_session(session_id).await
}

pub async fn list_messages(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    if let Some(state) = local_server {
        return agendao_server::local_list_messages(Arc::clone(state), session_id, None, None).await;
    }
    api_client.get_messages(session_id).await
}

pub async fn list_questions(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    if let Some(state) = local_server {
        return agendao_server::local_list_questions(Arc::clone(state)).await;
    }
    api_client.list_questions().await
}

pub async fn reply_question(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        return agendao_server::local_reply_question(Arc::clone(state), question_id, answers).await;
    }
    api_client.reply_question(question_id, answers).await
}

pub async fn reject_question(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    question_id: &str,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        return agendao_server::local_reject_question(Arc::clone(state), question_id).await;
    }
    api_client.reject_question(question_id).await
}

pub async fn reply_permission(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    api_client: &CliApiClient,
    permission_id: &str,
    reply: &str,
    message: Option<String>,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        return agendao_server::local_reply_permission(
            Arc::clone(state),
            permission_id,
            reply.to_string(),
            message,
        )
        .await;
    }
    api_client.reply_permission(permission_id, reply, message).await
}

pub async fn get_workspace_context(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    if let Some(state) = local_server {
        return agendao_server::local_get_workspace_context(Arc::clone(state)).await;
    }
    if let Some(transport) = transport {
        if let Ok(context) = transport.get_workspace_context().await {
            return Ok(context);
        }
    }
    api_client.get_workspace_context().await
}

pub async fn list_sessions(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
    search: Option<&str>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    if let Some(state) = local_server {
        return agendao_server::local_list_sessions(
            Arc::clone(state),
            search.map(str::to_string),
            limit,
        )
        .await;
    }
    if search.is_none() {
        if let Some(transport) = transport {
            if let Ok(sessions) = transport.list_sessions().await {
                return Ok(sessions);
            }
        }
    }
    api_client.list_sessions(search, limit).await
}

pub async fn get_all_providers(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    if let Some(state) = local_server {
        return agendao_server::local_get_all_providers(Arc::clone(state)).await;
    }
    if let Some(transport) = transport {
        if let Ok(response) = transport.get_all_providers().await {
            return Ok(response);
        }
    }
    api_client.get_all_providers().await
}

pub async fn get_recent_models(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(state) = local_server {
        return agendao_server::local_get_recent_models(Arc::clone(state)).await;
    }
    if let Some(transport) = transport {
        if let Ok(response) = transport.get_recent_models().await {
            return Ok(response);
        }
    }
    api_client.get_recent_models().await
}

pub async fn put_recent_models(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
    recent_models: &[RecentModelEntry],
) -> anyhow::Result<Vec<RecentModelEntry>> {
    if let Some(state) = local_server {
        return agendao_server::local_put_recent_models(
            Arc::clone(state),
            recent_models.to_vec(),
        )
        .await;
    }
    if let Some(transport) = transport {
        if let Ok(response) = transport.put_recent_models(recent_models).await {
            return Ok(response);
        }
    }
    api_client.put_recent_models(recent_models).await
}

pub async fn list_execution_modes(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    if let Some(state) = local_server {
        return agendao_server::local_list_execution_modes(Arc::clone(state)).await;
    }
    if let Some(transport) = transport {
        if let Ok(response) = transport.list_execution_modes().await {
            return Ok(response);
        }
    }
    api_client.list_execution_modes().await
}

pub async fn list_agents(
    local_server: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<FrontendTransport>>,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    if let Some(state) = local_server {
        return agendao_server::local_list_agents(Arc::clone(state)).await;
    }
    if let Some(transport) = transport {
        if let Ok(response) = transport.list_agents().await {
            return Ok(response);
        }
    }
    api_client.list_agents().await
}
