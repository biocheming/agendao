#[path = "catalog.rs"]
mod catalog;
#[path = "commands.rs"]
mod commands;
#[path = "dialogs.rs"]
mod dialogs;
#[path = "event_loop.rs"]
mod event_loop;
#[path = "input_pipeline.rs"]
mod input_pipeline;
#[path = "mappers.rs"]
mod mappers;
#[path = "model_controls.rs"]
mod model_controls;
#[path = "permissions.rs"]
mod permissions;
#[path = "prompt_flow.rs"]
mod prompt_flow;
#[path = "questions.rs"]
mod questions;
#[path = "runtime.rs"]
mod runtime;
#[cfg(feature = "remote-stream")]
#[path = "server_events.rs"]
mod server_events;
#[path = "session_actions.rs"]
mod session_actions;
#[path = "status_panels.rs"]
mod status_panels;
#[path = "support.rs"]
mod support;
#[path = "sync.rs"]
mod sync;

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agendao_command::{CommandRegistry, UiActionId};
use agendao_command_render::output_blocks::{BlockTone, StatusBlock};
use agendao_command_runtime::interactive::{parse_interactive_command, InteractiveCommand};
use agendao_core::agent_task_registry::{global_task_registry, AgentTaskStatus};
use base64::Engine;
use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::watch;

use crate::app::state::AppState;
use crate::client::{
    ApiClient, ExecutionModeInfo, ExecutionStatus as ApiExecutionStatus, LocalServerState,
    McpStatusInfo, MemoryConflictResponse, MemoryDetailView, MemoryListQuery,
    MemoryRetrievalPreviewResponse, MemoryRetrievalQuery, MemoryValidationReportResponse,
    MessageInfo, PermissionRequestInfo, QuestionInfo, RecoveryActionKind as ApiRecoveryActionKind,
    RecoveryProtocolStatus as ApiRecoveryProtocolStatus, SessionExecutionNode, SessionInfo,
    SessionRecoveryProtocol, SessionRevertInfo,
};
use crate::components::prompt_return_flow::{format_return_flow_item, resolve_return_flow_strip};
use crate::core::{
    collect_attached_sessions, is_primary_key_event, normalize_key_event, AppContext, CustomEvent,
    Event, LeaderKeyState, McpConnectionStatus, McpServerStatus, Message, MessageRole, RevertInfo,
    Route, Session, SessionDeleteOutcome, SessionStatus, StatusDialogView, TokenUsage,
    TuiEventsBrowserState, TuiMemoryConsolidationState, TuiMemoryDetailState, TuiMemoryListState,
    TuiMemoryPreviewState, TuiMemoryRuleHitsState,
};
use crate::render::{strip_session_gutter, truncate, Clipboard, Selection};
use crate::render::{
    exit_logo_lines, Agent, AgentSelectDialog, AlertDialog, CommandPalette, ForkDialog, ForkEntry,
    HelpDialog, McpDialog, McpItem, ModeKind, Model, ModelSelectDialog, PendingSubmit,
    PermissionPrompt, Prompt, PromptStashDialog, ProviderDialog, QuestionOption, QuestionPrompt,
    QuestionRequest, QuestionType, RecoveryActionDialog, RecoveryActionItem, SessionDeleteState,
    SessionExportDialog, SessionItem, SessionListDialog, SessionRenameDialog, SkillListDialog,
    SkillProposalReviewDialog, SkillProposalReviewItem, SlashCommandPopup, StashItem, StatusDialog,
    StatusLine, SubagentDialog, TagDialog, TaskKind, ThemeListDialog, ThemeOption, TimelineDialog,
    TimelineEntry, Toast, ToastVariant, ToolCallCancelDialog, ToolCallItem, OTHER_OPTION_ID,
    OTHER_OPTION_LABEL,
};
use crate::state::MessagePart as ContextMessagePart;

use self::mappers::{
    agent_color_from_name, apply_incremental_session_sync, infer_task_kind_from_message,
    map_api_diff, map_api_message, map_api_revert, map_api_run_status, map_api_session,
    map_api_todo, map_mcp_status, provider_from_model,
};
use self::runtime::spawn_tui_direct_event_bridge;
#[cfg(feature = "remote-stream")]
use self::server_events::{
    env_var, env_var_enabled, resolve_tui_base_url, spawn_server_event_listener_task, SessionFilter,
};
use self::support::{
    append_execution_status_node, apply_selected_mode, current_mode_label, default_export_filename,
    format_theme_option_label, map_execution_mode_to_dialog_option, parse_model_ref_selection,
    recovery_action_items, recovery_status_blocks_from_protocol, resolve_command_execution_mode,
    resolve_recovery_action_selection, selected_execution_mode, status_line_from_block,
};

#[cfg(not(feature = "remote-stream"))]
type SessionFilter = watch::Sender<Option<String>>;

#[cfg(not(feature = "remote-stream"))]
fn env_var_enabled(_name: &str) -> bool {
    false
}

#[cfg(not(feature = "remote-stream"))]
fn env_var(_name: &str) -> Option<String> {
    None
}

#[cfg(not(feature = "remote-stream"))]
fn resolve_tui_base_url(base_url_override: Option<&str>) -> String {
    base_url_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("http://localhost:3000")
        .to_string()
}

const SESSION_SYNC_DEBOUNCE_MS: u64 = 180;
const SESSION_TELEMETRY_SYNC_DEBOUNCE_MS: u64 = 120;
const QUESTION_SYNC_DEBOUNCE_MS: u64 = 40;
const PERMISSION_SYNC_DEBOUNCE_MS: u64 = 40;
const PROCESS_REFRESH_DEBOUNCE_MS: u64 = 120;
const PERF_LOG_INTERVAL_SECS: u64 = 10;
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_DIM: &str = "\x1b[90m";
const ANSI_BOLD: &str = "\x1b[1m";

pub struct App {
    context: Arc<AppContext>,
    local_direct: bool,
    local_server: Option<Arc<LocalServerState>>,
    state: AppState,
    viewport_area: Rect,
    prompt: Prompt,
    selection: Selection,
    command_palette: CommandPalette,
    slash_popup: SlashCommandPopup,
    leader_state: LeaderKeyState,
    model_select: ModelSelectDialog,
    agent_select: AgentSelectDialog,
    alert_dialog: AlertDialog,
    help_dialog: HelpDialog,
    session_list_dialog: SessionListDialog,
    session_rename_dialog: SessionRenameDialog,
    session_export_dialog: SessionExportDialog,
    prompt_stash_dialog: PromptStashDialog,
    skill_list_dialog: SkillListDialog,
    theme_list_dialog: ThemeListDialog,
    status_dialog: StatusDialog,
    mcp_dialog: McpDialog,
    timeline_dialog: TimelineDialog,
    fork_dialog: ForkDialog,
    provider_dialog: ProviderDialog,
    subagent_dialog: SubagentDialog,
    tag_dialog: TagDialog,
    tool_call_cancel_dialog: ToolCallCancelDialog,
    skill_proposal_review_dialog: SkillProposalReviewDialog,
    recovery_action_dialog: RecoveryActionDialog,
    permission_prompt: PermissionPrompt,
    question_prompt: QuestionPrompt,
    toast: Toast,
    /// Snapshot of rendered screen lines for text selection copy.
    screen_lines: Arc<std::sync::Mutex<Vec<String>>>,
    available_models: HashSet<String>,
    model_variants: HashMap<String, Vec<String>>,
    model_variant_selection: HashMap<String, Option<String>>,
    permission_runtime: PermissionRuntimeState,
    question_runtime: QuestionRuntimeState,
    sync_runtime: SyncLifecycleState,
    diagnostics: DiagnosticsState,
    event_caused_change: bool,
    suppress_current_terminal_event_for_reratui: bool,
    /// Session IDs whose scheduler handoff metadata has been consumed.
    consumed_handoffs: HashSet<String>,
    /// Base URL for the server event stream.
    server_event_base_url: String,
    server_password: Option<String>,
    /// Shared session filter for the SSE listener task.
    /// Updated when navigating to a different session so the listener
    /// reconnects with `?session={id}`.
    sse_session_filter: SessionFilter,
    /// Unix socket path for event subscription (socket mode).
    unix_socket_path: Option<String>,
    prompt_draft: PromptDraft,
    pending_shell_dispatch: Option<PendingShellDispatch>,
}

#[derive(Clone)]
pub(crate) struct ReactiveDialogLayerSnapshot {
    pub(crate) show_backdrop: bool,
    pub(crate) permission_prompt: PermissionPrompt,
    pub(crate) question_prompt: QuestionPrompt,
    pub(crate) help_dialog: HelpDialog,
    pub(crate) alert_dialog: AlertDialog,
    pub(crate) slash_popup: SlashCommandPopup,
    pub(crate) command_palette: CommandPalette,
    pub(crate) model_select: ModelSelectDialog,
    pub(crate) agent_select: AgentSelectDialog,
    pub(crate) status_dialog: StatusDialog,
    pub(crate) session_list_dialog: SessionListDialog,
    pub(crate) session_export_dialog: SessionExportDialog,
    pub(crate) session_rename_dialog: SessionRenameDialog,
    pub(crate) skill_list_dialog: SkillListDialog,
    pub(crate) mcp_dialog: McpDialog,
    pub(crate) timeline_dialog: TimelineDialog,
    pub(crate) fork_dialog: ForkDialog,
    pub(crate) provider_dialog: ProviderDialog,
    pub(crate) recovery_action_dialog: RecoveryActionDialog,
    pub(crate) skill_proposal_review_dialog: SkillProposalReviewDialog,
    pub(crate) subagent_dialog: SubagentDialog,
    pub(crate) tag_dialog: TagDialog,
    pub(crate) theme_list_dialog: ThemeListDialog,
    pub(crate) prompt_stash_dialog: PromptStashDialog,
    pub(crate) tool_call_cancel_dialog: ToolCallCancelDialog,
}

/// Tracks an in-flight shell dispatch so the TUI can observe the
/// gap between ignition and settlement — and so Esc can hit a real
/// Cancelled path instead of falling through to session-level abort.
#[derive(Clone, Debug)]
struct PendingShellDispatch {
    session_id: String,
    optimistic_message_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct RunOutcome {
    pub exit_summary: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PromptDraft {
    attachments: Vec<crate::api::PromptPart>,
}

impl PromptDraft {
    pub(crate) fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    pub(crate) fn image_count(&self) -> usize {
        self.attachments
            .iter()
            .filter(|part| {
                matches!(
                    part,
                    crate::api::PromptPart::File {
                        mime: Some(mime),
                        ..
                    } if mime.starts_with("image/")
                )
            })
            .count()
    }

    fn has_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }

    fn push_attachment(&mut self, part: crate::api::PromptPart) {
        self.attachments.push(part);
    }

    fn clear_attachments(&mut self) {
        self.attachments.clear();
    }

    fn take_attachments(&mut self) -> Option<Vec<crate::api::PromptPart>> {
        self.has_attachments()
            .then(|| std::mem::take(&mut self.attachments))
    }

    fn attachment_hint(&self) -> Option<String> {
        let attachment_count = self.attachments.len();
        if attachment_count == 0 {
            return None;
        }
        let image_count = self.image_count();
        Some(if image_count == attachment_count {
            if image_count == 1 {
                "1 image attached".to_string()
            } else {
                format!("{} images attached", image_count)
            }
        } else if attachment_count == 1 {
            "1 attachment queued".to_string()
        } else {
            format!("{} attachments queued", attachment_count)
        })
    }
}

#[derive(Clone, Debug, Default)]
struct PendingQuestionDraft {
    current_index: usize,
    answers: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Default)]
struct PermissionRuntimeState {
    pending_ids: HashSet<String>,
    pending_requests: HashMap<String, PermissionRequestInfo>,
    last_submit_error: Option<String>,
    last_submit_started_at: Option<String>,
    last_submit_completed_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct QuestionRuntimeState {
    pending_ids: HashSet<String>,
    pending_queue: VecDeque<String>,
    pending_questions: HashMap<String, QuestionInfo>,
    pending_drafts: HashMap<String, PendingQuestionDraft>,
}

#[derive(Clone, Debug, Default)]
struct SelectedExecutionMode {
    agent: Option<String>,
    scheduler_profile: Option<String>,
    display_mode: Option<String>,
}

#[derive(Clone, Debug)]
struct SyncLifecycleState {
    pending_initial_submit: bool,
    pending_session_sync: Option<String>,
    pending_session_sync_due_at: Option<Instant>,
    pending_session_telemetry_sync: Option<String>,
    pending_session_telemetry_sync_due_at: Option<Instant>,
    pending_question_sync_due_at: Option<Instant>,
    pending_permission_sync_due_at: Option<Instant>,
    pending_process_refresh_due_at: Option<Instant>,
    session_telemetry_sync_inflight: bool,
    last_tick_at: Instant,
    last_session_sync: Instant,
    last_question_sync: Instant,
    last_permission_sync: Instant,
    last_process_refresh: Instant,
    last_perf_log: Instant,
    last_ui_bridge_dropped_events: u64,
}

impl SyncLifecycleState {
    fn new(now: Instant, pending_initial_submit: bool) -> Self {
        Self {
            pending_initial_submit,
            pending_session_sync: None,
            pending_session_sync_due_at: None,
            pending_session_telemetry_sync: None,
            pending_session_telemetry_sync_due_at: None,
            pending_question_sync_due_at: None,
            pending_permission_sync_due_at: None,
            pending_process_refresh_due_at: None,
            session_telemetry_sync_inflight: false,
            last_tick_at: now,
            last_session_sync: now,
            last_question_sync: now,
            last_permission_sync: now,
            last_process_refresh: now,
            last_perf_log: now,
            last_ui_bridge_dropped_events: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PerfCounters {
    draws: u64,
    screen_snapshots: u64,
    session_sync_full: u64,
    session_sync_incremental: u64,
    question_sync: u64,
    session_updated_events: u64,
}

#[derive(Clone, Debug, Default)]
struct DiagnosticsState {
    perf: PerfCounters,
    perf_log_info: bool,
}

#[derive(Clone)]
pub(crate) struct BridgeLoopSnapshot {
    pub(crate) is_exiting: bool,
    pub(crate) tick_due: bool,
    pub(crate) wait_strategy: BridgeWaitStrategy,
    pub(crate) max_events_per_frame: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeWaitStrategy {
    PollReady,
    Wait { deadline: Option<Instant> },
}

pub(crate) struct BridgeIterationOutcome {
    pub(crate) should_draw: bool,
    pub(crate) reratui_event: Option<crossterm::event::Event>,
    pub(crate) reactive_root_snapshot: Option<crate::bridge::ReactiveRootSnapshot>,
}

pub(crate) struct BridgeRuntimeStartup {
    pub(crate) app_context: Arc<AppContext>,
    pub(crate) server_event_task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionSyncMode {
    Full,
    Incremental,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptDispatchOutcome {
    AwaitingUser,
    Queued,
    Running,
    Failed,
}

#[derive(Clone, Default)]
pub struct AppLaunchConfig {
    pub base_url: Option<String>,
    pub server_password: Option<String>,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub initial_prompt: Option<String>,
    pub working_dir: Option<PathBuf>,
    /// Unix socket path for local IPC transport auto-selection.
    pub unix_socket_path: Option<String>,
    /// Run in Direct (in-process) mode — no server, no IPC.
    /// The TUI constructs OrchestrationCore internally with
    /// unified session authority.
    pub local_direct: bool,
    /// Optional shared in-process server authority for Direct mode so the
    /// product shell can resolve sessions before launching the TUI.
    pub local_server: Option<Arc<LocalServerState>>,
}

impl App {
    fn prompt_dispatch_outcome(
        response: Option<&crate::api::PromptResponse>,
        error: Option<&str>,
    ) -> PromptDispatchOutcome {
        if error.is_some() {
            return PromptDispatchOutcome::Failed;
        }
        match response.map(|value| value.status.as_str()) {
            Some("awaiting_user") => PromptDispatchOutcome::AwaitingUser,
            Some("queued") => PromptDispatchOutcome::Queued,
            _ => PromptDispatchOutcome::Running,
        }
    }

    fn settle_prompt_dispatch(
        &mut self,
        session_id: &str,
        optimistic_message_id: &str,
        response: Option<&crate::api::PromptResponse>,
        error: Option<&str>,
    ) {
        match Self::prompt_dispatch_outcome(response, error) {
            PromptDispatchOutcome::Failed => {
                self.remove_optimistic_message(session_id, optimistic_message_id);
                self.set_session_status(session_id, SessionStatus::Idle);
                self.sync_prompt_spinner_state();
                if let Some(err) = error {
                    self.alert_dialog
                        .set_message(&format!("Failed to send prompt:\n{}", err));
                    self.open_alert_dialog();
                }
            }
            PromptDispatchOutcome::AwaitingUser => {
                self.set_session_status(session_id, SessionStatus::Idle);
                self.prompt.set_spinner_active(false);
                self.queue_session_telemetry_refresh(session_id);
                self.sync_question_requests();
            }
            PromptDispatchOutcome::Queued => {
                self.set_session_status(session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
                self.queue_session_telemetry_refresh(session_id);
            }
            PromptDispatchOutcome::Running => {
                self.set_session_status(session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
            }
        }
    }

    pub fn new() -> anyhow::Result<Self> {
        Self::new_with_config(AppLaunchConfig::default())
    }

    pub fn new_with_config(config: AppLaunchConfig) -> anyhow::Result<Self> {
        let context = Arc::new(AppContext::new());
        let mut prompt = Prompt::new(context.clone())
            .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
        let mut pending_initial_submit = false;
        let mut initial_session_id: Option<String> = None;

        if let Some(dir) = config.working_dir.as_ref() {
            *context.directory.write() = dir.display().to_string();
        } else if let Ok(dir) = std::env::current_dir() {
            *context.directory.write() = dir.display().to_string();
        }

        let base_url = if config.local_direct {
            "direct://local".to_string()
        } else {
            resolve_tui_base_url(config.base_url.as_deref())
        };
        let api_client = if config.local_direct {
            Arc::new(ApiClient::new_local_with_server(
                config.local_server.clone(),
            ))
        } else {
            Arc::new(ApiClient::new_with_password(
                base_url.clone(),
                config.server_password.clone(),
                config.unix_socket_path.clone(),
            )?)
        };
        context.set_api_client(api_client);
        let (sse_session_filter, _session_filter_rx) = watch::channel(None::<String>);

        if let Some(agent) = config
            .agent_name
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("AGENDAO_TUI_AGENT"))
        {
            let agent = agent.trim();
            if !agent.is_empty() {
                context.set_agent(agent.to_string());
            }
        }
        if let Some(model) = config
            .model
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("AGENDAO_TUI_MODEL"))
        {
            let model = model.trim();
            if !model.is_empty() {
                context.set_model_selection(model.to_string(), provider_from_model(model));
                context.set_model_variant(None);
            }
        }
        if let Some(session_id) = config
            .session_id
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("AGENDAO_TUI_SESSION"))
        {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                initial_session_id = Some(session_id.to_string());
                // Set the SSE session filter so the listener subscribes
                // to this session's events from the start.
                sse_session_filter.send_replace(Some(session_id.to_string()));
                context.navigate(Route::Session {
                    session_id: session_id.to_string(),
                });
            }
        }
        if let Some(initial_prompt) = config
            .initial_prompt
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("AGENDAO_TUI_PROMPT"))
        {
            let initial_prompt = initial_prompt.trim();
            if !initial_prompt.is_empty() {
                prompt.set_input(initial_prompt.to_string());
                pending_initial_submit = true;
            }
        }
        {
            let theme = context.theme.read().clone();
            let mode_name = current_mode_label(&context).unwrap_or_default();
            prompt.set_spinner_color(agent_color_from_name(&theme, &mode_name));
        }

        let now = Instant::now();
        let mut app = Self {
            context,
            local_direct: config.local_direct,
            local_server: config.local_server,
            state: AppState::default(),
            viewport_area: Rect::default(),
            prompt,
            selection: Selection::new(),
            command_palette: CommandPalette::new(),
            slash_popup: SlashCommandPopup::new(),
            leader_state: LeaderKeyState::new(),
            model_select: ModelSelectDialog::new(),
            agent_select: AgentSelectDialog::new(),
            alert_dialog: AlertDialog::info(""),
            help_dialog: HelpDialog::new(),
            session_list_dialog: SessionListDialog::new(),
            session_rename_dialog: SessionRenameDialog::new(),
            session_export_dialog: SessionExportDialog::new(),
            prompt_stash_dialog: PromptStashDialog::new(),
            skill_list_dialog: SkillListDialog::new(),
            theme_list_dialog: ThemeListDialog::new(),
            status_dialog: StatusDialog::new(),
            mcp_dialog: McpDialog::new(),
            timeline_dialog: TimelineDialog::new(),
            fork_dialog: ForkDialog::new(),
            provider_dialog: ProviderDialog::new(),
            subagent_dialog: SubagentDialog::new(),
            tag_dialog: TagDialog::new(),
            tool_call_cancel_dialog: ToolCallCancelDialog::new(),
            skill_proposal_review_dialog: SkillProposalReviewDialog::new(),
            recovery_action_dialog: RecoveryActionDialog::new(),
            permission_prompt: PermissionPrompt::new(),
            question_prompt: QuestionPrompt::new(),
            toast: Toast::new(),
            screen_lines: Arc::new(std::sync::Mutex::new(Vec::new())),
            available_models: HashSet::new(),
            model_variants: HashMap::new(),
            model_variant_selection: HashMap::new(),
            permission_runtime: PermissionRuntimeState::default(),
            question_runtime: QuestionRuntimeState::default(),
            sync_runtime: SyncLifecycleState::new(now, pending_initial_submit),
            diagnostics: DiagnosticsState {
                perf: PerfCounters::default(),
                perf_log_info: env_var_enabled("AGENDAO_PERF_LOG"),
            },
            event_caused_change: true,
            suppress_current_terminal_event_for_reratui: false,
            consumed_handoffs: HashSet::new(),
            server_event_base_url: base_url,
            server_password: config.server_password,
            sse_session_filter,
            unix_socket_path: config.unix_socket_path,
            prompt_draft: PromptDraft::default(),
            pending_shell_dispatch: None,
        };

        let _ = app.sync_config_from_server();
        app.refresh_model_dialog();
        app.refresh_agent_dialog();
        let _ = app.refresh_skill_list_dialog();
        app.refresh_session_list_dialog();
        app.refresh_theme_list_dialog();
        if !app.local_direct {
            let _ = app.refresh_lsp_status();
            let _ = app.refresh_mcp_dialog();
        }
        if !app.local_direct {
            let _ = app.sync_question_requests();
            let _ = app.sync_permission_requests();
        }

        if let Some(session_id) = initial_session_id {
            let _ = app.sync_session_from_server(&session_id);
            app.ensure_session_view(&session_id);
        }
        app.sync_prompt_spinner_style();
        app.sync_prompt_spinner_state();
        app.sync_prompt_draft_hint();

        Ok(app)
    }

    pub fn run(self) -> anyhow::Result<RunOutcome> {
        crate::bridge::run_app(self)
    }

    pub fn exit_summary(&self) -> Option<String> {
        let Route::Session { session_id } = self.context.current_route() else {
            return None;
        };
        let session_ctx = self.context.session.read();
        let session = session_ctx.sessions.get(&session_id)?;
        let title = truncate(&session.title.replace(['\r', '\n'], " "), 50);
        let pad_label = |label: &str| format!("{ANSI_DIM}{:<10}{ANSI_RESET}", label);

        let mut lines = Vec::new();
        lines.push(String::new());
        lines.extend(exit_logo_lines("  "));
        lines.push(String::new());
        lines.push(format!(
            "  {}{ANSI_BOLD}{}{ANSI_RESET}",
            pad_label("Session"),
            title
        ));
        lines.push(format!(
            "  {}{ANSI_BOLD}agendao tui -s {}{ANSI_RESET}",
            pad_label("Continue"),
            session.id
        ));
        lines.push(String::new());
        Some(lines.join("\n"))
    }

    pub(crate) fn process_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        self.suppress_current_terminal_event_for_reratui = false;
        self.context.record_ui_event(event);
        self.handle_event(event)?;
        Ok(self.event_caused_change)
    }

    pub(crate) fn should_forward_current_terminal_event_to_reratui(&self) -> bool {
        !self.suppress_current_terminal_event_for_reratui
    }

    pub(crate) fn current_reratui_event(
        &self,
        event: Option<&Event>,
    ) -> Option<crossterm::event::Event> {
        if !self.should_forward_current_terminal_event_to_reratui() {
            return None;
        }
        event.and_then(Self::map_terminal_event_for_reratui)
    }

    pub(crate) fn suppress_current_terminal_event_for_reratui(&mut self) {
        self.suppress_current_terminal_event_for_reratui = true;
    }

    pub(crate) fn drain_pending_events(&mut self, limit: usize) -> anyhow::Result<bool> {
        let mut should_draw = false;

        for next in self.context.drain_ui_events(limit) {
            should_draw |= self.process_event(&next)?;
        }

        Ok(should_draw)
    }

    pub(crate) fn process_bridge_iteration(
        &mut self,
        resized_area: Option<Rect>,
        tick_due: bool,
        max_events_per_frame: usize,
        polled_event: Option<&Event>,
        capture_root_snapshot: bool,
    ) -> anyhow::Result<BridgeIterationOutcome> {
        if let Some(area) = resized_area {
            self.set_viewport_area(area);
        }

        let mut should_draw = false;
        if tick_due {
            should_draw |= self.process_event(&Event::Tick)?;
        }
        should_draw |= self.drain_pending_events(max_events_per_frame)?;
        if let Some(event) = polled_event {
            should_draw |= self.process_event(event)?;
        }

        let reactive_root_snapshot = if should_draw || capture_root_snapshot {
            Some(self.prepare_reactive_root_snapshot(self.viewport_area))
        } else {
            None
        };

        Ok(BridgeIterationOutcome {
            should_draw,
            reratui_event: self.current_reratui_event(polled_event),
            reactive_root_snapshot,
        })
    }

    pub(crate) fn is_exiting(&self) -> bool {
        self.state == AppState::Exiting
    }

    pub(crate) fn bridge_loop_snapshot(
        &self,
        now: Instant,
        first_frame: bool,
    ) -> BridgeLoopSnapshot {
        let tick_deadline = self.next_tick_deadline(now);
        let tick_due = tick_deadline.is_some_and(|deadline| deadline <= now);
        let bridge_pending = self.context.ui_bridge_pending_event_count() > 0;
        BridgeLoopSnapshot {
            is_exiting: self.is_exiting(),
            tick_due,
            wait_strategy: if !first_frame && !tick_due && !bridge_pending {
                BridgeWaitStrategy::Wait {
                    deadline: tick_deadline,
                }
            } else {
                BridgeWaitStrategy::PollReady
            },
            max_events_per_frame: self.context.runtime_budget().max_events_per_frame.max(1),
        }
    }

    pub(crate) fn prepare_bridge_runtime_start(
        &mut self,
        initial_area: Option<Rect>,
    ) -> BridgeRuntimeStartup {
        if let Some(area) = initial_area {
            self.set_viewport_area(area);
        }
        BridgeRuntimeStartup {
            app_context: self.context.clone(),
            server_event_task: self.spawn_server_event_listener_task(),
        }
    }

    pub(crate) fn spawn_server_event_listener_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        if self.local_direct {
            return spawn_tui_direct_event_bridge(
                self.local_server.clone(),
                self.sse_session_filter.clone(),
                self.context.ui_bridge.clone(),
            );
        }
        if let Some(socket_path) = self.unix_socket_path.clone() {
            let ui_bridge = self.context.ui_bridge.clone();
            let filter = self.sse_session_filter.clone();
            return Some(tokio::spawn(async move {
                runtime::socket_event_subscriber(socket_path, filter, ui_bridge).await;
            }));
        }
        #[cfg(feature = "remote-stream")]
        {
            return Some(spawn_server_event_listener_task(
                self.context.ui_bridge.clone(),
                self.server_event_base_url.clone(),
                self.server_password.clone(),
                self.sse_session_filter.clone(),
            ));
        }

        #[cfg(not(feature = "remote-stream"))]
        {
            None
        }
    }

    pub(crate) fn set_viewport_area(&mut self, area: Rect) {
        self.viewport_area = area;
    }

    pub(crate) fn can_render_reactive_route(&self) -> bool {
        !self.has_non_reactive_dialog_layer()
    }

    pub(crate) fn context_handle(&self) -> Arc<AppContext> {
        self.context.clone()
    }

    pub(crate) fn prepare_reactive_render(&mut self, area: Rect) {
        self.viewport_area = area;
        self.context
            .set_pending_permissions(self.permission_prompt.pending_count());
    }

    pub(crate) fn prepare_reactive_root_snapshot(
        &mut self,
        area: Rect,
    ) -> crate::bridge::ReactiveRootSnapshot {
        debug_assert!(
            self.can_render_reactive_route(),
            "legacy frame fallback should be unreachable after reratui migration"
        );
        self.prepare_reactive_render(area);
        self.reactive_root_snapshot()
    }

    fn map_terminal_event_for_reratui(event: &Event) -> Option<crossterm::event::Event> {
        match event {
            Event::Key(key) => Some(crossterm::event::Event::Key(*key)),
            Event::Mouse(mouse) => Some(crossterm::event::Event::Mouse(*mouse)),
            Event::Resize(width, height) => {
                Some(crossterm::event::Event::Resize(*width, *height))
            }
            Event::Paste(text) => Some(crossterm::event::Event::Paste(text.clone())),
            Event::FocusGained => Some(crossterm::event::Event::FocusGained),
            Event::FocusLost => Some(crossterm::event::Event::FocusLost),
            Event::Tick | Event::Custom(_) => None,
        }
    }

    pub(crate) fn reactive_root_snapshot(&self) -> crate::bridge::ReactiveRootSnapshot {
        let dialog_layer = self.reactive_dialog_layer_snapshot();
        let app_context = self.context_handle();
        let theme = app_context.theme.read().clone();
        let keybinds = app_context.keybind.read().clone();
        let route = app_context.current_route();
        let animations_enabled = *app_context.animations_enabled.read();
        let prompt_input_blocked = app_context.has_blocking_dialogs();
        let slash_popup_open = app_context.is_dialog_open(crate::context::DialogSlot::SlashPopup);
        let prompt = self.prompt_handle();
        let selection = self.selection_snapshot();
        let toast = self.toast_snapshot();
        let screen_lines = self.take_reactive_screen_lines();
        crate::bridge::ReactiveRootSnapshot {
            app_context,
            theme,
            keybinds,
            route,
            animations_enabled,
            prompt_input_blocked,
            slash_popup_open,
            prompt,
            selection,
            toast,
            dialog_layer,
            screen_lines,
        }
    }

    pub(crate) fn prompt_handle(&self) -> Prompt {
        self.prompt.clone()
    }

    pub(crate) fn selection_snapshot(&self) -> Selection {
        self.selection.clone()
    }

    pub(crate) fn toast_snapshot(&self) -> Toast {
        self.toast.clone()
    }

    pub(crate) fn reactive_dialog_layer_snapshot(&self) -> ReactiveDialogLayerSnapshot {
        ReactiveDialogLayerSnapshot {
            show_backdrop: self.has_reactive_home_dialog_layer()
                || self.permission_prompt.is_open
                || self.question_prompt.is_open,
            permission_prompt: self.permission_prompt.clone(),
            question_prompt: self.question_prompt.clone(),
            help_dialog: self.help_dialog.clone(),
            alert_dialog: self.alert_dialog.clone(),
            slash_popup: self.slash_popup.clone(),
            command_palette: self.command_palette.clone(),
            model_select: self.model_select.clone(),
            agent_select: self.agent_select.clone(),
            status_dialog: self.status_dialog.clone(),
            session_list_dialog: self.session_list_dialog.clone(),
            session_export_dialog: self.session_export_dialog.clone(),
            session_rename_dialog: self.session_rename_dialog.clone(),
            skill_list_dialog: self.skill_list_dialog.clone(),
            mcp_dialog: self.mcp_dialog.clone(),
            timeline_dialog: self.timeline_dialog.clone(),
            fork_dialog: self.fork_dialog.clone(),
            provider_dialog: self.provider_dialog.clone(),
            recovery_action_dialog: self.recovery_action_dialog.clone(),
            skill_proposal_review_dialog: self.skill_proposal_review_dialog.clone(),
            subagent_dialog: self.subagent_dialog.clone(),
            tag_dialog: self.tag_dialog.clone(),
            theme_list_dialog: self.theme_list_dialog.clone(),
            prompt_stash_dialog: self.prompt_stash_dialog.clone(),
            tool_call_cancel_dialog: self.tool_call_cancel_dialog.clone(),
        }
    }

    pub(crate) fn take_reactive_screen_lines(&self) -> Arc<std::sync::Mutex<Vec<String>>> {
        Arc::clone(&self.screen_lines)
    }

    // P0-3: TUI state mutation gate (lock-level, documented; full semantic
    // convergence is ongoing).
    // All transport-driven state changes now flow through explicit CustomEvent /
    // FrontendEvent variants before mutating context and triggering rerender.
    // The context.session.write() lock remains the single state mutation gate.

    fn terminal_width(&self) -> u16 {
        self.viewport_area.width
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
