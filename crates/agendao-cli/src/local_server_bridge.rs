use std::path::PathBuf;
use std::sync::Arc;

use agendao_runtime_context::ResolvedWorkspaceContext;
use agendao_state::RecentModelEntry;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum CliDirectEvent {
    SessionBusy {
        session_id: String,
    },
    SessionIdle {
        session_id: String,
    },
    SessionUpdated {
        session_id: String,
    },
    OutputBlock {
        session_id: String,
        block: serde_json::Value,
    },
    QuestionCreated {
        session_id: String,
        request_id: String,
        questions_json: Option<serde_json::Value>,
    },
    QuestionResolved {
        request_id: String,
    },
    PermissionRequested {
        session_id: String,
        permission_id: String,
        info_json: Option<serde_json::Value>,
    },
    PermissionResolved {
        session_id: String,
        permission_id: String,
    },
    ToolCallStarted {
        session_id: String,
    },
    ToolCallCompleted {
        session_id: String,
    },
    ConfigUpdated,
    ControlInputTransition {
        session_id: String,
        phase: String,
    },
    TopologyChanged {
        session_id: String,
    },
    DiffUpdated {
        session_id: String,
    },
    SessionTreeChanged {
        session_id: String,
    },
}

#[cfg(feature = "local-server")]
pub(crate) type CliLocalServerState = agendao_server::ServerState;

#[cfg(not(feature = "local-server"))]
#[derive(Debug)]
pub(crate) enum CliLocalServerState {}

pub(crate) fn direct_mode_available() -> bool {
    cfg!(feature = "local-server")
}

#[cfg(feature = "local-server")]
pub(crate) async fn create_local_server_state(
    base_url: String,
    working_dir: PathBuf,
) -> anyhow::Result<Arc<CliLocalServerState>> {
    Ok(Arc::new(
        agendao_server::ServerState::new_with_storage_for_url_in_workspace(base_url, working_dir)
            .await?,
    ))
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn create_local_server_state(
    _base_url: String,
    _working_dir: PathBuf,
) -> anyhow::Result<Arc<CliLocalServerState>> {
    anyhow::bail!("direct mode requires the `local-server` CLI feature")
}

#[cfg(feature = "local-server")]
pub(crate) fn spawn_direct_event_loop(
    state: Arc<CliLocalServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<CliDirectEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut server_rx = agendao_server::spawn_direct_event_loop(state, session_id, cancel);
    tokio::spawn(async move {
        while let Some(event) = server_rx.recv().await {
            if tx.send(map_direct_event(event)).is_err() {
                break;
            }
        }
    });
    rx
}

#[cfg(not(feature = "local-server"))]
pub(crate) fn spawn_direct_event_loop(
    _state: Arc<CliLocalServerState>,
    _session_id: String,
    _cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<CliDirectEvent> {
    let (_tx, rx) = mpsc::unbounded_channel();
    rx
}

#[cfg(feature = "local-server")]
fn map_direct_event(event: agendao_server::DirectEvent) -> CliDirectEvent {
    use agendao_server::DirectEvent;

    match event {
        DirectEvent::SessionBusy { session_id } => CliDirectEvent::SessionBusy { session_id },
        DirectEvent::SessionIdle { session_id } => CliDirectEvent::SessionIdle { session_id },
        DirectEvent::SessionUpdated { session_id } => CliDirectEvent::SessionUpdated { session_id },
        DirectEvent::OutputBlock { session_id, block } => {
            CliDirectEvent::OutputBlock { session_id, block }
        }
        DirectEvent::QuestionCreated {
            session_id,
            request_id,
            questions_json,
        } => CliDirectEvent::QuestionCreated {
            session_id,
            request_id,
            questions_json,
        },
        DirectEvent::QuestionResolved { request_id } => {
            CliDirectEvent::QuestionResolved { request_id }
        }
        DirectEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        } => CliDirectEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        },
        DirectEvent::PermissionResolved {
            session_id,
            permission_id,
        } => CliDirectEvent::PermissionResolved {
            session_id,
            permission_id,
        },
        DirectEvent::ToolCallStarted { session_id } => {
            CliDirectEvent::ToolCallStarted { session_id }
        }
        DirectEvent::ToolCallCompleted { session_id } => {
            CliDirectEvent::ToolCallCompleted { session_id }
        }
        DirectEvent::ConfigUpdated => CliDirectEvent::ConfigUpdated,
        DirectEvent::ControlInputTransition { session_id, phase } => {
            CliDirectEvent::ControlInputTransition { session_id, phase }
        }
        DirectEvent::TopologyChanged { session_id } => {
            CliDirectEvent::TopologyChanged { session_id }
        }
        DirectEvent::DiffUpdated { session_id } => CliDirectEvent::DiffUpdated { session_id },
        DirectEvent::SessionTreeChanged { session_id } => {
            CliDirectEvent::SessionTreeChanged { session_id }
        }
    }
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_create_session(
    state: Arc<CliLocalServerState>,
    request: agendao_client::CreateSessionRequest,
) -> anyhow::Result<agendao_types::SessionInfo> {
    agendao_server::local_create_session(state, request).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_prompt(
    state: Arc<CliLocalServerState>,
    session_id: &str,
    request: agendao_client::PromptRequest,
) -> anyhow::Result<agendao_client::PromptResponse> {
    agendao_server::local_prompt(state, session_id, request).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_get_session(
    state: Arc<CliLocalServerState>,
    session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    agendao_server::local_get_session(state, session_id).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_list_messages(
    state: Arc<CliLocalServerState>,
    session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    agendao_server::local_list_messages(state, session_id, None, None).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_list_questions(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    agendao_server::local_list_questions(state).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_reply_question(
    state: Arc<CliLocalServerState>,
    question_id: &str,
    answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    agendao_server::local_reply_question(state, question_id, answers).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_reject_question(
    state: Arc<CliLocalServerState>,
    question_id: &str,
) -> anyhow::Result<()> {
    agendao_server::local_reject_question(state, question_id).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_reply_permission(
    state: Arc<CliLocalServerState>,
    permission_id: &str,
    reply: String,
    message: Option<String>,
) -> anyhow::Result<()> {
    agendao_server::local_reply_permission(state, permission_id, reply, message).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_get_workspace_context(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    agendao_server::local_get_workspace_context(state).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_list_sessions(
    state: Arc<CliLocalServerState>,
    search: Option<String>,
    limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    agendao_server::local_list_sessions(state, search, limit).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_fork_session(
    state: Arc<CliLocalServerState>,
    session_id: &str,
) -> anyhow::Result<agendao_types::SessionInfo> {
    agendao_server::local_fork_session(state, session_id, None).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_get_all_providers(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    agendao_server::local_get_all_providers(state).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_get_recent_models(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    agendao_server::local_get_recent_models(state).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_put_recent_models(
    state: Arc<CliLocalServerState>,
    recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    agendao_server::local_put_recent_models(state, recent_models).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_list_execution_modes(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    agendao_server::local_list_execution_modes(state).await
}

#[cfg(feature = "local-server")]
pub(crate) async fn local_list_agents(
    state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    agendao_server::local_list_agents(state).await
}

#[cfg(not(feature = "local-server"))]
macro_rules! local_server_unavailable {
    () => {
        anyhow::bail!("direct mode requires the `local-server` CLI feature")
    };
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_create_session(
    _state: Arc<CliLocalServerState>,
    _request: agendao_client::CreateSessionRequest,
) -> anyhow::Result<agendao_types::SessionInfo> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_prompt(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
    _request: agendao_client::PromptRequest,
) -> anyhow::Result<agendao_client::PromptResponse> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_get_session(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
) -> anyhow::Result<agendao_client::SessionInfo> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_list_messages(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
) -> anyhow::Result<Vec<agendao_client::MessageInfo>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_list_questions(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::QuestionInfo>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_reply_question(
    _state: Arc<CliLocalServerState>,
    _question_id: &str,
    _answers: Vec<Vec<String>>,
) -> anyhow::Result<()> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_reject_question(
    _state: Arc<CliLocalServerState>,
    _question_id: &str,
) -> anyhow::Result<()> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_reply_permission(
    _state: Arc<CliLocalServerState>,
    _permission_id: &str,
    _reply: String,
    _message: Option<String>,
) -> anyhow::Result<()> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_get_workspace_context(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<ResolvedWorkspaceContext> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_list_sessions(
    _state: Arc<CliLocalServerState>,
    _search: Option<String>,
    _limit: Option<usize>,
) -> anyhow::Result<Vec<agendao_client::SessionListItem>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_fork_session(
    _state: Arc<CliLocalServerState>,
    _session_id: &str,
) -> anyhow::Result<agendao_types::SessionInfo> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_get_all_providers(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<agendao_client::FullProviderListResponse> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_get_recent_models(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_put_recent_models(
    _state: Arc<CliLocalServerState>,
    _recent_models: Vec<RecentModelEntry>,
) -> anyhow::Result<Vec<RecentModelEntry>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_list_execution_modes(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::ExecutionModeInfo>> {
    local_server_unavailable!()
}

#[cfg(not(feature = "local-server"))]
pub(crate) async fn local_list_agents(
    _state: Arc<CliLocalServerState>,
) -> anyhow::Result<Vec<agendao_client::AgentInfo>> {
    local_server_unavailable!()
}
