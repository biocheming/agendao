use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use rocode_api::CompactResponse;
use rocode_session::{load_session_telemetry_snapshot, SessionForkError, SessionForkSpec};
use rocode_types::{
    FileDiff, PermissionRulesetInfo, SessionForkHistoryMode, SessionInfo, SessionListContract,
    SessionListHints, SessionListItem, SessionListResponse, SessionListSummary, SessionRevertInfo,
    SessionShareInfo, SessionStatusInfo, SessionSummaryInfo, SessionTimeInfo, SessionTodoInfo,
};
use serde::Deserialize;

use crate::runtime_control::SessionRunStatus;
use crate::session_runtime::events::broadcast_session_updated;
use crate::{ApiError, Result, ServerState};

use super::scheduler::resolve_scheduler_request_defaults_validated;

// ─── Request / Response structs ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub directory: Option<String>,
    pub roots: Option<bool>,
    pub start: Option<i64>,
    pub search: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub parent_id: Option<String>,
    pub scheduler_profile: Option<String>,
    pub directory: Option<String>,
    pub project_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CreateSessionSpec {
    pub scheduler_profile: Option<String>,
    pub directory: Option<String>,
    pub project_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionRulesetInput {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionTimeRequest {
    pub archived: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub time: Option<UpdateSessionTimeRequest>,
}

#[derive(Debug, Deserialize)]
pub struct ForkSessionRequest {
    pub message_id: Option<String>,
    #[serde(default)]
    pub history_mode: Option<SessionForkHistoryMode>,
    #[serde(default)]
    pub history_message_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ArchiveSessionRequest {
    pub archive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SetTitleRequest {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct SetSummaryRequest {
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub files: Option<u64>,
    pub diffs: Option<Vec<SetSummaryFileDiff>>,
}

#[derive(Debug, Deserialize)]
pub struct SetSummaryFileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Deserialize)]
pub struct RevertRequest {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePartRequest {
    pub part: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteShellRequest {
    pub command: String,
    pub workdir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandRequest {
    pub command: String,
    pub arguments: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct CompactRequest {
    #[serde(default)]
    focus: Option<String>,
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn validate_session_revert_target(
    session: &rocode_session::Session,
    message_id: &str,
    part_id: Option<&str>,
) -> Result<()> {
    let Some(message) = session.get_message(message_id) else {
        return Err(ApiError::NotFound(format!(
            "Message not found: {}",
            message_id
        )));
    };

    if rocode_session::Session::is_imported_fork_history_message(message) {
        return Err(ApiError::BadRequest(
            "Revert only applies to fork-local history; imported fork history is read-only."
                .to_string(),
        ));
    }

    if let Some(part_id) = part_id {
        if !message.parts.iter().any(|part| part.id == part_id) {
            return Err(ApiError::NotFound(format!("Part not found: {}", part_id)));
        }
    }

    Ok(())
}

fn session_time_info(session: &rocode_session::Session) -> SessionTimeInfo {
    let session = session.record();
    rocode_types::SessionTime {
        created: session.time.created,
        updated: session.time.updated,
        compacting: session.time.compacting,
        archived: session.time.archived,
    }
}

fn session_summary_info(session: &rocode_session::Session) -> Option<SessionSummaryInfo> {
    let session = session.record();
    session.summary.as_ref().map(|s| SessionListSummary {
        additions: s.additions,
        deletions: s.deletions,
        files: s.files,
    })
}

fn session_list_time(session: &rocode_session::Session) -> rocode_types::SessionTime {
    let session = session.record();
    rocode_types::SessionTime {
        created: session.time.created,
        updated: session.time.updated,
        compacting: session.time.compacting,
        archived: session.time.archived,
    }
}

fn session_list_summary(session: &rocode_session::Session) -> Option<SessionListSummary> {
    let session = session.record();
    session.summary.as_ref().map(|s| SessionListSummary {
        additions: s.additions,
        deletions: s.deletions,
        files: s.files,
    })
}

fn session_list_hints(session: &rocode_session::Session) -> Option<SessionListHints> {
    let session = session.record();
    let scheduler_profile = session
        .metadata
        .get("scheduler_profile")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let hints = SessionListHints {
        current_model: session
            .metadata
            .get("current_model")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        model_provider: session
            .metadata
            .get("model_provider")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        model_id: session
            .metadata
            .get("model_id")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        scheduler_profile: scheduler_profile.clone(),
        agent: session
            .metadata
            .get("agent")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
    };

    if hints.current_model.is_none()
        && hints.model_provider.is_none()
        && hints.model_id.is_none()
        && hints.scheduler_profile.is_none()
        && hints.agent.is_none()
    {
        None
    } else {
        Some(hints)
    }
}

fn session_pending_command_invocation(
    session: &rocode_session::Session,
) -> Option<serde_json::Value> {
    session
        .record()
        .metadata
        .get("pending_command_invocation")
        .cloned()
}

fn session_list_contract() -> SessionListContract {
    SessionListContract {
        filter_query_parameters: vec![
            "directory".to_string(),
            "roots".to_string(),
            "start".to_string(),
            "search".to_string(),
            "limit".to_string(),
        ],
        search_fields: rocode_session::SESSION_LIST_SEARCH_FIELDS
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        non_search_fields: vec![
            "hints".to_string(),
            "pending_command_invocation".to_string(),
        ],
        note: "Server-side session list search is restricted to lightweight SessionListItem fields. Display-only fields such as hints never participate.".to_string(),
    }
}

fn session_list_response(items: Vec<SessionListItem>) -> SessionListResponse {
    SessionListResponse {
        items,
        contract: session_list_contract(),
    }
}

pub(super) fn session_to_list_item(session: &rocode_session::Session) -> SessionListItem {
    let session_record = session.record();
    SessionListItem {
        id: session_record.id.clone(),
        slug: session_record.slug.clone(),
        project_id: session_record.project_id.clone(),
        directory: session_record.directory.clone(),
        parent_id: session_record.parent_id.clone(),
        title: session_record.title.clone(),
        version: session_record.version.clone(),
        time: session_list_time(session),
        summary: session_list_summary(session),
        hints: session_list_hints(session),
        pending_command_invocation: session_pending_command_invocation(session),
    }
}

pub(crate) fn session_to_info(session: &rocode_session::Session) -> SessionInfo {
    let session_record = session.record();
    SessionInfo {
        id: session_record.id.clone(),
        slug: session_record.slug.clone(),
        project_id: session_record.project_id.clone(),
        directory: session_record.directory.clone(),
        parent_id: session_record.parent_id.clone(),
        title: session_record.title.clone(),
        version: session_record.version.clone(),
        time: session_time_info(session),
        summary: session_summary_info(session),
        share: session_record
            .share
            .as_ref()
            .map(|s| rocode_types::SessionShare { url: s.url.clone() }),
        revert: session_record.revert.as_ref().map(|r| SessionRevertInfo {
            message_id: r.message_id.clone(),
            part_id: r.part_id.clone(),
            snapshot: r.snapshot.clone(),
            diff: r.diff.clone(),
        }),
        permission: session_record
            .permission
            .as_ref()
            .map(|p| PermissionRulesetInfo {
                allow: p.allow.clone(),
                deny: p.deny.clone(),
                mode: p.mode.clone(),
            }),
        fork: session.fork_explain(),
        telemetry: load_session_telemetry_snapshot(session),
        metadata: if session_record.metadata.is_empty() {
            None
        } else {
            Some(session_record.metadata.clone())
        },
    }
}

fn collect_session_tree_ids(
    sessions: &rocode_session::SessionManager,
    root_id: &str,
) -> Option<Vec<String>> {
    if sessions.get(root_id).is_none() {
        return None;
    }

    fn visit(sessions: &rocode_session::SessionManager, session_id: &str, out: &mut Vec<String>) {
        out.push(session_id.to_string());
        let child_ids: Vec<String> = sessions
            .attached_sessions(session_id)
            .into_iter()
            .map(|session| session.id.clone())
            .collect();
        for attached_id in child_ids {
            visit(sessions, &attached_id, out);
        }
    }

    let mut ids = Vec::new();
    visit(sessions, root_id, &mut ids);
    Some(ids)
}

pub(super) async fn persist_sessions_if_enabled(state: &Arc<ServerState>) {
    if let Err(err) = state.sync_sessions_to_storage().await {
        tracing::error!("failed to sync sessions to storage: {}", err);
    }
}

pub(crate) fn resolved_session_directory(raw: &str, workspace_root: &FsPath) -> String {
    let trimmed = raw.trim();
    let candidate = if trimmed.is_empty() || trimmed == "." {
        workspace_root.to_path_buf()
    } else {
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            workspace_root.join(path)
        }
    };
    candidate
        .canonicalize()
        .unwrap_or(candidate)
        .to_string_lossy()
        .to_string()
}

pub(super) fn session_model_override(session: &rocode_session::Session) -> Option<String> {
    session
        .record()
        .metadata
        .get("model_provider")
        .and_then(|value| value.as_str())
        .zip(
            session
                .metadata
                .get("model_id")
                .and_then(|value| value.as_str()),
        )
        .map(|(provider, model)| format!("{provider}/{model}"))
}

pub(super) fn session_variant_override(session: &rocode_session::Session) -> Option<String> {
    session
        .record()
        .metadata
        .get("model_variant")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

pub(super) fn session_agent_override(session: &rocode_session::Session) -> Option<String> {
    session
        .record()
        .metadata
        .get("agent")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

pub(super) fn session_scheduler_profile_override(
    session: &rocode_session::Session,
) -> Option<String> {
    session
        .record()
        .metadata
        .get("scheduler_profile")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

pub(super) async fn set_session_run_status(
    state: &Arc<ServerState>,
    session_id: &str,
    status: SessionRunStatus,
) {
    state
        .runtime_telemetry
        .set_session_run_status(session_id, status)
        .await;
}

pub(super) fn compaction_lifecycle_status_hook(
    state: Arc<ServerState>,
    session_id: String,
    settled_status: SessionRunStatus,
) -> rocode_session::prompt::CompactionLifecycleHook {
    Arc::new(move |summary| {
        let state = state.clone();
        let session_id = session_id.clone();
        let settled_status = settled_status.clone();
        tokio::spawn(async move {
            let next_status = match summary.status {
                rocode_types::ContextCompactionLifecycleStatus::Started => {
                    SessionRunStatus::Compacting
                }
                _ => settled_status,
            };
            set_session_run_status(&state, &session_id, next_status).await;
        });
    })
}

/// Drop guard that sets session status to idle when the prompt task exits.
/// Mirrors the TS `defer(() => cancel(sessionID))` pattern to guarantee
/// the spinner stops even if the spawned task panics.
pub(super) struct IdleGuard {
    pub state: Arc<ServerState>,
    pub session_id: Option<String>,
}

impl IdleGuard {
    /// Defuse the guard — the caller will handle cleanup explicitly.
    pub fn defuse(&mut self) {
        self.session_id = None;
    }
}

impl Drop for IdleGuard {
    fn drop(&mut self) {
        let Some(sid) = self.session_id.take() else {
            return;
        };
        let state = self.state.clone();
        tokio::spawn(async move {
            set_session_run_status(&state, &sid, SessionRunStatus::Idle).await;
        });
    }
}

// ─── Handlers ─────────────────────────────────────────────────────────

pub(super) async fn list_sessions(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<SessionListResponse>> {
    let filter = rocode_session::SessionFilter {
        directory: query.directory,
        roots: query.roots.unwrap_or(false),
        start: query.start,
        search: query.search,
        limit: query.limit,
    };
    let manager = state.sessions.lock().await;
    let sessions = manager.list_filtered(filter);
    let items: Vec<SessionListItem> = sessions.into_iter().map(session_to_list_item).collect();
    Ok(Json(session_list_response(items)))
}

pub(super) async fn session_status(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<HashMap<String, SessionStatusInfo>>> {
    let run_status = state.runtime_telemetry.session_run_statuses().await;
    let manager = state.sessions.lock().await;
    let sessions = manager.list();
    let status: HashMap<String, SessionStatusInfo> = sessions
        .into_iter()
        .map(|s| {
            let lifecycle_status = match s.status {
                rocode_session::SessionStatus::Active => "active",
                rocode_session::SessionStatus::Completed => "completed",
                rocode_session::SessionStatus::Archived => "archived",
                rocode_session::SessionStatus::Compacting => "compacting",
            };
            let run = run_status.get(&s.id).cloned().unwrap_or_default();
            let (status, idle, busy, attempt, message, next) = match run {
                SessionRunStatus::Idle => {
                    (lifecycle_status.to_string(), true, false, None, None, None)
                }
                SessionRunStatus::Compacting => {
                    ("compacting".to_string(), false, true, None, None, None)
                }
                SessionRunStatus::Busy => ("busy".to_string(), false, true, None, None, None),
                SessionRunStatus::Retry {
                    attempt,
                    message,
                    next,
                } => (
                    "retry".to_string(),
                    false,
                    true,
                    Some(attempt),
                    Some(message),
                    Some(next),
                ),
            };
            (
                s.id.clone(),
                SessionStatusInfo {
                    status,
                    idle,
                    busy,
                    attempt,
                    message,
                    next,
                },
            )
        })
        .collect();
    Ok(Json(status))
}

pub(super) async fn create_session(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<SessionInfo>> {
    if req.parent_id.is_some() {
        return Err(ApiError::BadRequest(
            "Creating an attached session via /session is no longer supported; use a typed owner-local session path instead."
                .to_string(),
        ));
    }
    let session = create_session_from_spec(
        &state,
        CreateSessionSpec {
            scheduler_profile: req.scheduler_profile,
            directory: req.directory,
            project_id: req.project_id,
            title: req.title,
        },
    )
    .await?;
    persist_sessions_if_enabled(&state).await;
    Ok(Json(session_to_info(&session)))
}

pub(crate) async fn create_session_from_spec(
    state: &Arc<ServerState>,
    spec: CreateSessionSpec,
) -> Result<rocode_session::Session> {
    let requested_scheduler_profile = spec
        .scheduler_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let effective_scheduler_profile = if let Some(profile) = requested_scheduler_profile.as_deref()
    {
        resolve_scheduler_request_defaults_validated(&state.config_store.config(), Some(profile))?
            .and_then(|defaults| defaults.profile_name)
            .or_else(|| Some(profile.to_string()))
    } else {
        None
    };
    let requested_directory = spec
        .directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolved_session_directory(value, &state.project_root()));
    let requested_project_id = spec
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let requested_title = spec
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let mut sessions = state.sessions.lock().await;
    let directory = requested_directory
        .unwrap_or_else(|| resolved_session_directory(".", &state.project_root()));
    let project_id = requested_project_id.unwrap_or_else(|| {
        PathBuf::from(&directory)
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "default".to_string())
    });
    let mut session = sessions.create(project_id, directory);
    let normalized_directory =
        resolved_session_directory(session.record().directory.as_str(), &state.project_root());
    if session.record().directory != normalized_directory {
        session.set_directory(normalized_directory);
    }
    if let Some(title) = requested_title {
        session.set_title(title);
    }
    sessions.update(session.clone());
    if let Some(profile) = effective_scheduler_profile
        .as_deref()
        .or(requested_scheduler_profile.as_deref())
    {
        session.insert_metadata("scheduler_profile", serde_json::json!(profile));
        session.insert_metadata("scheduler_applied", serde_json::json!(true));
        sessions.update(session.clone());
    }
    drop(sessions);
    Ok(session)
}

pub(super) async fn get_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>> {
    let sessions = state.sessions.lock().await;
    let session = sessions.get(&id).ok_or(ApiError::SessionNotFound(id))?;
    Ok(Json(session_to_info(session)))
}

pub(super) async fn update_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;

    if let Some(title) = req.title {
        session.set_title(title);
    }
    if let Some(time) = req.time {
        if let Some(archived) = time.archived {
            session.set_archived(Some(archived));
        }
    }
    let info = session_to_info(session);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn delete_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let deleted_session_ids = {
        let mut sessions = state.sessions.lock().await;
        let deleted_ids = collect_session_tree_ids(&sessions, &id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        sessions
            .delete(&id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        deleted_ids
    };

    for session_id in &deleted_session_ids {
        rocode_tool::tool_access::clear_tool_access_tracker(session_id);
        state
            .runtime_telemetry
            .set_session_run_status(session_id, SessionRunStatus::Idle)
            .await;
        state
            .runtime_telemetry
            .clear_session_runtime(session_id)
            .await;
    }
    persist_sessions_if_enabled(&state).await;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /session/{id}/runtime` — aggregated runtime state snapshot for a session.
pub(super) async fn get_session_runtime(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<crate::session_runtime::state::SessionRuntimeState>> {
    Ok(Json(runtime_snapshot_or_default(&state, &id).await?))
}

pub(super) async fn runtime_snapshot_or_default(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Result<crate::session_runtime::state::SessionRuntimeState> {
    match state
        .runtime_telemetry
        .get_runtime_snapshot(session_id)
        .await
    {
        Some(runtime) => Ok(runtime),
        None => {
            let sessions = state.sessions.lock().await;
            if sessions.get(session_id).is_some() {
                drop(sessions);
                Ok(crate::session_runtime::state::SessionRuntimeState::new(
                    session_id.to_string(),
                ))
            } else {
                Err(ApiError::SessionNotFound(session_id.to_string()))
            }
        }
    }
}

pub(super) async fn get_session_attached_sessions(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionListResponse>> {
    let manager = state.sessions.lock().await;
    let attached_sessions = manager.attached_sessions(&id);
    let items = attached_sessions
        .into_iter()
        .map(session_to_list_item)
        .collect();
    Ok(Json(session_list_response(items)))
}

pub(super) async fn get_session_todos(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionTodoInfo>>> {
    let sessions = state.sessions.lock().await;
    if sessions.get(&id).is_none() {
        return Err(ApiError::SessionNotFound(id));
    }
    drop(sessions);

    let todos = state.todo_manager.get(&id).await;
    let items = todos
        .into_iter()
        .enumerate()
        .map(|(idx, todo)| SessionTodoInfo {
            id: format!("{}_{}", id, idx),
            content: todo.content,
            status: todo.status,
            priority: todo.priority,
        })
        .collect();
    Ok(Json(items))
}

pub(super) async fn fork_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ForkSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let spec = SessionForkSpec {
        message_id: req.message_id.as_deref(),
        history_mode: req.history_mode.unwrap_or_default(),
        history_message_limit: req.history_message_limit,
    };
    let forked = state
        .sessions
        .lock()
        .await
        .fork(&id, spec)
        .map_err(|error| match error {
            SessionForkError::SessionNotFound => ApiError::SessionNotFound(id.clone()),
            SessionForkError::OriginMessageNotFound(message_id) => ApiError::BadRequest(format!(
                "Fork origin message not found in session `{id}`: {message_id}"
            )),
            SessionForkError::InvalidRequest(message) => ApiError::BadRequest(message),
        })?;
    persist_sessions_if_enabled(&state).await;
    Ok(Json(session_to_info(&forked)))
}

pub(super) async fn share_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionShareInfo>> {
    let mut sessions = state.sessions.lock().await;
    let share_url = format!("https://share.opencode.ai/{}", id);
    sessions
        .share(&id, share_url.clone())
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(SessionShareInfo { url: share_url }))
}

pub(super) async fn unshare_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    sessions
        .unshare(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(serde_json::json!({ "unshared": true })))
}

pub(super) async fn archive_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ArchiveSessionRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let info = if req.archive.unwrap_or(true) {
        let updated = sessions
            .set_archived(&id, None)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        session_to_info(&updated)
    } else {
        let session = sessions
            .get(&id)
            .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
        session_to_info(session)
    };
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn set_session_title(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetTitleRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    session.set_title(&req.title);
    let updated = session.clone();
    sessions.update(updated.clone());
    let info = session_to_info(&updated);
    drop(sessions);
    broadcast_session_updated(state.as_ref(), id, "session.title.set");
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn set_session_permission(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<PermissionRulesetInput>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .set_permission(
            &id,
            rocode_session::PermissionRuleset {
                allow: req.allow.unwrap_or_default(),
                deny: req.deny.unwrap_or_default(),
                mode: req.mode,
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn get_session_summary(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Option<SessionSummaryInfo>>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    Ok(Json(session.record().summary.as_ref().map(|s| {
        SessionListSummary {
            additions: s.additions,
            deletions: s.deletions,
            files: s.files,
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::collect_session_tree_ids;
    use super::{
        create_session, fork_session, get_session_attached_sessions, session_list_contract,
        session_revert, session_to_info, session_to_list_item, start_compaction, CompactRequest,
        CreateSessionRequest, ForkSessionRequest, RevertRequest,
    };
    use crate::ApiError;
    use crate::ServerState;
    use axum::{
        extract::{Path, Query, State},
        Json,
    };
    use rocode_command::stage_protocol::StageStatus;
    use rocode_session::{
        persist_session_telemetry_snapshot, MessageRole, PartType, PersistedStageTelemetrySummary,
        Session, SessionForkSpec, SessionMessage, SessionTelemetrySnapshot,
        SessionTelemetrySnapshotVersion,
    };
    use rocode_types::SessionForkHistoryMode;
    use std::sync::Arc;

    #[test]
    fn collect_session_tree_ids_includes_descendants() {
        let mut sessions = rocode_session::SessionManager::new();
        let root = sessions.create("project", "/tmp/project");
        let child = Session::attached_with_context_kind(
            &root,
            rocode_types::SessionContextKind::DelegatedSubsession,
        );
        sessions.update(child.clone());
        let grandchild = Session::attached_with_context_kind(
            &child,
            rocode_types::SessionContextKind::DelegatedSubsession,
        );
        sessions.update(grandchild.clone());

        let ids = collect_session_tree_ids(&sessions, &root.id).expect("root subtree");

        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], root.id);
        assert!(ids.contains(&child.id));
        assert!(ids.contains(&grandchild.id));
    }

    #[test]
    fn session_to_info_includes_typed_persisted_telemetry() {
        let mut session = Session::new("project", "/tmp/project");
        let snapshot = SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V1,
            usage: rocode_types::SessionUsage {
                input_tokens: 10,
                output_tokens: 20,
                reasoning_tokens: 3,
                cache_write_tokens: 4,
                cache_read_tokens: 5,
                cache_miss_tokens: 0,
                context_tokens: 0,
                total_cost: 0.25,
            },
            stage_summaries: vec![PersistedStageTelemetrySummary {
                stage_id: "stage-1".to_string(),
                stage_name: "Plan".to_string(),
                index: Some(1),
                total: Some(2),
                step: Some(1),
                step_total: Some(3),
                status: StageStatus::Running,
                prompt_tokens: Some(11),
                completion_tokens: Some(7),
                reasoning_tokens: Some(5),
                cache_read_tokens: Some(2),
                cache_write_tokens: Some(1),
                focus: Some("inspect".to_string()),
                last_event: Some("scheduler.stage.started".to_string()),
                waiting_on: None,
                estimated_context_tokens: Some(99),
                skill_tree_budget: Some(512),
                skill_tree_truncation_strategy: Some("head".to_string()),
                skill_tree_truncated: Some(false),
                retry_attempt: None,
                active_agent_count: 1,
                active_tool_count: 2,
                attached_session_count: 0,
                primary_attached_session_id: None,
            }],
            memory: None,
            compaction_continuity: None,
            last_run_status: "completed".to_string(),
            updated_at: 123,
        };
        persist_session_telemetry_snapshot(&mut session, &snapshot)
            .expect("persisted telemetry should serialize");

        let info = session_to_info(&session);

        assert_eq!(info.telemetry, Some(snapshot));
    }

    #[test]
    fn session_to_info_surfaces_typed_fork_contract() {
        let mut sessions = rocode_session::SessionManager::new();
        let mut root = sessions.create("project", "/tmp/project");
        root.insert_metadata("model_provider".to_string(), serde_json::json!("openai"));
        root.insert_metadata("model_id".to_string(), serde_json::json!("gpt-4.1"));
        root.add_user_message("one");
        root.add_user_message("two");
        root.add_user_message("three");
        sessions.update(root.clone());

        let forked = sessions
            .fork(
                &root.id,
                SessionForkSpec {
                    message_id: None,
                    history_mode: SessionForkHistoryMode::LastN,
                    history_message_limit: Some(2),
                },
            )
            .expect("fork should succeed");

        let info = session_to_info(&forked);
        let explain = info.fork.expect("fork explain should be present");
        assert_eq!(explain.history_mode, SessionForkHistoryMode::LastN);
        assert_eq!(explain.history_message_limit, Some(2));
        assert_eq!(explain.source_history_messages, Some(3));
        assert_eq!(explain.imported_history_messages, 2);
        assert!(explain.policy_frozen);
        assert_eq!(
            explain.lifecycle.compaction_scope,
            rocode_types::SessionForkLifecycleScope::ForkPromptSurface
        );
    }

    #[test]
    fn session_list_item_stays_lightweight_but_keeps_selection_hints() {
        let mut session = Session::new("project", "/tmp/project");
        session.insert_metadata("model_provider", serde_json::json!("zhipuai"));
        session.insert_metadata("model_id", serde_json::json!("glm-5.1"));
        session.insert_metadata("scheduler_profile", serde_json::json!("prometheus"));
        session.insert_metadata(
            "pending_command_invocation",
            serde_json::json!({"command": "connect"}),
        );
        persist_session_telemetry_snapshot(
            &mut session,
            &SessionTelemetrySnapshot {
                version: SessionTelemetrySnapshotVersion::V1,
                usage: rocode_types::SessionUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    reasoning_tokens: 3,
                    cache_write_tokens: 4,
                    cache_read_tokens: 5,
                    cache_miss_tokens: 0,
                    context_tokens: 0,
                    total_cost: 0.1,
                },
                stage_summaries: vec![],
                memory: None,
                compaction_continuity: None,
                last_run_status: "completed".to_string(),
                updated_at: 123,
            },
        )
        .expect("persisted telemetry should serialize");

        let item = session_to_list_item(&session);
        let value = serde_json::to_value(&item).expect("list item should serialize");

        assert_eq!(
            item.hints
                .as_ref()
                .and_then(|hints| hints.model_provider.as_deref()),
            Some("zhipuai")
        );
        assert!(item.pending_command_invocation.is_some());
        assert!(value.get("telemetry").is_none());
        assert!(value.get("metadata").is_none());
    }

    #[test]
    fn session_list_contract_exposes_search_allowlist_from_authority() {
        let contract = session_list_contract();

        assert_eq!(
            contract.filter_query_parameters,
            vec!["directory", "roots", "start", "search", "limit"]
        );
        assert_eq!(contract.search_fields, vec!["title".to_string()]);
        assert!(contract.non_search_fields.contains(&"hints".to_string()));
        assert!(contract.note.contains("hints"));
    }

    #[tokio::test]
    async fn session_attached_route_returns_list_wrapper_contract() {
        let state = Arc::new(ServerState::new());
        let parent_id = {
            let mut sessions = state.sessions.lock().await;
            let parent = sessions.create("project", "/tmp/project");
            let child = Session::attached_with_context_kind(
                &parent,
                rocode_types::SessionContextKind::DelegatedSubsession,
            );
            sessions.update(child);
            parent.id.clone()
        };

        let axum::Json(response) = get_session_attached_sessions(State(state), Path(parent_id))
            .await
            .expect("attached route should succeed");

        assert_eq!(response.items.len(), 1);
        assert_eq!(response.contract.search_fields, vec!["title".to_string()]);
        assert!(response
            .contract
            .non_search_fields
            .contains(&"hints".to_string()));
    }

    #[tokio::test]
    async fn create_session_route_rejects_legacy_parent_id_attached_creation() {
        let state = Arc::new(ServerState::new());
        let parent_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };

        let error = create_session(
            State(state),
            Json(CreateSessionRequest {
                parent_id: Some(parent_id),
                scheduler_profile: None,
                directory: None,
                project_id: None,
                title: None,
            }),
        )
        .await
        .expect_err("legacy parent-based attached creation must fail");

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn fork_session_route_rejects_invalid_last_n_contract() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            sessions.create("project", "/tmp/project").id.clone()
        };

        let error = fork_session(
            State(state),
            Path(session_id),
            Json(ForkSessionRequest {
                message_id: None,
                history_mode: Some(SessionForkHistoryMode::LastN),
                history_message_limit: None,
            }),
        )
        .await
        .expect_err("invalid last_n contract must fail closed");

        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn session_revert_route_rejects_imported_fork_history_targets() {
        let state = Arc::new(ServerState::new());
        let (forked_id, imported_message_id) = {
            let mut sessions = state.sessions.lock().await;
            let mut root = sessions.create("project", "/tmp/project");
            let imported_message_id = root.add_user_message("origin prompt").id.clone();
            sessions.update(root.clone());
            let forked = sessions
                .fork(
                    &root.id,
                    SessionForkSpec {
                        message_id: Some(imported_message_id.as_str()),
                        history_mode: SessionForkHistoryMode::All,
                        history_message_limit: None,
                    },
                )
                .expect("fork should succeed");
            (forked.id.clone(), imported_message_id)
        };

        let error = session_revert(
            State(state),
            Path(forked_id),
            Json(RevertRequest {
                message_id: imported_message_id,
                part_id: None,
                snapshot: None,
                diff: None,
            }),
        )
        .await
        .expect_err("imported fork history must be read-only for revert");

        match error {
            ApiError::BadRequest(message) => {
                assert!(message.contains("fork-local history"));
            }
            other => panic!("expected bad request, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_revert_route_accepts_fork_local_targets() {
        let state = Arc::new(ServerState::new());
        let (forked_id, local_message_id) = {
            let mut sessions = state.sessions.lock().await;
            let mut root = sessions.create("project", "/tmp/project");
            root.add_user_message("origin prompt");
            sessions.update(root.clone());
            let mut forked = sessions
                .fork(
                    &root.id,
                    SessionForkSpec {
                        message_id: None,
                        history_mode: SessionForkHistoryMode::All,
                        history_message_limit: None,
                    },
                )
                .expect("fork should succeed");
            let local_message_id = forked.add_user_message("local follow-up").id.clone();
            let forked_id = forked.id.clone();
            sessions.update(forked);
            (forked_id, local_message_id)
        };

        let axum::Json(info) = session_revert(
            State(state),
            Path(forked_id),
            Json(RevertRequest {
                message_id: local_message_id.clone(),
                part_id: None,
                snapshot: None,
                diff: None,
            }),
        )
        .await
        .expect("fork-local revert target should succeed");

        assert_eq!(
            info.revert
                .as_ref()
                .map(|revert| revert.message_id.as_str()),
            Some(local_message_id.as_str())
        );
    }

    #[tokio::test]
    async fn start_compaction_route_compacts_session_messages() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions.create("project", "/tmp/project");
            for index in 0..10 {
                session.push_message(SessionMessage::user(
                    session.id.clone(),
                    format!("message {index}"),
                ));
            }
            let session_id = session.id.clone();
            sessions.update(session);
            session_id
        };

        let axum::Json(response) = start_compaction(
            State(state.clone()),
            Path(session_id.clone()),
            Query(CompactRequest::default()),
        )
        .await
        .expect("compaction route should succeed");

        assert!(response.success);
        assert_eq!(
            response.message,
            "Session compacted (5 summarized, 5 kept)."
        );
        assert!(response.lifecycle.is_some());
        assert!(response.compaction.is_some());

        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .expect("session should still exist after compaction");
        let last_message = session
            .messages
            .last()
            .expect("compaction should append a summary message");
        assert!(matches!(last_message.role, MessageRole::Assistant));
        assert!(last_message
            .parts
            .iter()
            .any(|part| matches!(part.part_type, PartType::Compaction { .. })));
    }

    #[tokio::test]
    async fn start_compaction_route_preserves_focus_topic_in_summary() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions.create("project", "/tmp/project");
            for line in [
                "Investigate xterm integration in the browser shell",
                "Hook xterm terminal resize events",
                "Add context meter support",
                "Document xterm startup path",
                "Review other recent changes",
                "Capture current shell behavior",
                "Summarize xterm tradeoffs",
                "Check prompt handling",
                "Finish xterm notes",
                "Prepare compaction",
            ] {
                session.push_message(SessionMessage::user(session.id.clone(), line));
            }
            let session_id = session.id.clone();
            sessions.update(session);
            session_id
        };

        let axum::Json(response) = start_compaction(
            State(state.clone()),
            Path(session_id.clone()),
            Query(CompactRequest {
                focus: Some("xterm".to_string()),
            }),
        )
        .await
        .expect("focused compaction route should succeed");

        assert!(response.success);
        assert_eq!(
            response.message,
            "Session compacted around focus: xterm (5 summarized, 5 kept)."
        );
        assert_eq!(
            response
                .lifecycle
                .as_ref()
                .map(|lifecycle| lifecycle.status),
            Some(rocode_types::ContextCompactionLifecycleStatus::Installed)
        );

        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .expect("session should still exist after focused compaction");
        let summary = session
            .messages
            .last()
            .and_then(|message| message.parts.last())
            .and_then(|part| match &part.part_type {
                PartType::Compaction { summary } => Some(summary.as_str()),
                _ => None,
            })
            .expect("focused compaction should append a compaction summary");
        assert!(summary.contains("Focused on `xterm`."));
    }

    #[tokio::test]
    async fn start_compaction_route_reports_skipped_when_history_is_too_small() {
        let state = Arc::new(ServerState::new());
        let session_id = {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions.create("project", "/tmp/project");
            session.push_message(SessionMessage::user(session.id.clone(), "only one message"));
            let session_id = session.id.clone();
            sessions.update(session);
            session_id
        };

        let axum::Json(response) = start_compaction(
            State(state.clone()),
            Path(session_id.clone()),
            Query(CompactRequest::default()),
        )
        .await
        .expect("compaction route should succeed");

        assert!(!response.success);
        assert_eq!(response.message, "Nothing to compact yet.");
        assert_eq!(
            response
                .lifecycle
                .as_ref()
                .map(|lifecycle| lifecycle.status),
            Some(rocode_types::ContextCompactionLifecycleStatus::Skipped)
        );
        assert!(response.compaction.is_none());
    }
}

pub(super) async fn set_session_summary(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSummaryRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .set_summary(
            &id,
            rocode_session::SessionSummary {
                additions: req.additions.unwrap_or(0),
                deletions: req.deletions.unwrap_or(0),
                files: req.files.unwrap_or(0),
                diffs: req.diffs.map(|diffs| {
                    diffs
                        .into_iter()
                        .map(|d| rocode_session::FileDiff {
                            path: d.path,
                            additions: d.additions,
                            deletions: d.deletions,
                        })
                        .collect()
                }),
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn session_revert(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<RevertRequest>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    validate_session_revert_target(session, &req.message_id, req.part_id.as_deref())?;
    let updated = sessions
        .set_revert(
            &id,
            rocode_session::SessionRevert {
                message_id: req.message_id,
                part_id: req.part_id,
                snapshot: req.snapshot,
                diff: req.diff,
            },
        )
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn clear_session_revert(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionInfo>> {
    let mut sessions = state.sessions.lock().await;
    let updated = sessions
        .clear_revert(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let info = session_to_info(&updated);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;
    Ok(Json(info))
}

pub(super) async fn start_compaction(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Query(req): Query<CompactRequest>,
) -> Result<Json<CompactResponse>> {
    if state.runtime_telemetry.has_prompt_run(&id).await {
        return Err(ApiError::BadRequest(format!("Session {} is busy", id)));
    }
    {
        let sessions = state.sessions.lock().await;
        if sessions.get(&id).is_none() {
            return Err(ApiError::SessionNotFound(id));
        }
    }
    set_session_run_status(&state, &id, SessionRunStatus::Compacting).await;
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let outcome =
        rocode_session::compact_session_now_with_focus_result(session, req.focus.as_deref());
    drop(sessions);
    set_session_run_status(&state, &id, SessionRunStatus::Idle).await;
    broadcast_session_updated(state.as_ref(), &id, "session.compact");
    persist_sessions_if_enabled(&state).await;
    Ok(Json(CompactResponse {
        success: outcome.success(),
        message: outcome.message(req.focus.as_deref()),
        lifecycle: Some(outcome.lifecycle),
        compaction: outcome.compaction,
    }))
}

pub(super) async fn get_message(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    let sessions = state.sessions.lock().await;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    let info = serde_json::json!({
        "id": message.id,
        "sessionID": session_id,
        "role": super::messages::message_role_name(&message.role),
        "createdAt": message.created_at.timestamp_millis(),
    });
    Ok(Json(serde_json::json!({
        "info": info,
        "parts": message.parts.clone(),
    })))
}

pub(super) async fn update_part(
    State(state): State<Arc<ServerState>>,
    Path((session_id, msg_id, part_id)): Path<(String, String, String)>,
    Json(req): Json<UpdatePartRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    let message = session
        .get_message_mut(&msg_id)
        .ok_or_else(|| ApiError::NotFound(format!("Message not found: {}", msg_id)))?;

    let mut part: rocode_session::MessagePart = serde_json::from_value(req.part)
        .map_err(|e| ApiError::BadRequest(format!("Invalid part payload: {}", e)))?;
    if part.id != part_id {
        return Err(ApiError::BadRequest(format!(
            "Part id mismatch: body has {}, path has {}",
            part.id, part_id
        )));
    }
    part.message_id = Some(msg_id.clone());

    let updated_part = {
        let target = message
            .parts
            .iter_mut()
            .find(|existing| existing.id == part_id)
            .ok_or_else(|| ApiError::NotFound(format!("Part not found: {}", part_id)))?;
        *target = part.clone();
        target.clone()
    };
    session.touch();
    drop(sessions);
    persist_sessions_if_enabled(&state).await;

    Ok(Json(serde_json::json!({
        "updated": true,
        "part": updated_part,
    })))
}

pub(super) async fn execute_shell(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteShellRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    session.add_user_message(format!("$ {}", req.command));
    let assistant = session.add_assistant_message();
    assistant.add_text(format!("Shell command queued: {}", req.command));
    let assistant_id = assistant.id.clone();
    drop(sessions);
    persist_sessions_if_enabled(&state).await;

    Ok(Json(serde_json::json!({
        "executed": true,
        "command": req.command,
        "workdir": req.workdir,
        "message_id": assistant_id,
    })))
}

pub(super) async fn session_unrevert(Path(_id): Path<String>) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({ "unreverted": true, "message": "Session unreverted successfully" }),
    ))
}

pub(super) async fn execute_command(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<ExecuteCommandRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let text = req
        .arguments
        .as_deref()
        .map(|args| format!("/{cmd} {args}", cmd = req.command))
        .unwrap_or_else(|| format!("/{}", req.command));
    session.add_user_message(text);
    let assistant = session.add_assistant_message();
    assistant.add_text(format!("Command queued: {}", req.command));
    let assistant_id = assistant.id.clone();
    let arguments = req
        .arguments
        .as_deref()
        .map(|value| {
            value
                .split_whitespace()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    sessions.publish_command_executed(&req.command, &id, arguments, &assistant_id);
    drop(sessions);
    persist_sessions_if_enabled(&state).await;

    Ok(Json(serde_json::json!({
        "executed": true,
        "command": req.command,
        "arguments": req.arguments,
        "model": req.model,
        "agent": req.agent,
        "message_id": assistant_id,
    })))
}

pub(super) async fn get_session_diff(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<FileDiff>>> {
    let sessions = state.sessions.lock().await;
    let session = sessions.get(&id).ok_or(ApiError::SessionNotFound(id))?;
    let diffs = session
        .summary
        .as_ref()
        .and_then(|summary| summary.diffs.as_ref())
        .map(|items| {
            items
                .iter()
                .map(|diff| FileDiff {
                    path: diff.path.clone(),
                    additions: diff.additions,
                    deletions: diff.deletions,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Json(diffs))
}

pub(super) async fn cancel_tool_call(
    State(state): State<Arc<ServerState>>,
    Path((session_id, tool_call_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>> {
    // Verify the tool call exists in the session (hold lock briefly).
    {
        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

        let found = session.messages.iter().any(|msg| {
            msg.parts.iter().any(|part| {
                matches!(
                    &part.part_type,
                    rocode_session::PartType::ToolCall { id, .. } if id == &tool_call_id
                )
            })
        });

        if !found {
            return Err(ApiError::NotFound(format!(
                "Tool call {} not found in session {}",
                tool_call_id, session_id
            )));
        }
    }

    // Look up the plugin request mapping from global tracking
    if let Some(tracking) = rocode_plugin::subprocess::get_tool_call_tracking(&tool_call_id).await {
        // Get the plugin loader and cancel the request
        if let Some(loader) = super::super::get_plugin_loader() {
            let clients = loader.clients().await;
            if let Some(plugin) = clients
                .iter()
                .find(|c| c.plugin_id() == tracking.plugin_name)
            {
                if let Err(e) = plugin.cancel_request(tracking.request_id).await {
                    tracing::warn!(
                        tool_call_id = %tool_call_id,
                        plugin_name = %tracking.plugin_name,
                        request_id = %tracking.request_id,
                        error = %e,
                        "Failed to send cancel request to plugin"
                    );
                    return Ok(Json(serde_json::json!({
                        "cancelled": false,
                        "message": format!("Failed to cancel: {}", e)
                    })));
                }

                // Remove from tracking
                rocode_plugin::subprocess::remove_tool_call_tracking(&tool_call_id).await;

                return Ok(Json(serde_json::json!({
                    "cancelled": true,
                    "message": "Cancel request sent to plugin"
                })));
            }
        }

        return Ok(Json(serde_json::json!({
            "cancelled": false,
            "message": "Plugin not found or not loaded"
        })));
    }

    Ok(Json(serde_json::json!({
        "cancelled": false,
        "message": "Tool call is not currently executing or not tracked"
    })))
}

#[derive(Debug, Deserialize)]
pub struct PromptAsyncRequest {
    pub message: Option<String>,
    pub model: Option<String>,
}

pub(super) async fn prompt_async(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    Json(req): Json<PromptAsyncRequest>,
) -> Result<Json<serde_json::Value>> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&id)
        .ok_or_else(|| ApiError::SessionNotFound(id.clone()))?;
    let text = req
        .message
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("Field `message` is required".to_string()))?;
    session.add_user_message(text);
    let assistant = session.add_assistant_message();
    let assistant_id = assistant.id.clone();
    drop(sessions);
    persist_sessions_if_enabled(&state).await;

    Ok(Json(serde_json::json!({
        "status": "queued",
        "message_id": assistant_id,
        "model": req.model,
    })))
}
