mod direct_dispatch;
mod transport_dispatch;

use crate::api_client::CliApiClient;
use agendao_client::FrontendTransport;
use agendao_client::PromptPart;
use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use std::sync::Arc;

type LocalServer = Option<Arc<crate::local_server_bridge::CliLocalServerState>>;
type Transport = Option<Arc<FrontendTransport>>;

pub async fn send_prompt(
    local_server: &LocalServer,
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
    if let Some(state) = local_server {
        direct_dispatch::send_prompt(
            state,
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
    } else {
        transport_dispatch::send_prompt(
            transport,
            api_client,
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

pub async fn send_command_prompt(
    local_server: &LocalServer,
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
        direct_dispatch::send_command_prompt(
            state,
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
    } else {
        transport_dispatch::send_command_prompt(
            api_client,
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
}

pub async fn get_session(
    local_server: &LocalServer,
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    if let Some(state) = local_server {
        direct_dispatch::get_session(state, session_id).await
    } else {
        transport_dispatch::get_session(api_client, session_id).await
    }
}

pub async fn list_messages(
    local_server: &LocalServer,
    api_client: &CliApiClient,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    if let Some(state) = local_server {
        direct_dispatch::list_messages(state, session_id).await
    } else {
        transport_dispatch::list_messages(api_client, session_id).await
    }
}

pub async fn list_questions(
    local_server: &LocalServer,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    if let Some(state) = local_server {
        direct_dispatch::list_questions(state).await
    } else {
        transport_dispatch::list_questions(api_client).await
    }
}

pub async fn reply_question(
    local_server: &LocalServer,
    api_client: &CliApiClient,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        direct_dispatch::reply_question(state, question_id, answers).await
    } else {
        transport_dispatch::reply_question(api_client, question_id, answers).await
    }
}

pub async fn reject_question(
    local_server: &LocalServer,
    api_client: &CliApiClient,
    question_id: &str,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        direct_dispatch::reject_question(state, question_id).await
    } else {
        transport_dispatch::reject_question(api_client, question_id).await
    }
}

pub async fn reply_permission(
    local_server: &LocalServer,
    api_client: &CliApiClient,
    permission_id: &str,
    reply: &str,
    message: Option<String>,
) -> anyhow::Result<()> {
    if let Some(state) = local_server {
        direct_dispatch::reply_permission(state, permission_id, reply.to_string(), message).await
    } else {
        transport_dispatch::reply_permission(api_client, permission_id, reply, message).await
    }
}

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

pub async fn list_sessions(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
    search: Option<&str>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    if let Some(state) = local_server {
        direct_dispatch::list_sessions(state, search.map(str::to_string), limit).await
    } else {
        transport_dispatch::list_sessions(transport, api_client, search, limit).await
    }
}

pub async fn get_all_providers(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    if let Some(state) = local_server {
        direct_dispatch::get_all_providers(state).await
    } else {
        transport_dispatch::get_all_providers(transport, api_client).await
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

pub async fn list_execution_modes(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    if let Some(state) = local_server {
        direct_dispatch::list_execution_modes(state).await
    } else {
        transport_dispatch::list_execution_modes(transport, api_client).await
    }
}

pub async fn list_agents(
    local_server: &LocalServer,
    transport: &Transport,
    api_client: &CliApiClient,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    if let Some(state) = local_server {
        direct_dispatch::list_agents(state).await
    } else {
        transport_dispatch::list_agents(transport, api_client).await
    }
}
