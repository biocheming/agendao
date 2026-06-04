use crate::api_client::CliApiClient;
use agendao_client::FrontendTransport;
use agendao_client::PromptPart;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type Transport = Option<Arc<FrontendTransport>>;

pub(super) async fn send_prompt(
    transport: &Transport,
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
    if let Some(transport) = transport {
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
                session_id,
                content,
                parts,
                agent,
                scheduler_profile,
                model,
                variant,
                ingress_source,
                idempotency_key,
                source_origin,
                source_surface,
            )
            .await
    }
}

pub(super) async fn send_command_prompt(
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

pub(super) async fn get_session(
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    api_client.get_session(session_id).await
}

pub(super) async fn list_messages(
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    api_client.get_messages(session_id).await
}

pub(super) async fn list_questions(
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    api_client.list_questions().await
}

pub(super) async fn reply_question(
    api_client: &CliApiClient,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    api_client.reply_question(question_id, answers).await
}

pub(super) async fn reject_question(
    api_client: &CliApiClient,
    question_id: &str,
) -> anyhow::Result<()> {
    api_client.reject_question(question_id).await
}

pub(super) async fn reply_permission(
    api_client: &CliApiClient,
    permission_id: &str,
    reply: &str,
    message: Option<String>,
) -> anyhow::Result<()> {
    api_client
        .reply_permission(permission_id, reply, message)
        .await
}

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

pub(super) async fn list_sessions(
    transport: &Transport,
    api_client: &CliApiClient,
    search: Option<&str>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    if search.is_none() {
        if let Some(transport) = transport {
            if let Ok(sessions) = transport.list_sessions().await {
                return Ok(sessions);
            }
        }
    }
    api_client.list_sessions(search, limit).await
}

pub(super) async fn get_all_providers(
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    if let Some(transport) = transport {
        if let Ok(response) = transport.get_all_providers().await {
            return Ok(response);
        }
    }
    api_client.get_all_providers().await
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

pub(super) async fn list_execution_modes(
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    if let Some(transport) = transport {
        if let Ok(response) = transport.list_execution_modes().await {
            return Ok(response);
        }
    }
    api_client.list_execution_modes().await
}

pub(super) async fn list_agents(
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    if let Some(transport) = transport {
        if let Ok(response) = transport.list_agents().await {
            return Ok(response);
        }
    }
    api_client.list_agents().await
}
