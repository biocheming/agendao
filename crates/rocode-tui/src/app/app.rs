#[path = "catalog.rs"]
mod catalog;
#[path = "commands.rs"]
mod commands;
#[path = "dialogs.rs"]
mod dialogs;
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
use std::thread;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Block};
use rocode_command::interactive::{parse_interactive_command, InteractiveCommand};
use rocode_command::output_blocks::{BlockTone, StatusBlock};
use rocode_command::{CommandRegistry, UiActionId};
use rocode_core::agent_task_registry::{global_task_registry, AgentTaskStatus};

use crate::api::{
    ApiClient, ExecutionModeInfo, ExecutionStatus as ApiExecutionStatus, McpStatusInfo,
    MemoryConflictResponse, MemoryDetailView, MemoryListQuery, MemoryRetrievalPreviewResponse,
    MemoryRetrievalQuery, MemoryValidationReportResponse, MessageInfo, PermissionRequestInfo,
    QuestionInfo, RecoveryActionKind as ApiRecoveryActionKind,
    RecoveryProtocolStatus as ApiRecoveryProtocolStatus, SessionExecutionNode, SessionInfo,
    SessionRecoveryProtocol, SessionRevertInfo,
};
use crate::app::state::AppState;
use crate::components::{
    exit_logo_lines, Agent, AgentSelectDialog, AlertDialog, CommandPalette, ForkDialog, ForkEntry,
    HelpDialog, HomeView, McpDialog, McpItem, ModeKind, Model, ModelSelectDialog, PendingSubmit,
    PermissionAction, PermissionPrompt, Prompt, PromptStashDialog, ProviderDialog, QuestionOption,
    QuestionPrompt, QuestionRequest, QuestionType, RecoveryActionDialog, RecoveryActionItem,
    SessionDeleteState, SessionExportDialog, SessionItem, SessionListDialog, SessionRenameDialog,
    SkillListDialog, SkillProposalReviewDialog, SkillProposalReviewItem, SlashCommandPopup,
    StashItem, StatusDialog, StatusLine, SubagentDialog, TagDialog, TaskKind, ThemeListDialog,
    ThemeOption, TimelineDialog, TimelineEntry, Toast, ToastVariant, ToolCallCancelDialog,
    ToolCallItem, OTHER_OPTION_ID, OTHER_OPTION_LABEL,
};
use crate::context::keybind::{is_primary_key_event, normalize_key_event, LeaderKeyState};
use crate::context::{
    collect_attached_sessions, AppContext, McpConnectionStatus, McpServerStatus, Message,
    MessagePart as ContextMessagePart, MessageRole, RevertInfo, Session, SessionStatus,
    StatusDialogView, TokenUsage, TuiEventsBrowserState, TuiMemoryConsolidationState,
    TuiMemoryDetailState, TuiMemoryListState, TuiMemoryPreviewState, TuiMemoryRuleHitsState,
};
use crate::event::{CustomEvent, Event, StateChange};
use crate::router::Route;
use crate::ui::{
    apply_selection_highlight, capture_screen_lines, strip_session_gutter, truncate, BufferSurface,
    Clipboard, RenderSurface, Selection,
};

use self::mappers::{
    agent_color_from_name, apply_incremental_session_sync, infer_task_kind_from_message,
    map_api_diff, map_api_message, map_api_revert, map_api_run_status, map_api_session,
    map_api_todo, map_mcp_status, provider_from_model,
};
use self::server_events::{
    env_var, env_var_enabled, resolve_tui_base_url, spawn_server_event_listener_task, SessionFilter,
};
use self::support::{
    append_execution_status_node, apply_selected_mode, current_mode_label, default_export_filename,
    format_theme_option_label, map_execution_mode_to_dialog_option, parse_model_ref_selection,
    recovery_action_items, recovery_status_blocks_from_protocol, resolve_command_execution_mode,
    resolve_recovery_action_selection, selected_execution_mode, status_line_from_block,
};

const SESSION_SYNC_DEBOUNCE_MS: u64 = 180;
const SESSION_FULL_SYNC_INTERVAL_SECS: u64 = 10;
const QUESTION_SYNC_FALLBACK_SECS: u64 = 5;
const PERMISSION_SYNC_FALLBACK_SECS: u64 = 5;
const PERMISSION_SYNC_BACKOFF_SECS: u64 = 15;
const AUX_SYNC_INTERVAL_SECS: u64 = 5;
const AUX_SYNC_BACKOFF_SECS: u64 = 15;
const PERF_LOG_INTERVAL_SECS: u64 = 10;
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_DIM: &str = "\x1b[90m";
const ANSI_BOLD: &str = "\x1b[1m";

fn session_update_requires_sync(source: Option<&str>) -> bool {
    // P1-2: exclude high-frequency sources that are handled incrementally
    // via output blocks. "topology" (ReconcileReason::Topology) is the new
    // canonical source for scheduler stage deltas — it fires on every stage
    // message change and must NOT trigger a full session sync.
    !matches!(
        source,
        Some(
            "prompt.stream"
                | "stream.prompt"
                | "prompt.scheduler.stage.content"
                | "prompt.scheduler.stage.reasoning"
                | "prompt.scheduler.stage.child.final"
                | "topology"
        )
    )
}

pub struct App {
    context: Arc<AppContext>,
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
    screen_lines: Vec<String>,
    available_models: HashSet<String>,
    model_variants: HashMap<String, Vec<String>>,
    model_variant_selection: HashMap<String, Option<String>>,
    prompt_runtime: PromptRuntimeState,
    permission_runtime: PermissionRuntimeState,
    question_runtime: QuestionRuntimeState,
    sync_runtime: SyncLifecycleState,
    diagnostics: DiagnosticsState,
    event_caused_change: bool,
    /// Session IDs whose scheduler handoff metadata has been consumed.
    consumed_handoffs: HashSet<String>,
    /// Base URL for the server event stream.
    server_event_base_url: String,
    server_password: Option<String>,
    /// Shared session filter for the SSE listener task.
    /// Updated when navigating to a different session so the listener
    /// reconnects with `?session={id}`.
    sse_session_filter: SessionFilter,
}

#[derive(Clone, Debug, Default)]
pub struct RunOutcome {
    pub exit_summary: Option<String>,
}

#[derive(Clone, Debug)]
struct QueuedPrompt {
    input: String,
    display_text: String,
    parts: Option<Vec<crate::api::PromptPart>>,
    agent: Option<String>,
    scheduler_profile: Option<String>,
    display_mode: Option<String>,
    model: Option<String>,
    variant: Option<String>,
    idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct PendingQuestionDraft {
    current_index: usize,
    answers: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Default)]
struct PromptRuntimeState {
    pending_queue: HashMap<String, VecDeque<QueuedPrompt>>,
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
    last_tick_at: Instant,
    last_session_sync: Instant,
    last_full_session_sync: Instant,
    last_question_sync: Instant,
    last_permission_sync: Instant,
    last_aux_sync: Instant,
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
            last_tick_at: now,
            last_session_sync: now,
            last_full_session_sync: now,
            last_question_sync: now,
            last_permission_sync: now,
            last_aux_sync: now,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionSyncMode {
    Full,
    Incremental,
}

#[derive(Clone, Debug, Default)]
pub struct AppLaunchConfig {
    pub base_url: Option<String>,
    pub server_password: Option<String>,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub initial_prompt: Option<String>,
    pub working_dir: Option<PathBuf>,
}

impl App {
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

        let base_url = resolve_tui_base_url(config.base_url.as_deref());
        let api_client = Arc::new(ApiClient::new_with_password(
            base_url.clone(),
            config.server_password.clone(),
        ));
        context.set_api_client(api_client);
        let sse_session_filter: SessionFilter = Arc::new(std::sync::Mutex::new(None));

        if let Some(agent) = config
            .agent_name
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("ROCODE_TUI_AGENT"))
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
            .or_else(|| env_var("ROCODE_TUI_MODEL"))
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
            .or_else(|| env_var("ROCODE_TUI_SESSION"))
        {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                initial_session_id = Some(session_id.to_string());
                // Set the SSE session filter so the listener subscribes
                // to this session's events from the start.
                if let Ok(mut filter) = sse_session_filter.lock() {
                    *filter = Some(session_id.to_string());
                }
                context.navigate(Route::Session {
                    session_id: session_id.to_string(),
                });
            }
        }
        if let Some(initial_prompt) = config
            .initial_prompt
            .as_deref()
            .map(str::to_string)
            .or_else(|| env_var("ROCODE_TUI_PROMPT"))
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
            screen_lines: Vec::new(),
            available_models: HashSet::new(),
            model_variants: HashMap::new(),
            model_variant_selection: HashMap::new(),
            prompt_runtime: PromptRuntimeState::default(),
            permission_runtime: PermissionRuntimeState::default(),
            question_runtime: QuestionRuntimeState::default(),
            sync_runtime: SyncLifecycleState::new(now, pending_initial_submit),
            diagnostics: DiagnosticsState {
                perf: PerfCounters::default(),
                perf_log_info: env_var_enabled("ROCODE_PERF_LOG"),
            },
            event_caused_change: true,
            consumed_handoffs: HashSet::new(),
            server_event_base_url: base_url,
            server_password: config.server_password,
            sse_session_filter,
        };

        let _ = app.sync_config_from_server();
        app.refresh_model_dialog();
        app.refresh_agent_dialog();
        let _ = app.refresh_skill_list_dialog();
        app.refresh_session_list_dialog();
        app.refresh_theme_list_dialog();
        let _ = app.refresh_lsp_status();
        let _ = app.refresh_mcp_dialog();
        let _ = app.sync_question_requests();
        let _ = app.sync_permission_requests();

        if let Some(session_id) = initial_session_id {
            let _ = app.sync_session_from_server(&session_id);
            app.ensure_session_view(&session_id);
        }
        app.sync_prompt_spinner_style();
        app.sync_prompt_spinner_state();

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
            "  {}{ANSI_BOLD}rocode tui -s {}{ANSI_RESET}",
            pad_label("Continue"),
            session.id
        ));
        lines.push(String::new());
        Some(lines.join("\n"))
    }

    pub(crate) fn process_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        self.context.record_ui_event(event);
        self.handle_event(event)?;
        Ok(self.event_caused_change)
    }

    pub(crate) fn drain_pending_events(&mut self, limit: usize) -> anyhow::Result<bool> {
        let mut should_draw = false;

        for next in self.context.drain_ui_events(limit) {
            should_draw |= self.process_event(&next)?;
        }

        Ok(should_draw)
    }

    pub(crate) fn is_exiting(&self) -> bool {
        self.state == AppState::Exiting
    }

    pub(crate) fn spawn_server_event_listener_task(&self) -> tokio::task::JoinHandle<()> {
        spawn_server_event_listener_task(
            self.context.ui_bridge.clone(),
            self.server_event_base_url.clone(),
            self.server_password.clone(),
            self.sse_session_filter.clone(),
        )
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

    pub(crate) fn begin_reactive_render(&mut self, area: Rect) {
        self.viewport_area = area;
        self.context
            .set_pending_permissions(self.permission_prompt.pending_count());
    }

    pub(crate) fn render_home_view<S: RenderSurface>(&self, surface: &mut S, area: Rect) {
        let home = HomeView::new(self.context.clone());
        home.render_with_prompt(surface, area, &self.prompt);
    }

    pub(crate) fn render_session_view(
        &self,
        view: &crate::components::SessionView,
        context: &Arc<AppContext>,
        buffer: &mut Buffer,
        area: Rect,
    ) -> Option<(u16, u16)> {
        let mut surface = BufferSurface::new(buffer);
        view.render(context, &mut surface, area, &self.prompt);
        surface.cursor_position()
    }

    pub(crate) fn render_reactive_dialog_layer<S: RenderSurface>(
        &mut self,
        surface: &mut S,
        area: Rect,
        theme: &crate::theme::Theme,
    ) {
        if !self.has_reactive_home_dialog_layer()
            && !self.permission_prompt.is_open
            && !self.question_prompt.is_open
            && !self.tool_call_cancel_dialog.is_open()
        {
            return;
        }

        if self.has_reactive_home_dialog_layer()
            || self.permission_prompt.is_open
            || self.question_prompt.is_open
        {
            let modal_backdrop = Block::default().style(Style::default().bg(theme.background_menu));
            surface.render_widget(modal_backdrop, area);
        }
        self.slash_popup.render(surface, area, theme);
        self.help_dialog.render(surface, area, theme);
        self.alert_dialog.render(surface, area, theme);
        self.command_palette.render(surface, area, theme);
        self.model_select.render(surface, area, theme);
        self.agent_select.render(surface, area, theme);
        self.session_list_dialog.render(surface, area, theme);
        self.theme_list_dialog.render(surface, area, theme);
        self.mcp_dialog.render(surface, area, theme);
        self.timeline_dialog.render(surface, area, theme);
        self.fork_dialog.render(surface, area, theme);
        self.subagent_dialog.render(surface, area, theme);
        self.tag_dialog.render(surface, area, theme);
        self.recovery_action_dialog.render(surface, area, theme);
        self.skill_proposal_review_dialog
            .render(surface, area, theme);
        self.status_dialog.render(surface, area, theme);
        self.session_rename_dialog.render(surface, area, theme);
        self.session_export_dialog.render(surface, area, theme);
        self.prompt_stash_dialog.render(surface, area, theme);
        self.skill_list_dialog.render(surface, area, theme);
        self.provider_dialog.render(surface, area, theme);
        self.permission_prompt.render(surface, area, theme);
        self.question_prompt.render(surface, area, theme);
        self.tool_call_cancel_dialog.render(surface, area, theme);
    }

    pub(crate) fn render_reactive_toast<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &crate::theme::Theme,
    ) {
        if !self.toast.is_visible() {
            return;
        }

        let toast_width = 60u16.min(area.width.saturating_sub(4));
        let toast_height = self.toast.desired_height(toast_width);
        let base_x = area.x + area.width.saturating_sub(toast_width.saturating_add(2));
        let max_x = area.x + area.width.saturating_sub(toast_width);
        let toast_x = base_x.saturating_add(self.toast.slide_offset()).min(max_x);
        let toast_area = Rect {
            x: toast_x,
            y: 2.min(area.height.saturating_sub(1)),
            width: toast_width,
            height: toast_height.min(area.height.saturating_sub(2)),
        };
        self.toast.render(surface, toast_area, &theme);
    }

    pub(crate) fn capture_reactive_screen_lines(&mut self, buffer: &Buffer, area: Rect) {
        let should_capture_screen_lines =
            self.selection.is_active() || self.selection.is_selecting();
        if !should_capture_screen_lines {
            return;
        }

        self.screen_lines = capture_screen_lines(buffer, area);
        self.diagnostics.perf.screen_snapshots =
            self.diagnostics.perf.screen_snapshots.saturating_add(1);
    }

    pub(crate) fn apply_reactive_selection(&self, buffer: &mut Buffer, area: Rect) {
        apply_selection_highlight(buffer, area, &self.selection);
    }

    fn handle_event(&mut self, event: &Event) -> anyhow::Result<()> {
        self.event_caused_change = true;

        match event {
            Event::Key(key) => {
                if !is_primary_key_event(*key) {
                    return Ok(());
                }
                let key = normalize_key_event(*key);

                // Handle inline permission prompt before dialogs
                if self.permission_prompt.is_open {
                    if self.permission_prompt.is_current_request_submitting() {
                        return Ok(());
                    }
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Enter => {
                            if let Some(request) = self.permission_prompt.approve_once() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "once",
                                    Some("approved once".to_string()),
                                );
                            }
                        }
                        KeyCode::Char('0') | KeyCode::Char('n') => {
                            if let Some(request) = self.permission_prompt.deny() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "reject",
                                    Some("rejected".to_string()),
                                );
                            }
                        }
                        KeyCode::Char('1') => {
                            if let Some(request) = self.permission_prompt.approve_once() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "once",
                                    Some("approved once".to_string()),
                                );
                            }
                        }
                        KeyCode::Char('2') => {
                            if let Some(request) = self.permission_prompt.approve_turn() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "turn",
                                    Some("approved for turn".to_string()),
                                );
                            }
                        }
                        KeyCode::Char('3') | KeyCode::Char('a') => {
                            if let Some(request) = self.permission_prompt.approve_session() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "session",
                                    Some("approved for session".to_string()),
                                );
                            }
                        }
                        KeyCode::Esc => {
                            if let Some(request) = self.permission_prompt.deny() {
                                self.resolve_permission_request(
                                    &request.id,
                                    "reject",
                                    Some("rejected".to_string()),
                                );
                            }
                        }
                        _ => {}
                    }
                    return Ok(());
                }

                // Handle inline question prompt before dialogs
                if self.question_prompt.is_open {
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab => self.question_prompt.move_up(),
                        KeyCode::Down | KeyCode::Tab => self.question_prompt.move_down(),
                        KeyCode::Char(' ') => self.question_prompt.handle_space(),
                        KeyCode::Enter => {
                            if let Some((question, answers)) = self.question_prompt.confirm() {
                                self.submit_question_reply(&question.id, answers);
                            }
                        }
                        KeyCode::Esc => {
                            if let Some(question) = self.question_prompt.current().cloned() {
                                self.reject_question(&question.id);
                            }
                            self.question_prompt.close();
                        }
                        KeyCode::Char(c) => self.question_prompt.type_char(c),
                        KeyCode::Backspace => self.question_prompt.backspace(),
                        _ => {}
                    }
                    return Ok(());
                }

                if self.handle_dialog_key(key)? {
                    return Ok(());
                }

                // Leader key handling
                if self.leader_state.active {
                    if self.leader_state.check_timeout() {
                        // Leader timed out, fall through to normal handling
                    } else {
                        let action = match key.code {
                            KeyCode::Char('n') => Some(UiActionId::NewSession),
                            KeyCode::Char('l') => Some(UiActionId::OpenSessionList),
                            KeyCode::Char('m') => Some(UiActionId::OpenModelList),
                            KeyCode::Char('a') => Some(UiActionId::OpenAgentList),
                            KeyCode::Char('t') => Some(UiActionId::OpenThemeList),
                            KeyCode::Char('b') => Some(UiActionId::ToggleSidebar),
                            KeyCode::Char('s') => Some(UiActionId::ViewStatus),
                            KeyCode::Char('q') => Some(UiActionId::Exit),
                            KeyCode::Char('u') => Some(UiActionId::Undo),
                            KeyCode::Char('r') => Some(UiActionId::Redo),
                            _ => None,
                        };
                        self.leader_state.reset();
                        if let Some(action) = action {
                            self.execute_ui_action(action)?;
                        }
                        return Ok(());
                    }
                }

                // Ctrl+X starts leader key sequence
                if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
                    self.leader_state.start(KeyCode::Char('x'));
                    return Ok(());
                }

                // Ctrl+Shift+C (crossterm reports uppercase 'C' with SHIFT modifier)
                if (key.code == KeyCode::Char('C') || key.code == KeyCode::Char('c'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    self.copy_selection();
                    return Ok(());
                }

                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    // If there's an active selection, copy it instead of exiting (TS parity)
                    if self.selection.is_active() {
                        self.copy_selection();
                        return Ok(());
                    }
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                // Ctrl+K to cancel current running tool call or session
                if key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL {
                    tracing::info!("Ctrl+K pressed");
                    if let Some(session_id) = self.current_session_id() {
                        let active_tool_calls = self.context.get_active_tool_calls();
                        let tool_call_count = active_tool_calls.len();
                        tracing::info!(
                            "Active session: {}, tool call count: {}",
                            session_id,
                            tool_call_count
                        );

                        if tool_call_count > 1 {
                            // Multiple tool calls - show selection dialog
                            let items: Vec<ToolCallItem> = active_tool_calls
                                .values()
                                .map(|info| ToolCallItem {
                                    id: info.id.clone(),
                                    tool_name: info.tool_name.clone(),
                                })
                                .collect();
                            self.open_tool_call_cancel_dialog_modal(items);
                        } else if tool_call_count == 1 {
                            // Single tool call - cancel directly
                            if let Some(api) = self.context.get_api_client() {
                                let tool_call_id = active_tool_calls.keys().next().unwrap().clone();
                                if let Err(e) = api.cancel_tool_call(&session_id, &tool_call_id) {
                                    self.toast.show(
                                        ToastVariant::Error,
                                        &format!("Failed to cancel tool: {}", e),
                                        3000,
                                    );
                                } else {
                                    self.toast.show(
                                        ToastVariant::Info,
                                        "Tool cancellation requested",
                                        3000,
                                    );
                                }
                            }
                        } else {
                            // No tool calls - cancel session
                            if let Some(api) = self.context.get_api_client() {
                                match api.abort_session(&session_id) {
                                    Err(e) => {
                                        self.toast.show(
                                            ToastVariant::Error,
                                            &format!("Failed to cancel session: {}", e),
                                            3000,
                                        );
                                    }
                                    Ok(value) => {
                                        let message = value
                                            .get("target")
                                            .and_then(|value| value.as_str())
                                            .map(|target| match target {
                                                "stage" => {
                                                    let stage = value
                                                        .get("stage")
                                                        .and_then(|value| value.as_str())
                                                        .unwrap_or("current stage");
                                                    format!(
                                                        "Stage cancellation requested: {}",
                                                        stage
                                                    )
                                                }
                                                _ => "Run cancellation requested".to_string(),
                                            })
                                            .unwrap_or_else(|| {
                                                "Run cancellation requested".to_string()
                                            });
                                        self.toast.show(ToastVariant::Info, &message, 3000);
                                    }
                                }
                            }
                        }
                    }
                    return Ok(());
                }

                if key.code == KeyCode::Esc {
                    if let Some(sv) = self.context.session_view_handle() {
                        if sv.clear_sidebar_focus() {
                            return Ok(());
                        }
                    }
                    if self.selection.is_active() {
                        self.selection.clear();
                        return Ok(());
                    }
                }

                // Process panel keyboard handling (when focused)
                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_process_focus())
                {
                    let proc_count = self.context.processes.read().len();
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_process_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            sv.move_sidebar_process_selection_down(proc_count);
                            return Ok(());
                        }
                        KeyCode::Char('d') | KeyCode::Delete => {
                            let procs = self.context.processes.read().clone();
                            if let Some(proc) = procs.get(sv.sidebar_process_selected()) {
                                let _ =
                                    rocode_orchestrator::global_lifecycle().kill_process(proc.pid);
                                *self.context.processes.write() =
                                    rocode_core::process_registry::global_registry().list();
                                sv.clamp_sidebar_process_selection(
                                    self.context.processes.read().len(),
                                );
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_workspace_focus())
                {
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_workspace_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            let count = sv.sidebar_workspace_node_count();
                            sv.move_sidebar_workspace_selection_down(count);
                            return Ok(());
                        }
                        KeyCode::Left => {
                            if sv.collapse_sidebar_workspace_selection() {
                                return Ok(());
                            }
                        }
                        KeyCode::Right => {
                            if sv.expand_sidebar_workspace_selection() {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }

                // Attached-session panel keyboard handling (when focused)
                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_attached_session_focus())
                {
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_attached_session_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            let count = self.context.attached_sessions().len();
                            sv.move_sidebar_attached_session_selection_down(count);
                            return Ok(());
                        }
                        KeyCode::Enter => {
                            let sessions = self.context.attached_sessions();
                            if let Some(child) =
                                sessions.get(sv.sidebar_attached_session_selected())
                            {
                                let attached_id = child.session_id.clone();
                                self.context.navigate_session(attached_id.clone());
                                self.ensure_session_view(&attached_id);
                                let _ = self.sync_session_from_server(&attached_id);
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                // 'p' toggles process panel focus when sidebar is visible
                if key.code == KeyCode::Char('p') && key.modifiers.is_empty() {
                    if let Some(sv) = self.context.session_view_handle() {
                        if sv.toggle_sidebar_process_focus(self.terminal_width()) {
                            return Ok(());
                        }
                    }
                }

                if key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                if self.matches_keybind("session_interrupt", key) {
                    if self.prompt.is_shell_mode() {
                        self.prompt.exit_shell_mode();
                        self.prompt.clear_interrupt_confirmation();
                        return Ok(());
                    }
                    if let Route::Session { session_id } = self.context.current_route() {
                        let status = {
                            let session_ctx = self.context.session.read();
                            session_ctx.status(&session_id).clone()
                        };
                        if !matches!(status, SessionStatus::Idle) {
                            if !self.prompt.register_interrupt_keypress() {
                                return Ok(());
                            }
                            if let Some(client) = self.context.get_api_client() {
                                let _ = client.abort_session(&session_id);
                            }
                            self.prompt.clear_interrupt_confirmation();
                            self.set_session_status(&session_id, SessionStatus::Idle);
                            self.sync_prompt_spinner_state();
                            return Ok(());
                        }
                    }
                    self.prompt.clear_interrupt_confirmation();
                    return Ok(());
                }

                if self.matches_keybind("input_paste", key) {
                    self.paste_clipboard_to_prompt();
                    return Ok(());
                }
                if self.matches_keybind("input_copy", key) {
                    self.copy_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("input_cut", key) {
                    self.cut_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("history_previous", key) {
                    self.prompt.history_previous_entry();
                    return Ok(());
                }
                if self.matches_keybind("history_next", key) {
                    self.prompt.history_next_entry();
                    return Ok(());
                }
                if self.matches_keybind("page_up", key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(sv) = self.context.session_view_handle() {
                            sv.scroll_page_up();
                            return Ok(());
                        }
                    }
                }
                if self.matches_keybind("page_down", key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(sv) = self.context.session_view_handle() {
                            sv.scroll_page_down();
                            return Ok(());
                        }
                    }
                }

                if self.matches_keybind("command_palette", key) {
                    self.sync_command_palette_labels();
                    self.open_command_palette_dialog();
                    return Ok(());
                }
                if self.matches_keybind("model_cycle", key) {
                    self.refresh_model_dialog();
                    self.open_model_select_dialog();
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle", key) {
                    self.cycle_agent(1);
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle_reverse", key) {
                    self.cycle_agent(-1);
                    return Ok(());
                }
                if self.matches_keybind("variant_cycle", key) {
                    self.cycle_model_variant();
                    return Ok(());
                }
                if self.matches_keybind("session_parent", key) {
                    self.navigate_to_parent_session();
                    return Ok(());
                }
                if self.matches_keybind("session_attached_open", key) {
                    self.navigate_to_attached_session();
                    return Ok(());
                }
                if self.matches_keybind("session_attached_focus", key) {
                    if let Some(sv) = self.context.session_view_handle() {
                        let _ = sv.toggle_sidebar_attached_session_focus(self.terminal_width());
                    }
                    return Ok(());
                }
                if self.matches_keybind("session_workspace_focus", key) {
                    if let Some(sv) = self.context.session_view_handle() {
                        let _ = sv.toggle_sidebar_workspace_focus(self.terminal_width());
                    }
                    return Ok(());
                }
                if self.matches_keybind("sidebar_toggle", key) {
                    self.toggle_session_sidebar();
                    return Ok(());
                }
                if self.matches_keybind("display_thinking", key) {
                    self.context.toggle_thinking();
                    return Ok(());
                }
                if self.matches_keybind("tool_details", key) {
                    self.context.toggle_tool_details();
                    return Ok(());
                }
                if self.matches_keybind("input_clear", key) {
                    self.prompt.clear();
                    return Ok(());
                }
                if self.matches_keybind("input_newline", key) {
                    let route = self.context.current_route();
                    if matches!(route, Route::Home | Route::Session { .. }) {
                        self.prompt.insert_text("\n");
                        return Ok(());
                    }
                }
                if self.matches_keybind("help_toggle", key) {
                    self.open_help_dialog();
                    return Ok(());
                }

                let route = self.context.current_route();
                match route {
                    Route::Home | Route::Session { .. } => {
                        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                            self.submit_prompt()?;
                        } else if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(key);
                        }
                    }
                    _ => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(key);
                        }
                    }
                }
            }
            Event::Resize(width, height) => {
                self.viewport_area = Rect::new(0, 0, *width, *height);
            }
            Event::Mouse(mouse_event) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                match mouse_event.kind {
                    MouseEventKind::Down(button) => {
                        let col = mouse_event.column;
                        let row = mouse_event.row;

                        if button == MouseButton::Right {
                            // Right-click copies selection (if any) then clears it
                            if self.selection.is_active() {
                                self.copy_selection();
                            }
                            return Ok(());
                        }

                        if self.permission_prompt.is_open {
                            self.permission_prompt.handle_click(col, row);
                            if let Some(action) = self.permission_prompt.take_pending_action() {
                                match action {
                                    PermissionAction::ApproveOnce => {
                                        if let Some(request) = self.permission_prompt.approve_once()
                                        {
                                            self.resolve_permission_request(
                                                &request.id,
                                                "once",
                                                Some("approved once".to_string()),
                                            );
                                        }
                                    }
                                    PermissionAction::ApproveTurn => {
                                        if let Some(request) = self.permission_prompt.approve_turn()
                                        {
                                            self.resolve_permission_request(
                                                &request.id,
                                                "turn",
                                                Some("approved for turn".to_string()),
                                            );
                                        }
                                    }
                                    PermissionAction::ApproveSession => {
                                        if let Some(request) =
                                            self.permission_prompt.approve_session()
                                        {
                                            self.resolve_permission_request(
                                                &request.id,
                                                "session",
                                                Some("approved for session".to_string()),
                                            );
                                        }
                                    }
                                    PermissionAction::Deny => {
                                        if let Some(request) = self.permission_prompt.deny() {
                                            self.resolve_permission_request(
                                                &request.id,
                                                "reject",
                                                Some("rejected".to_string()),
                                            );
                                        }
                                    }
                                }
                            }
                            return Ok(());
                        }

                        // Question prompt click
                        if self.question_prompt.is_open {
                            self.question_prompt.handle_click(col, row);
                            return Ok(());
                        }

                        if self.status_dialog.is_open() {
                            if self.status_dialog.handle_click(col, row) {
                                self.close_status_dialog_modal();
                                return Ok(());
                            }
                            if self.status_dialog.contains_point(col, row) {
                                if button == MouseButton::Left {
                                    if let Some(area) = self.status_dialog.selection_area() {
                                        if col >= area.x
                                            && col < area.x.saturating_add(area.width)
                                            && row >= area.y
                                            && row < area.y.saturating_add(area.height)
                                        {
                                            self.selection.start_scoped(row, col, Some(area));
                                        } else {
                                            self.selection.clear();
                                        }
                                    }
                                }
                                return Ok(());
                            }
                        }

                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }

                        if button == MouseButton::Left {
                            if let Route::Session { .. } = self.context.current_route() {
                                if let Some(sv) = self.context.session_view_handle() {
                                    if sv.handle_sidebar_click(&self.context, col, row) {
                                        // Check if the click triggered attached-session navigation
                                        if let Some(cs_idx) = sv.take_pending_navigate_attached() {
                                            let sessions = self.context.attached_sessions();
                                            if let Some(child) = sessions.get(cs_idx) {
                                                let attached_id = child.session_id.clone();
                                                self.context.navigate_session(attached_id.clone());
                                                self.ensure_session_view(&attached_id);
                                                let _ = self.sync_session_from_server(&attached_id);
                                            }
                                        }
                                        if sv.take_pending_navigate_parent() {
                                            self.navigate_to_parent_session();
                                        }
                                        return Ok(());
                                    }
                                    if sv.is_point_in_sidebar(col, row) {
                                        return Ok(());
                                    }
                                    if sv.handle_scrollbar_click(col, row) {
                                        return Ok(());
                                    }
                                    if sv.handle_click(col, row) {
                                        return Ok(());
                                    }
                                }
                            }
                            if let Route::Session { .. } = self.context.current_route() {
                                if let Some(sv) = self.context.session_view_handle() {
                                    if let Some(area) = sv.selection_area() {
                                        if col >= area.x
                                            && col < area.x.saturating_add(area.width)
                                            && row >= area.y
                                            && row < area.y.saturating_add(area.height)
                                        {
                                            self.selection.start_scoped(row, col, Some(area));
                                        } else {
                                            self.selection.clear();
                                        }
                                    } else {
                                        self.selection.clear();
                                    }
                                }
                            } else {
                                // Clear previous selection and start a new one
                                self.selection.start(row, col);
                            }
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if !sv.scroll_sidebar_up_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_up_mouse();
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if !sv.scroll_sidebar_down_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_down_mouse();
                            }
                        }
                    }
                    MouseEventKind::Drag(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.update(mouse_event.row, mouse_event.column);
                            return Ok(());
                        }
                        let col = mouse_event.column;
                        let row = mouse_event.row;
                        if let Some(sv) = self.context.session_view_handle() {
                            if sv.handle_scrollbar_drag(col, row) {
                                return Ok(());
                            }
                        }
                        self.selection.update(row, col);
                    }
                    MouseEventKind::Moved => {
                        if self.status_dialog.is_open() {
                            self.event_caused_change = false;
                            return Ok(());
                        }
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        self.event_caused_change = false;
                    }
                    MouseEventKind::Up(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.finalize();
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if sv.stop_scrollbar_drag() {
                                return Ok(());
                            }
                        }
                        self.selection.finalize();
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                if !text.is_empty() {
                    if self.provider_dialog.is_open() && self.provider_dialog.accepts_text_input() {
                        for c in text.chars() {
                            self.provider_dialog.push_char(c);
                        }
                    } else {
                        self.prompt.insert_text(text);
                    }
                }
            }
            Event::Custom(event) => match event.as_ref() {
                CustomEvent::PromptDispatchHomeFinished {
                    optimistic_session_id,
                    optimistic_message_id,
                    created_session,
                    response,
                    error,
                } => {
                    if let Some(session) = created_session.as_deref() {
                        self.promote_optimistic_session(optimistic_session_id, session);

                        if let Route::Session { session_id: active } = self.context.current_route()
                        {
                            if active == *optimistic_session_id {
                                self.context.navigate(Route::Session {
                                    session_id: session.id.clone(),
                                });
                            }
                        }
                        self.ensure_session_view(&session.id);

                        if let Some(err) = error {
                            self.remove_optimistic_message(&session.id, optimistic_message_id);
                            self.set_session_status(&session.id, SessionStatus::Idle);
                            self.sync_prompt_spinner_state();
                            self.alert_dialog
                                .set_message(&format!("Failed to send prompt:\n{}", err));
                            self.open_alert_dialog();
                        } else {
                            match response.as_ref().map(|response| response.status.as_str()) {
                                Some("awaiting_user") => {
                                    self.set_session_status(&session.id, SessionStatus::Idle);
                                    self.prompt.set_spinner_active(false);
                                    self.refresh_session_telemetry(&session.id);
                                    self.sync_question_requests();
                                }
                                _ => {
                                    self.set_session_status(&session.id, SessionStatus::Running);
                                    self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                                    self.prompt.set_spinner_active(true);
                                }
                            }
                        }
                    } else {
                        self.remove_optimistic_session(optimistic_session_id);
                        if let Route::Session { session_id: active } = self.context.current_route()
                        {
                            if active == *optimistic_session_id {
                                self.context.navigate_home();
                            }
                        }
                        self.prompt.set_spinner_active(false);
                        if let Some(err) = error {
                            self.alert_dialog
                                .set_message(&format!("Failed to create session:\n{}", err));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::PromptDispatchSessionFinished {
                    session_id,
                    optimistic_message_id,
                    response,
                    error,
                } => {
                    if let Some(err) = error {
                        self.remove_optimistic_message(session_id, optimistic_message_id);
                        self.set_session_status(session_id, SessionStatus::Idle);
                        let _ = self.dispatch_next_queued_prompt(session_id);
                        self.sync_prompt_spinner_state();
                        self.alert_dialog
                            .set_message(&format!("Failed to send prompt:\n{}", err));
                        self.open_alert_dialog();
                    } else if matches!(
                        response.as_ref().map(|response| response.status.as_str()),
                        Some("awaiting_user")
                    ) {
                        self.set_session_status(session_id, SessionStatus::Idle);
                        self.prompt.set_spinner_active(false);
                        self.refresh_session_telemetry(session_id);
                        self.sync_question_requests();
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::PermissionReplyFinished {
                    permission_id,
                    outcome,
                } => {
                    self.permission_runtime.last_submit_completed_at =
                        Some(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                    match outcome {
                        crate::event::PermissionReplyOutcome::Succeeded => {
                            self.permission_runtime.last_submit_error = None;
                            self.toast.show(
                                crate::components::ToastVariant::Success,
                                "Permission reply sent",
                                2000,
                            );
                        }
                        crate::event::PermissionReplyOutcome::Failed { message } => {
                            self.permission_runtime.last_submit_error = Some(message.clone());
                            self.permission_prompt
                                .mark_submit_failed(permission_id, message.clone());
                            self.alert_dialog.set_message(&format!(
                                "Failed to submit permission response:\n{}",
                                message
                            ));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::StateChanged(StateChange::SessionUpdated { session_id, source }) => {
                    self.diagnostics.perf.session_updated_events = self
                        .diagnostics
                        .perf
                        .session_updated_events
                        .saturating_add(1);
                    // P1-3: session.updated is the RECONCILE FALLBACK path.
                    // Incremental updates (output blocks, custom events) are the
                    // PRIMARY refresh mechanism. This handler only triggers a
                    // debounced full sync for non-droppable reconcile reasons.
                    if let Route::Session { session_id: active } = self.context.current_route() {
                        if active == *session_id && session_update_requires_sync(source.as_deref())
                        {
                            self.sync_runtime.pending_session_sync = Some(session_id.to_string());
                            self.sync_runtime.pending_session_sync_due_at = Some(
                                Instant::now() + Duration::from_millis(SESSION_SYNC_DEBOUNCE_MS),
                            );
                        }
                    }
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusBusy(session_id)) => {
                    self.set_session_status(session_id, SessionStatus::Running);
                    self.refresh_session_telemetry(session_id);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusCompacting(session_id)) => {
                    self.set_session_status(session_id, SessionStatus::Compacting);
                    self.refresh_session_telemetry(session_id);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusIdle(session_id)) => {
                    self.set_session_status(session_id, SessionStatus::Idle);
                    self.refresh_session_telemetry(session_id);
                    let _ = self.dispatch_next_queued_prompt(session_id);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::SessionStatusRetrying {
                    session_id,
                    attempt,
                    message,
                    next,
                }) => {
                    self.set_session_status(
                        session_id,
                        SessionStatus::Retrying {
                            message: message.clone(),
                            attempt: *attempt,
                            next: *next,
                        },
                    );
                    self.refresh_session_telemetry(session_id);
                    self.sync_prompt_spinner_state();
                }
                CustomEvent::StateChanged(StateChange::ConfigUpdated) => {
                    let _ = self.sync_config_from_server();
                    self.event_caused_change = true;
                }
                CustomEvent::StateChanged(StateChange::QuestionCreated { session_id, .. })
                | CustomEvent::StateChanged(StateChange::QuestionResolved { session_id, .. }) => {
                    let should_sync = match self.context.current_route() {
                        Route::Session {
                            session_id: active_session_id,
                        } => active_session_id == *session_id,
                        _ => true,
                    };
                    if should_sync {
                        self.event_caused_change = self.sync_question_requests();
                        self.sync_runtime.last_question_sync = Instant::now();
                    }
                }
                CustomEvent::StateChanged(StateChange::PermissionRequested {
                    session_id,
                    permission,
                }) => {
                    let should_surface = match self.context.current_route() {
                        Route::Session {
                            session_id: active_session_id,
                        } => active_session_id == *session_id,
                        _ => true,
                    };
                    if should_surface {
                        self.enqueue_permission_request(permission.clone());
                        self.sync_runtime.last_permission_sync = Instant::now();
                        self.event_caused_change = true;
                    }
                }
                CustomEvent::StateChanged(StateChange::PermissionResolved {
                    session_id,
                    permission_id,
                }) => {
                    let should_surface = match self.context.current_route() {
                        Route::Session {
                            session_id: active_session_id,
                        } => active_session_id == *session_id,
                        _ => true,
                    };
                    if should_surface {
                        self.clear_permission_request(permission_id);
                        self.permission_runtime.last_submit_error = None;
                        self.sync_runtime.last_permission_sync = Instant::now();
                        self.event_caused_change = true;
                    }
                }
                CustomEvent::StateChanged(StateChange::ToolCallStarted { session_id, .. }) => {
                    // Refresh runtime state to get updated active tools from server
                    self.refresh_session_telemetry(session_id);
                }
                CustomEvent::StateChanged(StateChange::ToolCallCompleted {
                    session_id, ..
                }) => {
                    // Refresh runtime state to get updated active tools from server
                    self.refresh_session_telemetry(session_id);
                }
                CustomEvent::StateChanged(StateChange::TopologyChanged { session_id }) => {
                    self.handle_topology_changed(session_id);
                }
                CustomEvent::StateChanged(StateChange::DiffUpdated { session_id, diffs }) => {
                    let mut session_ctx = self.context.session.write();
                    session_ctx
                        .session_diff
                        .insert(session_id.clone(), diffs.clone());
                    drop(session_ctx);
                }
                CustomEvent::StateChanged(StateChange::OutputBlock {
                    session_id,
                    id,
                    payload,
                }) => {
                    let current_session = self.current_session_id();
                    let is_active_session = current_session.as_deref() == Some(session_id.as_str());
                    let current_is_parent_of_target =
                        current_session.as_deref().is_some_and(|active| {
                            let session_ctx = self.context.session.read();
                            session_ctx
                                .sessions
                                .get(session_id)
                                .and_then(|session| session.parent_id.as_deref())
                                == Some(active)
                        });

                    {
                        let mut session_ctx = self.context.session.write();
                        session_ctx.apply_output_block_incremental(
                            session_id,
                            id.as_deref(),
                            payload,
                        );
                    }
                    if payload.get("kind").and_then(|value| value.as_str())
                        == Some("scheduler_stage")
                    {
                        if let Ok(block) = serde_json::from_value::<
                            rocode_command::output_blocks::SchedulerStageBlock,
                        >(payload.clone())
                        {
                            self.context
                                .apply_scheduler_stage_summary(session_id, &block);
                        }
                    }

                    if let Route::Session { session_id: active } = self.context.current_route() {
                        if active == *session_id {
                            if payload.get("kind").and_then(|value| value.as_str())
                                == Some("scheduler_stage")
                            {
                                self.refresh_attached_sessions();
                            }
                            self.event_caused_change = true;
                        }
                    }

                    if current_is_parent_of_target {
                        self.refresh_attached_sessions();
                        self.event_caused_change = true;
                    }

                    if is_active_session && self.status_dialog.is_open() {
                        self.refresh_active_status_dialog();
                    }
                }
                _ => {}
            },
            Event::Tick => {
                let now = Instant::now();
                let delta_ms = now
                    .saturating_duration_since(self.sync_runtime.last_tick_at)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                self.sync_runtime.last_tick_at = now;
                let mut tick_changed = false;
                tick_changed |= self.toast.tick(delta_ms);
                tick_changed |= self.prompt.tick_spinner(delta_ms);
                tick_changed |= self.sync_prompt_spinner_state();

                if self.sync_runtime.pending_initial_submit
                    && !self.prompt.get_input().trim().is_empty()
                {
                    self.sync_runtime.pending_initial_submit = false;
                    self.submit_prompt()?;
                    tick_changed = true;
                }

                let route = self.context.current_route();
                if let Route::Session { session_id } = &route {
                    let should_sync_pending = self.sync_runtime.pending_session_sync.as_deref()
                        == Some(session_id.as_str())
                        && self
                            .sync_runtime
                            .pending_session_sync_due_at
                            .map(|due| Instant::now() >= due)
                            .unwrap_or(false);
                    if should_sync_pending {
                        let sync_result = self
                            .sync_session_from_server_with_mode(
                                session_id,
                                SessionSyncMode::Incremental,
                            )
                            .or_else(|_| {
                                self.sync_session_from_server_with_mode(
                                    session_id,
                                    SessionSyncMode::Full,
                                )
                            });
                        if sync_result.is_ok() {
                            tick_changed = true;
                            self.check_scheduler_handoff(session_id);
                            self.refresh_attached_sessions();
                            if self.status_dialog.is_open() {
                                self.refresh_active_status_dialog();
                            }
                        }
                        self.sync_runtime.pending_session_sync = None;
                        self.sync_runtime.pending_session_sync_due_at = None;
                    }
                    if self.sync_runtime.last_full_session_sync.elapsed()
                        >= Duration::from_secs(SESSION_FULL_SYNC_INTERVAL_SECS)
                        && self
                            .sync_session_from_server_with_mode(session_id, SessionSyncMode::Full)
                            .is_ok()
                    {
                        tick_changed = true;
                        self.refresh_attached_sessions();
                        if self.status_dialog.is_open() {
                            self.refresh_active_status_dialog();
                        }
                    }
                }
                if self.sync_runtime.last_question_sync.elapsed()
                    >= Duration::from_secs(QUESTION_SYNC_FALLBACK_SECS)
                {
                    tick_changed |= self.sync_question_requests();
                    self.sync_runtime.last_question_sync = Instant::now();
                }
                if self.sync_runtime.last_permission_sync.elapsed()
                    >= self.permission_sync_interval()
                {
                    tick_changed |= self.sync_permission_requests();
                    self.sync_runtime.last_permission_sync = Instant::now();
                }
                if self.sync_runtime.last_aux_sync.elapsed() >= self.aux_sync_interval() {
                    if self.session_list_dialog.is_open() {
                        self.refresh_session_list_dialog();
                    }
                    if self.skill_list_dialog.is_open() {
                        let _ = self.refresh_skill_list_dialog();
                    }
                    let _ = self.refresh_lsp_status();
                    let _ = self.refresh_mcp_dialog();
                    self.sync_runtime.last_aux_sync = Instant::now();
                    tick_changed = true;
                }
                if self.sync_runtime.last_process_refresh.elapsed() >= Duration::from_secs(2) {
                    let should_refresh_processes =
                        matches!(route, Route::Session { .. }) && self.session_sidebar_visible();
                    if should_refresh_processes {
                        rocode_core::process_registry::global_registry().refresh_stats();
                        *self.context.processes.write() =
                            rocode_core::process_registry::global_registry().list();
                        tick_changed = true;
                    }
                    self.sync_runtime.last_process_refresh = Instant::now();
                }
                tick_changed |= self.sync_ui_bridge_health();
                self.maybe_log_perf_snapshot();
                self.event_caused_change = tick_changed;
            }
            _ => {}
        }

        Ok(())
    }

    fn terminal_width(&self) -> u16 {
        self.viewport_area.width
    }

    fn permission_interaction_active(&self) -> bool {
        !self.permission_runtime.pending_ids.is_empty() || self.permission_prompt.is_open
    }

    fn permission_sync_interval(&self) -> Duration {
        if self.permission_interaction_active() {
            Duration::from_secs(PERMISSION_SYNC_BACKOFF_SECS)
        } else {
            Duration::from_secs(PERMISSION_SYNC_FALLBACK_SECS)
        }
    }

    fn aux_sync_interval(&self) -> Duration {
        if self.permission_interaction_active() {
            Duration::from_secs(AUX_SYNC_BACKOFF_SECS)
        } else {
            Duration::from_secs(AUX_SYNC_INTERVAL_SECS)
        }
    }

    fn session_sidebar_visible(&self) -> bool {
        self.context
            .session_view_handle()
            .map(|sv| sv.sidebar_visible(self.terminal_width()))
            .unwrap_or(false)
    }

    fn toggle_session_sidebar(&self) {
        if let Some(sv) = self.context.session_view_handle() {
            sv.toggle_sidebar(self.terminal_width());
        }
    }

    fn maybe_log_perf_snapshot(&mut self) {
        if self.sync_runtime.last_perf_log.elapsed() < Duration::from_secs(PERF_LOG_INTERVAL_SECS) {
            return;
        }
        self.sync_runtime.last_perf_log = Instant::now();
        let ui_bridge = self.context.ui_bridge_snapshot();
        if self.diagnostics.perf_log_info {
            tracing::info!(
                draws = self.diagnostics.perf.draws,
                screen_snapshots = self.diagnostics.perf.screen_snapshots,
                session_sync_full = self.diagnostics.perf.session_sync_full,
                session_sync_incremental = self.diagnostics.perf.session_sync_incremental,
                question_sync = self.diagnostics.perf.question_sync,
                session_updated_events = self.diagnostics.perf.session_updated_events,
                ui_bridge_pending = ui_bridge.pending_events,
                ui_bridge_high_water = ui_bridge.high_water_mark,
                ui_bridge_coalesced = ui_bridge.coalesced_events,
                ui_bridge_dropped = ui_bridge.dropped_events,
                ui_bridge_capacity = ui_bridge.capacity,
                "tui perf snapshot"
            );
        } else {
            tracing::debug!(
                draws = self.diagnostics.perf.draws,
                screen_snapshots = self.diagnostics.perf.screen_snapshots,
                session_sync_full = self.diagnostics.perf.session_sync_full,
                session_sync_incremental = self.diagnostics.perf.session_sync_incremental,
                question_sync = self.diagnostics.perf.question_sync,
                session_updated_events = self.diagnostics.perf.session_updated_events,
                ui_bridge_pending = ui_bridge.pending_events,
                ui_bridge_high_water = ui_bridge.high_water_mark,
                ui_bridge_coalesced = ui_bridge.coalesced_events,
                ui_bridge_dropped = ui_bridge.dropped_events,
                ui_bridge_capacity = ui_bridge.capacity,
                "tui perf snapshot"
            );
        }
    }

    pub(crate) fn next_tick_deadline(&self, now: Instant) -> Option<Instant> {
        let mut deadline = None;

        let mut schedule_at = |candidate: Instant| match deadline {
            Some(current) if current <= candidate => {}
            _ => deadline = Some(candidate),
        };

        let mut schedule_after_last_tick = |delta: Duration| {
            schedule_at(self.sync_runtime.last_tick_at + delta);
        };

        if let Some(delta) = self.toast.next_tick_after() {
            schedule_after_last_tick(delta);
        }
        if let Some(delta) = self
            .prompt
            .next_tick_after(now, self.sync_runtime.last_tick_at)
        {
            schedule_at(now + delta);
        }

        if self.sync_runtime.pending_initial_submit && !self.prompt.get_input().trim().is_empty() {
            schedule_at(now);
        }

        let route = self.context.current_route();
        if let Route::Session { session_id } = &route {
            if self.sync_runtime.pending_session_sync.as_deref() == Some(session_id.as_str()) {
                if let Some(due_at) = self.sync_runtime.pending_session_sync_due_at {
                    schedule_at(due_at);
                }
            }

            schedule_at(
                self.sync_runtime.last_full_session_sync
                    + Duration::from_secs(SESSION_FULL_SYNC_INTERVAL_SECS),
            );

            if self.session_sidebar_visible() {
                schedule_at(self.sync_runtime.last_process_refresh + Duration::from_secs(2));
            }
        }

        schedule_at(
            self.sync_runtime.last_question_sync + Duration::from_secs(QUESTION_SYNC_FALLBACK_SECS),
        );
        schedule_at(self.sync_runtime.last_permission_sync + self.permission_sync_interval());
        schedule_at(self.sync_runtime.last_aux_sync + self.aux_sync_interval());
        schedule_at(self.sync_runtime.last_perf_log + Duration::from_secs(PERF_LOG_INTERVAL_SECS));

        deadline
    }

    fn sync_ui_bridge_health(&mut self) -> bool {
        let ui_bridge = self.context.ui_bridge_snapshot();
        let previous_dropped = self.sync_runtime.last_ui_bridge_dropped_events;
        self.sync_runtime.last_ui_bridge_dropped_events = ui_bridge.dropped_events;
        if ui_bridge.dropped_events <= previous_dropped {
            return false;
        }

        let dropped_delta = ui_bridge.dropped_events.saturating_sub(previous_dropped);
        self.toast.show(
            ToastVariant::Warning,
            &format!(
                "TUI event stream lagged; dropped {} queued update{}. Open /runtime for queue stats.",
                dropped_delta,
                if dropped_delta == 1 { "" } else { "s" }
            ),
            4200,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{MessageTokensInfo, SessionTimeInfo};
    use chrono::Utc;

    #[test]
    fn session_update_requires_sync_for_prompt_final_sources() {
        assert!(session_update_requires_sync(Some("prompt.final")));
        assert!(session_update_requires_sync(Some("prompt.completed")));
        assert!(session_update_requires_sync(Some(
            "prompt.scheduler.completed"
        )));
        assert!(!session_update_requires_sync(Some("prompt.stream")));
        assert!(session_update_requires_sync(Some(
            "prompt.scheduler.stage.step"
        )));
        assert!(session_update_requires_sync(Some(
            "prompt.scheduler.stage.usage"
        )));
        assert!(!session_update_requires_sync(Some(
            "prompt.scheduler.stage.reasoning"
        )));
    }

    #[test]
    fn incremental_session_sync_refreshes_title_and_revert_metadata() {
        let now = Utc::now().timestamp_millis();
        let session_id = "session-1";
        let mut session_ctx = crate::context::SessionContext::new();
        session_ctx.upsert_session(Session {
            id: session_id.to_string(),
            title: "New Session".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            parent_id: None,
            share: None,
            metadata: None,
        });
        session_ctx.set_messages(
            session_id,
            vec![Message {
                id: "m1".to_string(),
                role: MessageRole::User,
                content: "hello".to_string(),
                created_at: Utc::now(),
                agent: None,
                model: None,
                mode: None,
                finish: None,
                error: None,
                completed_at: None,
                cost: 0.0,
                tokens: TokenUsage::default(),
                metadata: None,
                multimodal: None,
                parts: vec![ContextMessagePart::Text {
                    text: "hello".to_string(),
                }],
            }],
        );

        let session = SessionInfo {
            id: session_id.to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: ".".to_string(),
            parent_id: None,
            title: "Greeting Session".to_string(),
            version: "1".to_string(),
            time: SessionTimeInfo {
                created: now,
                updated: now + 1000,
                compacting: None,
                archived: None,
            },
            summary: None,
            share: None,
            permission: None,
            revert: Some(SessionRevertInfo {
                message_id: "m2".to_string(),
                part_id: Some("p1".to_string()),
                snapshot: Some("snapshot".to_string()),
                diff: None,
            }),
            fork: None,
            telemetry: None,
            metadata: None,
        };
        let mapped_messages = vec![map_api_message(&MessageInfo {
            id: "m2".to_string(),
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            created_at: now + 500,
            completed_at: None,
            agent: None,
            model: None,
            mode: None,
            finish: Some("stop".to_string()),
            error: None,
            cost: 0.0,
            tokens: MessageTokensInfo::default(),
            parts: vec![crate::api::MessagePart {
                id: "p1".to_string(),
                part_type: "text".to_string(),
                text: Some("world".to_string()),
                file: None,
                tool_call: None,
                tool_result: None,
                synthetic: None,
                ignored: None,
            }],
            metadata: None,
            multimodal: None,
        })];

        apply_incremental_session_sync(&mut session_ctx, session_id, &session, mapped_messages);

        assert_eq!(
            session_ctx
                .sessions
                .get(session_id)
                .map(|session| session.title.as_str()),
            Some("Greeting Session")
        );
        assert_eq!(
            session_ctx
                .messages
                .get(session_id)
                .map(|messages| messages.len()),
            Some(2)
        );
        assert_eq!(
            session_ctx
                .revert
                .get(session_id)
                .map(|revert| revert.message_id.as_str()),
            Some("m2")
        );
    }

    #[test]
    fn question_prompt_at_appends_other_option_once() {
        let prompt = App::question_prompt_at(
            &QuestionInfo {
                id: "q1".to_string(),
                session_id: "s1".to_string(),
                questions: vec!["Pick one".to_string()],
                options: Some(vec![vec!["Yes".to_string(), "No".to_string()]]),
                items: Vec::new(),
            },
            0,
        )
        .expect("prompt should exist");

        assert_eq!(prompt.question_type, QuestionType::SingleChoice);
        assert_eq!(
            prompt.options.last().map(|option| option.id.as_str()),
            Some(OTHER_OPTION_ID)
        );
        assert_eq!(
            prompt.options.last().map(|option| option.label.as_str()),
            Some(OTHER_OPTION_LABEL)
        );
        assert_eq!(
            prompt
                .options
                .iter()
                .filter(|option| option.id == OTHER_OPTION_ID)
                .count(),
            1
        );
    }

    #[test]
    fn diff_updated_event_populates_session_diff() {
        use crate::context::DiffEntry;

        let session_id = "session-diff-test";
        let mut session_ctx = crate::context::SessionContext::new();

        // Simulate what the DiffUpdated event handler does in app.rs
        let diffs = vec![
            DiffEntry {
                file: "src/main.rs".to_string(),
                additions: 10,
                deletions: 3,
            },
            DiffEntry {
                file: "src/lib.rs".to_string(),
                additions: 5,
                deletions: 0,
            },
        ];
        session_ctx
            .session_diff
            .insert(session_id.to_string(), diffs);

        // Verify the data is stored correctly
        let stored = session_ctx.session_diff.get(session_id).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].file, "src/main.rs");
        assert_eq!(stored[0].additions, 10);
        assert_eq!(stored[0].deletions, 3);
        assert_eq!(stored[1].file, "src/lib.rs");
        assert_eq!(stored[1].additions, 5);
        assert_eq!(stored[1].deletions, 0);
    }

    #[test]
    fn map_api_diff_converts_correctly() {
        use crate::api::ApiDiffEntry;

        let api_diff = ApiDiffEntry {
            path: "src/foo.rs".to_string(),
            additions: 42,
            deletions: 7,
        };
        let mapped = map_api_diff(&api_diff);
        assert_eq!(mapped.file, "src/foo.rs");
        assert_eq!(mapped.additions, 42);
        assert_eq!(mapped.deletions, 7);
    }

    #[test]
    fn map_api_todo_converts_status_strings() {
        use crate::api::ApiTodoItem;
        use crate::context::TodoStatus;

        let cases = vec![
            ("pending", TodoStatus::Pending),
            ("in_progress", TodoStatus::InProgress),
            ("completed", TodoStatus::Completed),
            ("done", TodoStatus::Completed),
            ("cancelled", TodoStatus::Cancelled),
            ("canceled", TodoStatus::Cancelled),
            ("unknown_status", TodoStatus::Pending),
        ];

        for (status_str, expected) in cases {
            let api_item = ApiTodoItem {
                id: "t1".to_string(),
                content: "Test".to_string(),
                status: status_str.to_string(),
                priority: "medium".to_string(),
            };
            let mapped = map_api_todo(&api_item);
            assert_eq!(
                std::mem::discriminant(&mapped.status),
                std::mem::discriminant(&expected),
                "Status '{}' should map to {:?}",
                status_str,
                expected
            );
        }
    }

    #[test]
    fn dialog_left_click_is_consumed_without_closing_dialog() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new().expect("app should initialize");
        app.open_model_select_dialog();

        let consumed = app
            .handle_dialog_mouse(&MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 10,
                row: 10,
                modifiers: KeyModifiers::empty(),
            })
            .expect("mouse event should be handled");

        assert!(consumed);
        assert!(app.model_select.is_open());
        assert!(!app.event_caused_change);
    }

    #[test]
    fn exit_summary_uses_current_cli_session_command() {
        let app = App::new().expect("app should initialize");
        let now = Utc::now();
        let session_id = "ses_continue_test";
        {
            let mut session_ctx = app.context.session.write();
            session_ctx.upsert_session(Session {
                id: session_id.to_string(),
                title: "Continue Test".to_string(),
                created_at: now,
                updated_at: now,
                parent_id: None,
                share: None,
                metadata: None,
            });
        }
        app.context.navigate_session(session_id);

        let summary = app.exit_summary().expect("exit summary");
        assert!(summary.contains("rocode tui -s ses_continue_test"));
        assert!(!summary.contains("rocode -s ses_continue_test"));
        assert!(!summary.contains("rocode run -s ses_continue_test"));
    }

    #[test]
    fn refresh_attached_sessions_uses_parent_graph_for_child_route() {
        let app = App::new().expect("app should initialize");
        let now = Utc::now();
        let parent_id = "parent-session";
        let attached_id = "attached-session";

        let mut metadata = HashMap::new();
        metadata.insert(
            "scheduler_stage_attached_session_id".to_string(),
            serde_json::json!(attached_id),
        );
        metadata.insert("scheduler_stage".to_string(), serde_json::json!("review"));
        metadata.insert(
            "scheduler_stage_title".to_string(),
            serde_json::json!("Review"),
        );
        metadata.insert(
            "scheduler_stage_status".to_string(),
            serde_json::json!("running"),
        );

        {
            let mut session_ctx = app.context.session.write();
            session_ctx.upsert_session(Session {
                id: parent_id.to_string(),
                title: "Parent".to_string(),
                created_at: now,
                updated_at: now,
                parent_id: None,
                share: None,
                metadata: None,
            });
            session_ctx.upsert_session(Session {
                id: attached_id.to_string(),
                title: "Child".to_string(),
                created_at: now,
                updated_at: now,
                parent_id: Some(parent_id.to_string()),
                share: None,
                metadata: None,
            });
            session_ctx.set_messages(
                parent_id,
                vec![Message {
                    id: "stage-message".to_string(),
                    role: MessageRole::Assistant,
                    content: String::new(),
                    created_at: now,
                    agent: None,
                    model: None,
                    mode: None,
                    finish: None,
                    error: None,
                    completed_at: None,
                    cost: 0.0,
                    tokens: TokenUsage::default(),
                    metadata: Some(metadata),
                    multimodal: None,
                    parts: vec![ContextMessagePart::Text {
                        text: String::new(),
                    }],
                }],
            );
            session_ctx.set_messages(attached_id, Vec::new());
        }

        app.context.navigate_session(attached_id);
        app.refresh_attached_sessions();

        let children = app.context.attached_sessions();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].session_id, attached_id);
    }

    #[test]
    fn ui_bridge_drop_growth_surfaces_warning_toast() {
        let mut app = App::new().expect("app should initialize");
        app.toast = Toast::new();
        assert!(!app.toast.is_visible());

        app.sync_runtime.last_ui_bridge_dropped_events = 2;
        app.context
            .ui_bridge
            .emit(Event::Custom(Box::new(crate::event::CustomEvent::Message(
                "message-1".to_string(),
            ))));
        app.context
            .ui_bridge
            .emit(Event::Custom(Box::new(crate::event::CustomEvent::Message(
                "message-2".to_string(),
            ))));

        app.context.ui_bridge.drain(1);
        let queue_capacity = app.context.ui_bridge_snapshot().capacity;
        for index in 0..(queue_capacity + 2) {
            app.context.ui_bridge.emit(Event::Custom(Box::new(
                crate::event::CustomEvent::Message(format!("overflow-{index}")),
            )));
        }

        assert!(app.sync_ui_bridge_health());
        assert!(app.toast.is_visible());
        assert_eq!(
            app.sync_runtime.last_ui_bridge_dropped_events,
            app.context.ui_bridge_snapshot().dropped_events
        );
        assert!(!app.sync_ui_bridge_health());
    }
}
