use agendao_command_render::run_status_labels::canonical_run_status_badge;

const SIDEBAR_WIDTH: u16 = 42;
const HEADER_NARROW_THRESHOLD: u16 = 80;
const THINKING_PREVIEW_LINES: usize = 2;
const MOUSE_SCROLL_LINES: usize = 3;
const MESSAGE_BLOCK_RIGHT_PADDING: usize = 1;
const SIDEBAR_CLOSE_BUTTON_WIDTH: u16 = 5;
const SIDEBAR_OPEN_BUTTON_WIDTH: u16 = 5;
const SEMANTIC_HIGHLIGHT_MAX_CHARS: usize = 8_000;

#[derive(Clone, PartialEq, Eq)]
struct ThinkingToggleHit {
    line_index: usize,
    reasoning_id: String,
}

#[derive(Clone, PartialEq, Eq)]
struct ToolArgumentsToggleHit {
    line_index: usize,
    arguments_id: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct SessionMessageViewportState {
    scroll_offset: usize,
    rendered_line_count: usize,
    messages_viewport_height: usize,
    render_model_memo_key: Option<u64>,
    last_messages_area: Option<Rect>,
    last_scrollbar_area: Option<Rect>,
    scrollbar_drag_active: bool,
    message_first_lines: HashMap<String, usize>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct SessionReasoningState {
    expanded: HashSet<String>,
    toggle_hits: Vec<ThinkingToggleHit>,
    expanded_tool_arguments: HashSet<String>,
    tool_arguments_toggle_hits: Vec<ToolArgumentsToggleHit>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct SessionSidebarChromeState {
    lifecycle: SidebarLifecycleState,
    render_state: SidebarRenderState,
    backdrop_area: Option<Rect>,
    close_button_area: Option<Rect>,
    open_button_area: Option<Rect>,
    last_terminal_width: u16,
}

#[derive(Clone, Default)]
struct SessionReactiveBindings {
    viewport: SessionMessageViewportState,
    viewport_setter: Option<StateSetter<SessionMessageViewportState>>,
    reasoning: SessionReasoningState,
    reasoning_setter: Option<StateSetter<SessionReasoningState>>,
    sidebar: SessionSidebarChromeState,
    sidebar_setter: Option<StateSetter<SessionSidebarChromeState>>,
}

#[derive(Clone)]
enum SessionInteractionAction {
    SetScrollOffset(usize),
    SetScrollbarDrag(bool),
    ToggleReasoning(String),
    ToggleToolArguments(String),
}

struct SessionStateBinderComponent {
    bindings: Arc<Mutex<SessionReactiveBindings>>,
    pending_actions: Arc<Mutex<Vec<SessionInteractionAction>>>,
    viewport_seed: SessionMessageViewportState,
    reasoning_seed: SessionReasoningState,
    sidebar_seed: SessionSidebarChromeState,
}

#[derive(Clone, Default)]
struct SessionMessagesOutput {
    viewport: SessionMessageViewportState,
    reasoning: SessionReasoningState,
    message_cache: SessionMessageOutputCache,
    render_model_cache: SessionRenderModelCache,
}

struct SessionMessagesComponent {
    area: Rect,
    snapshot: SessionMessagesSnapshot,
    viewport: SessionMessageViewportState,
    reasoning: SessionReasoningState,
    output: Arc<Mutex<Option<SessionMessagesOutput>>>,
}

struct SessionMessageViewportComponent {
    theme: crate::theme::Theme,
    model: Arc<SessionRenderModel>,
    messages_area: Rect,
    scrollbar_area: Option<Rect>,
    scroll_offset: usize,
    viewport_height: usize,
}

#[derive(Clone)]
struct SessionMessagesSnapshot {
    theme: crate::theme::Theme,
    messages: Vec<Message>,
    revert_info: Option<RevertInfo>,
    directory: String,
    message_density: crate::context::MessageDensity,
    show_scrollbar: bool,
    show_timestamps: bool,
    show_thinking: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
}

#[derive(Clone)]
struct SessionMessagesSnapshotSeed {
    session_id: String,
    theme: crate::theme::Theme,
    messages: Vec<Message>,
    revert_info: Option<RevertInfo>,
    directory: String,
    message_density: crate::context::MessageDensity,
    show_scrollbar: bool,
    show_timestamps: bool,
    show_thinking: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
    status: crate::context::SessionStatus,
    context_compaction_summary: Option<agendao_types::ContextCompactionSummary>,
    context_compaction_lifecycle_summary: Option<agendao_types::ContextCompactionLifecycleSummary>,
}

#[derive(Clone)]
struct SessionMessagesSnapshotKey {
    session_id: String,
    theme_debug: String,
    directory: String,
    message_density: crate::context::MessageDensity,
    show_scrollbar: bool,
    show_timestamps: bool,
    show_thinking: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
    revert_debug: String,
    message_revision: u64,
    status_debug: String,
    context_compaction_summary_debug: String,
    context_compaction_lifecycle_summary_debug: String,
}

impl SessionMessagesSnapshot {
    /// P2-3: max messages rendered in the session viewport.
    /// Derived from agendao_config::RuntimeBudgetConfig.tui_max_viewport_messages (default 200).
    const MAX_VIEWPORT_MESSAGES: usize = 200;

    fn from_seed(seed: &SessionMessagesSnapshotSeed) -> Self {
        let mut messages = seed.messages.clone();
        let compaction = (
            seed.status.clone(),
            seed.context_compaction_summary.clone(),
            seed.context_compaction_lifecycle_summary.clone(),
        );

        if let Some(message) = synthetic_compaction_message(&seed.session_id, compaction) {
            messages.push(message);
        }

        // P2-3: UUID-anchored viewport capping. When messages exceed the
        // budget, keep only the last MAX_VIEWPORT_MESSAGES. The anchor is
        // the count-based slice — if compaction changes the UUID set, we
        // fall back to showing as many as the budget allows.
        let total = messages.len();
        if total > Self::MAX_VIEWPORT_MESSAGES {
            messages = messages.split_off(total.saturating_sub(Self::MAX_VIEWPORT_MESSAGES));
        }

        Self {
            theme: seed.theme.clone(),
            messages,
            revert_info: seed.revert_info.clone(),
            directory: seed.directory.clone(),
            message_density: seed.message_density,
            show_scrollbar: seed.show_scrollbar,
            show_timestamps: seed.show_timestamps,
            show_thinking: seed.show_thinking,
            show_tool_details: seed.show_tool_details,
            semantic_hl: seed.semantic_hl,
            fallback_model: seed.fallback_model.clone(),
        }
    }
}

impl SessionMessagesSnapshotSeed {
    fn capture(context: &Arc<AppContext>, session_id: &str) -> Self {
        let theme = context.theme.read().clone();
        let directory = context.directory.read().clone();
        let message_density = context.message_density();
        let show_scrollbar = context.show_scrollbar_enabled();
        let show_timestamps = context.show_timestamps_enabled();
        let show_thinking = context.show_thinking_enabled();
        let show_tool_details = context.show_tool_details_enabled();
        let semantic_hl = context.semantic_highlight_enabled();
        let fallback_model = context.current_model();
        let context_compaction_summary = context.session_context_compaction_summary_for(session_id);
        let context_compaction_lifecycle_summary =
            context.session_context_compaction_lifecycle_summary_for(session_id);
        let (messages, revert_info, status) = {
            let session_ctx = context.session.read();
            (
                session_ctx
                    .messages
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
                session_ctx.revert.get(session_id).cloned(),
                session_ctx.status(session_id).clone(),
            )
        };

        Self {
            session_id: session_id.to_string(),
            theme,
            messages,
            revert_info,
            directory,
            message_density,
            show_scrollbar,
            show_timestamps,
            show_thinking,
            show_tool_details,
            semantic_hl,
            fallback_model,
            status,
            context_compaction_summary,
            context_compaction_lifecycle_summary,
        }
    }
}

impl SessionMessagesSnapshotKey {
    fn capture(context: &Arc<AppContext>, session_id: &str) -> Self {
        let theme = context.theme.read().clone();
        let directory = context.directory.read().clone();
        let message_density = context.message_density();
        let show_scrollbar = context.show_scrollbar_enabled();
        let show_timestamps = context.show_timestamps_enabled();
        let show_thinking = context.show_thinking_enabled();
        let show_tool_details = context.show_tool_details_enabled();
        let semantic_hl = context.semantic_highlight_enabled();
        let fallback_model = context.current_model();
        let context_compaction_summary = context.session_context_compaction_summary_for(session_id);
        let context_compaction_lifecycle_summary =
            context.session_context_compaction_lifecycle_summary_for(session_id);
        let (revert_info, message_revision, status) = {
            let session_ctx = context.session.read();
            (
                session_ctx.revert.get(session_id).cloned(),
                session_ctx.message_revision(session_id),
                session_ctx.status(session_id).clone(),
            )
        };

        Self {
            session_id: session_id.to_string(),
            theme_debug: format!("{:?}", theme),
            directory,
            message_density,
            show_scrollbar,
            show_timestamps,
            show_thinking,
            show_tool_details,
            semantic_hl,
            fallback_model,
            revert_debug: format!("{:?}", revert_info),
            message_revision,
            status_debug: format!("{:?}", status),
            context_compaction_summary_debug: format!("{:?}", context_compaction_summary),
            context_compaction_lifecycle_summary_debug: format!(
                "{:?}",
                context_compaction_lifecycle_summary
            ),
        }
    }

}

impl PartialEq for SessionMessagesSnapshotKey {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.theme_debug == other.theme_debug
            && self.directory == other.directory
            && self.message_density == other.message_density
            && self.show_scrollbar == other.show_scrollbar
            && self.show_timestamps == other.show_timestamps
            && self.show_thinking == other.show_thinking
            && self.show_tool_details == other.show_tool_details
            && self.semantic_hl == other.semantic_hl
            && self.fallback_model == other.fallback_model
            && self.revert_debug == other.revert_debug
            && self.message_revision == other.message_revision
            && self.status_debug == other.status_debug
            && self.context_compaction_summary_debug == other.context_compaction_summary_debug
            && self.context_compaction_lifecycle_summary_debug
                == other.context_compaction_lifecycle_summary_debug
    }
}

impl Eq for SessionMessagesSnapshotKey {}

fn synthetic_compaction_message(
    session_id: &str,
    compaction: (
        crate::context::SessionStatus,
        Option<agendao_types::ContextCompactionSummary>,
        Option<agendao_types::ContextCompactionLifecycleSummary>,
    ),
) -> Option<Message> {
    let (status, summary, lifecycle) = compaction;
    if !matches!(status, crate::context::SessionStatus::Compacting) {
        return None;
    }

    let mut lines = vec!["Compacting conversation".to_string()];
    lines.push(format!("  {}", compaction_progress_bar()));

    if let Some(status_line) = compaction_status_line(lifecycle.as_ref(), summary.as_ref()) {
        lines.push(format!("  {}", status_line));
    }

    let mut details = Vec::new();
    if let Some(lifecycle) = lifecycle.as_ref() {
        if let Some(reason) = lifecycle.reason.as_deref().filter(|value| !value.trim().is_empty()) {
            details.push(reason.replace('_', " "));
        }
        if let Some(phase) = lifecycle.phase.as_deref().filter(|value| !value.trim().is_empty()) {
            details.push(phase.replace('_', " "));
        }
        if let Some(limit) = lifecycle.limit_tokens {
            let used = lifecycle
                .request_context_tokens
                .or(lifecycle.live_context_tokens)
                .unwrap_or(0);
            let percent = agendao_types::context_usage_percent(used, limit).unwrap_or(0);
            details.push(format!(
                "{}/{} {}%",
                compact_number(used),
                compact_number(limit),
                percent
            ));
        }
    } else if let Some(summary) = summary.as_ref() {
        if let Some(reason) = summary.reason.as_deref().filter(|value| !value.trim().is_empty()) {
            details.push(reason.replace('_', " "));
        }
        if let Some(limit) = summary.limit_tokens {
            let used = summary
                .request_context_tokens
                .or(summary.live_context_tokens)
                .unwrap_or(0);
            let percent = agendao_types::context_usage_percent(used, limit).unwrap_or(0);
            details.push(format!(
                "{}/{} {}%",
                compact_number(used),
                compact_number(limit),
                percent
            ));
        }
    }

    if !details.is_empty() {
        lines.push(format!("  {}", details.join(" · ")));
    }

    Some(Message {
        id: format!("__compaction__:{}", session_id),
        role: crate::context::MessageRole::System,
        content: lines.join("\n"),
        created_at: chrono::Utc::now(),
        agent: None,
        model: None,
        mode: None,
        finish: None,
        error: None,
        completed_at: None,
        cost: 0.0,
        tokens: crate::context::TokenUsage::default(),
        metadata: None,
        multimodal: None,
        parts: Vec::new(),
    })
}

fn compaction_progress_bar() -> String {
    format!("{} ...", "▰".repeat(12) + &"▱".repeat(28))
}

fn compaction_status_line(
    lifecycle: Option<&agendao_types::ContextCompactionLifecycleSummary>,
    summary: Option<&agendao_types::ContextCompactionSummary>,
) -> Option<String> {
    let used = lifecycle
        .and_then(|lifecycle| {
            lifecycle
                .request_context_tokens
                .or(lifecycle.live_context_tokens)
        })
        .or_else(|| summary.and_then(|summary| summary.request_context_tokens.or(summary.live_context_tokens)));

    let body_chars = lifecycle
        .and_then(|lifecycle| lifecycle.body_chars)
        .or_else(|| summary.and_then(|summary| summary.body_chars));

    let message_count = summary.and_then(|summary| summary.message_count_before);

    let mut parts = Vec::new();
    if let Some(message_count) = message_count {
        parts.push(format!("compressing {message_count} messages"));
    } else if used.is_some() || body_chars.is_some() {
        parts.push("compressing conversation".to_string());
    }
    if let Some(used) = used {
        parts.push(format!("~{} tok", compact_number(used)));
    }
    if let Some(body_chars) = body_chars {
        parts.push(format!("{} chars", compact_number(body_chars as u64)));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        let compact = value as f64 / 1_000_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        };
    }
    if value >= 1_000 {
        let compact = value as f64 / 1_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        };
    }
    value.to_string()
}

#[derive(Clone)]
struct SessionHeaderSnapshot {
    parent_title: Option<String>,
    title: String,
    subtitle: Option<String>,
    usage: Option<String>,
    status_label: Option<String>,
    status_running: bool,
    status_retrying: bool,
}

#[derive(Clone)]
struct SessionRenderSnapshot {
    theme: crate::theme::Theme,
    show_header: bool,
    header: SessionHeaderSnapshot,
}

#[derive(Clone)]
struct SessionRenderSnapshotSeed {
    theme: crate::theme::Theme,
    show_header: bool,
    selection: crate::context::SelectionState,
    session: Option<crate::context::Session>,
    parent_title: Option<String>,
    last_assistant_tokens: Option<crate::context::TokenUsage>,
    fallback_total_cost: f64,
    fallback_total_tokens: u64,
    status: crate::context::SessionStatus,
    session_usage: Option<agendao_types::SessionUsage>,
    pending_permission: bool,
    pending_question: bool,
    tail_status: Option<String>,
}

impl SessionRenderSnapshot {
    fn from_seed(seed: &SessionRenderSnapshotSeed) -> Self {
        let title = seed
            .session
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "New Session".to_string());
        let subtitle = build_session_header_subtitle(
            seed.session
                .as_ref()
                .and_then(|session| session.metadata.as_ref()),
            &seed.selection,
        );
        let total_cost = seed
            .session_usage
            .as_ref()
            .map(|usage| usage.total_cost)
            .unwrap_or(seed.fallback_total_cost);
        let usage = seed.last_assistant_tokens.as_ref().and_then(|_tokens| {
            let total_tokens = seed
                .session_usage
                .as_ref()
                .map(total_session_tokens)
                .unwrap_or(seed.fallback_total_tokens);
            if total_tokens == 0 {
                return None;
            }

            let mut parts = Vec::new();
            parts.push(format!("session {} total", format_compact_number(total_tokens)));
            parts.push(format!("${:.4}", total_cost));
            Some(parts.join("  ·  "))
        });

        let (status_label, status_running, status_retrying) = if seed.pending_permission {
            (
                Some(canonical_run_status_badge("awaiting_permission").to_string()),
                false,
                false,
            )
        } else if seed.pending_question {
            (
                Some(canonical_run_status_badge("awaiting_user").to_string()),
                false,
                false,
            )
        } else {
            match &seed.status {
                crate::context::SessionStatus::Running => (
                    Some(canonical_run_status_badge("running").to_string()),
                    true,
                    false,
                ),
                crate::context::SessionStatus::Compacting => (
                    Some(canonical_run_status_badge("compacting").to_string()),
                    true,
                    false,
                ),
                crate::context::SessionStatus::Reconnecting => (
                    Some(canonical_run_status_badge("reconnecting").to_string()),
                    true,
                    false,
                ),
                crate::context::SessionStatus::WaitingOnUser => (
                    Some(canonical_run_status_badge("waiting_on_user").to_string()),
                    false,
                    false,
                ),
                crate::context::SessionStatus::Retrying { attempt, .. } => (
                    Some(format!(
                        "{} {}",
                        canonical_run_status_badge("retrying"),
                        attempt
                    )),
                    true,
                    true,
                ),
                crate::context::SessionStatus::Idle => (
                    Some(
                        seed.tail_status
                            .as_deref()
                            .map(|status| canonical_run_status_badge(status).to_string())
                            .unwrap_or_else(|| canonical_run_status_badge("idle").to_string()),
                    ),
                    false,
                    false,
                ),
            }
        };

        Self {
            theme: seed.theme.clone(),
            show_header: seed.show_header,
            header: SessionHeaderSnapshot {
                parent_title: seed.parent_title.clone(),
                title,
                subtitle,
                usage,
                status_label,
                status_running,
                status_retrying,
            },
        }
    }
}

impl SessionRenderSnapshotSeed {
    fn capture(context: &Arc<AppContext>, session_id: &str) -> Self {
        let theme = context.theme.read().clone();
        let show_header = context.show_header_enabled();
        let selection = context.selection_state();
        let session_usage = context.session_usage_for(session_id);
        let pending_permission = context.get_pending_permission_for(session_id).is_some();
        let pending_question = context.has_pending_question_for(session_id);
        let tail_status = context.session_terminal_tail_status(session_id);
        let (session, parent_title, last_assistant_tokens, fallback_total_cost, fallback_total_tokens, status) = {
            let session_ctx = context.session.read();
            let session = session_ctx.sessions.get(session_id).cloned();
            let parent_title = session
                .as_ref()
                .and_then(|session| session.parent_id.as_ref())
                .and_then(|parent_id| session_ctx.sessions.get(parent_id))
                .map(|session| session.title.clone());
            let messages = session_ctx.messages.get(session_id);
            let last_assistant_tokens = messages.and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|m| matches!(m.role, MessageRole::Assistant) && m.tokens.output > 0)
                    .map(|m| m.tokens.clone())
            });
            let fallback_total_cost = messages
                .map(|messages| {
                    messages
                        .iter()
                        .filter(|m| matches!(m.role, MessageRole::Assistant))
                        .map(|m| m.cost)
                        .sum()
                })
                .unwrap_or(0.0);
            let fallback_total_tokens = messages
                .and_then(|messages| {
                    messages
                        .iter()
                        .rev()
                        .find(|m| matches!(m.role, MessageRole::Assistant) && m.tokens.output > 0)
                        .map(|m| {
                            let t = &m.tokens;
                            t.input + t.output + t.reasoning
                        })
                })
                .unwrap_or(0);
            let status = session_ctx
                .session_status
                .get(session_id)
                .cloned()
                .unwrap_or_default();
            (
                session,
                parent_title,
                last_assistant_tokens,
                fallback_total_cost,
                fallback_total_tokens,
                status,
            )
        };

        Self {
            theme,
            show_header,
            selection,
            session,
            parent_title,
            last_assistant_tokens,
            fallback_total_cost,
            fallback_total_tokens,
            status,
            session_usage,
            pending_permission,
            pending_question,
            tail_status,
        }
    }
}

fn build_session_header_subtitle(
    metadata: Option<&HashMap<String, serde_json::Value>>,
    selection: &crate::context::SelectionState,
) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(metadata) = metadata {
        if let Some(agent) = super::sidebar::sidebar_metadata_text(metadata, "agent") {
            parts.push(agent);
        }
        if let Some(model) = super::sidebar::sidebar_model_summary(metadata) {
            parts.push(model);
        }
        if let Some(scheduler) = super::sidebar::sidebar_scheduler_summary(metadata) {
            parts.push(scheduler);
        }
    }

    if parts.is_empty() {
        if !selection.current_agent.is_empty() {
            parts.push(selection.current_agent.clone());
        }
        if let Some(model) = selection_model_summary(selection) {
            parts.push(model);
        }
        if let Some(profile) = selection.current_scheduler_profile.as_ref() {
            parts.push(format!("scheduler {}", profile));
        }
    }

    (!parts.is_empty()).then(|| parts.join("  ·  "))
}

fn selection_model_summary(selection: &crate::context::SelectionState) -> Option<String> {
    match (
        selection.current_provider.as_ref(),
        selection.current_model.as_ref(),
    ) {
        (Some(provider), Some(model)) => Some(format!("{}/{}", provider, model)),
        (None, Some(model)) => Some(model.clone()),
        _ => None,
    }
}

fn session_sidebar_visible(lifecycle: &SidebarLifecycleState, terminal_width: u16) -> bool {
    match lifecycle.mode {
        SidebarMode::Hide => false,
        SidebarMode::Show => true,
        SidebarMode::Auto => {
            terminal_width > crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD || lifecycle.visible
        }
    }
}

enum SessionSidebarLayout {
    Docked { sidebar_area: Rect },
    Overlay,
    Hidden,
}

struct SessionRenderLayout {
    main_area: Rect,
    sidebar: SessionSidebarLayout,
}

struct MainPaneLayout {
    header_area: Rect,
    messages_area: Rect,
    prompt_area: Rect,
    show_header: bool,
    show_prompt: bool,
}

struct SessionRenderResources<'a> {
    theme: crate::theme::Theme,
    messages: &'a [Message],
    terminal_messages: &'a [TerminalMessage],
    revert_info: Option<crate::context::RevertInfo>,
    directory: String,
    content_width: usize,
    show_thinking: bool,
    show_timestamps: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
    user_bg: Color,
    thinking_bg: Color,
    assistant_border: Color,
    thinking_border: Color,
    message_gap_lines: usize,
}

#[derive(PartialEq, Eq)]
struct SessionRenderChunk {
    start_line: usize,
    end_line: usize,
    lines: Arc<Vec<Line<'static>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct VisibleChunkRange {
    chunk_index: usize,
    start_in_chunk: usize,
    end_in_chunk: usize,
}

#[derive(Default)]
struct SessionRenderBuffer {
    chunks: Vec<SessionRenderChunk>,
    rendered_line_count: usize,
    message_first_lines: HashMap<String, usize>,
}

impl SessionRenderBuffer {
    fn line_count(&self) -> usize {
        self.rendered_line_count
    }

    fn record_message_start(&mut self, message_id: &str) {
        self.message_first_lines
            .entry(message_id.to_string())
            .or_insert(self.rendered_line_count);
    }

    fn append_message(&mut self, _message_id: &str, lines: Arc<Vec<Line<'static>>>) {
        let line_count = lines.len();
        let start_line = self.rendered_line_count;
        let end_line = start_line.saturating_add(line_count);
        self.chunks.push(SessionRenderChunk {
            start_line,
            end_line,
            lines,
        });
        self.rendered_line_count = end_line;
    }

    fn append_non_message(&mut self, lines: Arc<Vec<Line<'static>>>) {
        let line_count = lines.len();
        let start_line = self.rendered_line_count;
        let end_line = start_line.saturating_add(line_count);
        self.chunks.push(SessionRenderChunk {
            start_line,
            end_line,
            lines,
        });
        self.rendered_line_count = end_line;
    }

    fn push_spacing(&mut self, count: usize, bg: Color, width: usize) {
        let lines = shared_lines(build_spacing_lines(count, bg, width));
        let line_count = lines.len();
        let start_line = self.rendered_line_count;
        let end_line = start_line.saturating_add(line_count);
        self.chunks.push(SessionRenderChunk {
            start_line,
            end_line,
            lines,
        });
        self.rendered_line_count = end_line;
    }
}

#[derive(PartialEq, Eq)]
struct SessionRenderModel {
    memo_key: u64,
    chunks: Vec<SessionRenderChunk>,
    rendered_line_count: usize,
    message_first_lines: HashMap<String, usize>,
    toggle_hits: Vec<ThinkingToggleHit>,
    tool_arguments_toggle_hits: Vec<ToolArgumentsToggleHit>,
    visible_reasoning_ids: HashSet<String>,
    visible_tool_arguments_ids: HashSet<String>,
}

#[derive(Clone)]
struct AssistantSegmentRenderOutput {
    lines: Vec<Line<'static>>,
    toggle_line_offsets: Vec<ThinkingToggleHitOffset>,
    tool_arguments_toggle_line_offsets: Vec<ToolArgumentsToggleHitOffset>,
    visible_reasoning_ids: HashSet<String>,
    visible_tool_arguments_ids: HashSet<String>,
}

#[derive(Clone)]
enum AssistantMessageItem {
    Spacer,
    Text(AssistantTextItem),
    Thinking(AssistantThinkingItem),
    Tool(AssistantToolBlockItem),
    File(AssistantFileItem),
    Image(AssistantImageItem),
    Footer(AssistantFooterItem),
}

fn build_spacing_lines(count: usize, bg: Color, width: usize) -> Vec<Line<'static>> {
    let spacing = Line::from(Span::styled(" ".repeat(width), Style::default().bg(bg)));
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        lines.push(spacing.clone());
    }
    lines
}

fn shared_lines(lines: Vec<Line<'static>>) -> Arc<Vec<Line<'static>>> {
    Arc::new(lines)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SessionRenderPerfCounters {
    render_model_cache_hits: usize,
    render_model_rebuilds: usize,
    message_cache_hits: usize,
    message_cache_misses: usize,
    visible_range_recomputes: usize,
    visible_lines_written: usize,
}

#[cfg(test)]
thread_local! {
    static SESSION_RENDER_PERF_COUNTERS: RefCell<SessionRenderPerfCounters> =
        RefCell::new(SessionRenderPerfCounters::default());
}

#[cfg(test)]
fn record_session_render_perf(mut update: impl FnMut(&mut SessionRenderPerfCounters)) {
    SESSION_RENDER_PERF_COUNTERS.with(|counters| update(&mut counters.borrow_mut()));
}

#[cfg(not(test))]
// Render perf counters are test-only instrumentation. Non-test builds compile
// the call sites down to a no-op so the main TUI render path stays allocation-
// free and branch-light.
fn record_session_render_perf(_update: impl FnMut(&mut SessionRenderPerfCounters)) {}

#[cfg(test)]
fn reset_session_render_perf_counters() {
    SESSION_RENDER_PERF_COUNTERS
        .with(|counters| *counters.borrow_mut() = SessionRenderPerfCounters::default());
}

#[cfg(test)]
fn snapshot_session_render_perf_counters() -> SessionRenderPerfCounters {
    SESSION_RENDER_PERF_COUNTERS.with(|counters| counters.borrow().clone())
}

fn collect_visible_chunk_ranges(
    chunks: &[SessionRenderChunk],
    scroll_offset: usize,
    viewport_height: usize,
) -> Vec<VisibleChunkRange> {
    record_session_render_perf(|counters| counters.visible_range_recomputes += 1);
    if viewport_height == 0 {
        return Vec::new();
    }

    let end_offset = scroll_offset.saturating_add(viewport_height);
    let mut visible = Vec::new();
    let first_visible_chunk = chunks.partition_point(|chunk| chunk.end_line <= scroll_offset);
    for (chunk_index, chunk) in chunks.iter().enumerate().skip(first_visible_chunk) {
        if chunk.start_line >= end_offset {
            break;
        }

        let start_in_chunk = scroll_offset.saturating_sub(chunk.start_line);
        let end_in_chunk = chunk
            .lines
            .len()
            .min(end_offset.saturating_sub(chunk.start_line));
        visible.push(VisibleChunkRange {
            chunk_index,
            start_in_chunk,
            end_in_chunk,
        });
    }
    visible
}

#[derive(Clone, PartialEq)]
struct MessageRenderOutput {
    lines: Arc<Vec<Line<'static>>>,
    toggle_line_offsets: Vec<ThinkingToggleHitOffset>,
    tool_arguments_toggle_line_offsets: Vec<ToolArgumentsToggleHitOffset>,
    visible_reasoning_ids: HashSet<String>,
    visible_tool_arguments_ids: HashSet<String>,
}

struct SessionMessageRenderItem {
    message_id: String,
    gap_before: bool,
    gap_after: usize,
    output: MessageRenderOutput,
}

#[derive(Clone, Default, PartialEq)]
struct SessionMessageOutputCache {
    entries: HashMap<String, CachedMessageRenderOutput>,
}

#[derive(Clone, PartialEq)]
struct CachedMessageRenderOutput {
    memo_key: u64,
    output: MessageRenderOutput,
}

#[derive(Clone, Default, PartialEq)]
struct SessionRenderModelCache {
    memo_key: Option<u64>,
    model: Option<Arc<SessionRenderModel>>,
}

#[derive(Clone)]
struct MessageRenderContext {
    theme: crate::theme::Theme,
    content_width: usize,
    show_timestamps: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
    user_bg: Color,
    thinking_bg: Color,
    assistant_border: Color,
    thinking_border: Color,
}

#[derive(Clone)]
struct UserMessageRenderProps {
    msg: Message,
    context: MessageRenderContext,
    show_system_prompt: bool,
}

#[derive(Clone)]
struct AssistantMessageRenderProps {
    msg: Message,
    context: MessageRenderContext,
    terminal_message: Option<TerminalMessage>,
    tool_results: HashMap<String, TerminalToolResultInfo>,
    running_tool_call: Option<String>,
    show_thinking: bool,
    expanded_reasoning: HashSet<String>,
    expanded_tool_arguments: HashSet<String>,
    footer_item: Option<AssistantFooterItem>,
}

#[derive(Clone)]
struct PlainMessageRenderProps {
    msg: Message,
    context: MessageRenderContext,
}

#[derive(Clone)]
enum SessionMessageRenderProps {
    User(UserMessageRenderProps),
    Assistant(AssistantMessageRenderProps),
    Plain(PlainMessageRenderProps),
}

#[derive(Clone)]
struct SessionMessageRenderInput {
    message_id: String,
    gap_before: bool,
    gap_after: usize,
    memo_key: u64,
    props: SessionMessageRenderProps,
}

struct SessionMessageItemComponent {
    input: SessionMessageRenderInput,
    output: Arc<Mutex<Option<SessionMessageRenderItem>>>,
}

struct AssistantMessageOutputComponent {
    props: AssistantMessageRenderProps,
    output: Arc<Mutex<Option<MessageRenderOutput>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct ThinkingToggleHitOffset {
    line_offset: usize,
    reasoning_id: String,
}

#[derive(Clone, PartialEq, Eq)]
struct ToolArgumentsToggleHitOffset {
    line_offset: usize,
    arguments_id: String,
}

#[derive(Clone)]
struct AssistantTextItem {
    text: String,
}

#[derive(Clone)]
struct AssistantThinkingItem {
    part_index: usize,
    text: String,
    hidden_by_preference: bool,
}

#[derive(Clone)]
struct AssistantToolBlockItem {
    message_id: String,
    part_index: usize,
    arguments_expanded: bool,
    name: String,
    arguments: String,
    state: agendao_command_render::terminal_presentation::TerminalToolState,
    result: Option<agendao_command_render::terminal_presentation::TerminalToolResultInfo>,
}

#[derive(Clone)]
struct AssistantFileItem {
    path: String,
    mime: String,
}

#[derive(Clone)]
struct AssistantImageItem {
    url: String,
}

#[derive(Clone, Debug)]
struct AssistantFooterItem {
    line: Line<'static>,
}

impl MessageRenderOutput {
    fn new(lines: Vec<Line<'static>>) -> Self {
        Self {
            lines: shared_lines(lines),
            toggle_line_offsets: Vec::new(),
            tool_arguments_toggle_line_offsets: Vec::new(),
            visible_reasoning_ids: HashSet::new(),
            visible_tool_arguments_ids: HashSet::new(),
        }
    }

    fn append_segment(&mut self, output: AssistantSegmentRenderOutput) {
        if output.lines.is_empty() {
            return;
        }

        let lines = Arc::make_mut(&mut self.lines);
        let start_line = lines.len();
        lines.extend(output.lines);
        self.visible_reasoning_ids
            .extend(output.visible_reasoning_ids);
        self.visible_tool_arguments_ids
            .extend(output.visible_tool_arguments_ids);
        self.toggle_line_offsets
            .extend(
                output
                    .toggle_line_offsets
                    .into_iter()
                    .map(|hit| ThinkingToggleHitOffset {
                        line_offset: start_line + hit.line_offset,
                        reasoning_id: hit.reasoning_id,
                    }),
            );
        self.tool_arguments_toggle_line_offsets
            .extend(
                output
                    .tool_arguments_toggle_line_offsets
                    .into_iter()
                    .map(|hit| ToolArgumentsToggleHitOffset {
                        line_offset: start_line + hit.line_offset,
                        arguments_id: hit.arguments_id,
                    }),
            );
    }
}

fn max_scroll_offset_for_viewport(viewport: &SessionMessageViewportState) -> usize {
    viewport
        .rendered_line_count
        .saturating_sub(viewport.messages_viewport_height)
}

fn apply_session_interaction_action(
    viewport: &mut SessionMessageViewportState,
    reasoning: &mut SessionReasoningState,
    action: &SessionInteractionAction,
) {
    match action {
        SessionInteractionAction::SetScrollOffset(offset) => {
            viewport.scroll_offset = (*offset).min(max_scroll_offset_for_viewport(viewport));
        }
        SessionInteractionAction::SetScrollbarDrag(active) => {
            viewport.scrollbar_drag_active = *active;
        }
        SessionInteractionAction::ToggleReasoning(reasoning_id) => {
            if !reasoning.expanded.insert(reasoning_id.clone()) {
                reasoning.expanded.remove(reasoning_id);
            }
        }
        SessionInteractionAction::ToggleToolArguments(arguments_id) => {
            if !reasoning
                .expanded_tool_arguments
                .insert(arguments_id.clone())
            {
                reasoning.expanded_tool_arguments.remove(arguments_id);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AssistantMessageRenderStyle {
    marker: Color,
    background: Color,
    border: Color,
}

impl Component for SessionStateBinderComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let (viewport, viewport_setter) = use_state(|| self.viewport_seed.clone());
        let (reasoning, reasoning_setter) = use_state(|| self.reasoning_seed.clone());
        let (sidebar, sidebar_setter) = use_state(|| self.sidebar_seed.clone());
        let mut next_viewport = viewport.clone();
        let mut next_reasoning = reasoning.clone();
        let next_sidebar = sidebar.clone();
        {
            let mut pending_actions = self.pending_actions.lock();
            for action in pending_actions.drain(..) {
                apply_session_interaction_action(&mut next_viewport, &mut next_reasoning, &action);
            }
        }
        if next_viewport != viewport {
            viewport_setter.set_if_changed(next_viewport.clone());
        }
        if next_reasoning != reasoning {
            reasoning_setter.set_if_changed(next_reasoning.clone());
        }
        if next_sidebar != sidebar {
            sidebar_setter.set_if_changed(next_sidebar.clone());
        }

        *self.bindings.lock() = SessionReactiveBindings {
            viewport: next_viewport,
            viewport_setter: Some(viewport_setter),
            reasoning: next_reasoning,
            reasoning_setter: Some(reasoning_setter),
            sidebar: next_sidebar,
            sidebar_setter: Some(sidebar_setter),
        };
    }
}

impl Component for SessionMessagesComponent {
    fn render(&self, _area: Rect, buffer: &mut Buffer) {
        let prompt_input_blocked =
            use_context::<crate::bridge::ReactivePromptInputBlocked>().0;
        let event_emitter = use_context::<crate::bridge::ReactiveUiEventEmitter>().0;
        let session_view = use_context::<crate::bridge::ReactiveSessionViewHandle>().0;
        let (message_cache, set_message_cache) = use_state(SessionMessageOutputCache::default);
        let (render_model_cache, set_render_model_cache) =
            use_state(SessionRenderModelCache::default);
        let viewport_ref = use_ref(|| self.viewport.clone());
        if !prompt_input_blocked {
            let keybind = use_context::<crate::context::KeybindRegistry>();
            let viewport_for_keys = viewport_ref.clone();
            let emitter_for_keys = event_emitter.clone();
            use_keyboard_press(move |key_event| {
                let key = crate::context::normalize_key_event(key_event);
                if keybind.match_key("page_up", key.code, key.modifiers) {
                    viewport_for_keys.update(|viewport| {
                        let step = viewport.messages_viewport_height.saturating_sub(1).max(1);
                        viewport.scroll_offset = viewport.scroll_offset.saturating_sub(step);
                    });
                    stop_propagation();
                } else if keybind.match_key("page_down", key.code, key.modifiers) {
                    viewport_for_keys.update(|viewport| {
                        let step = viewport.messages_viewport_height.saturating_sub(1).max(1);
                        let max_scroll = viewport
                            .rendered_line_count
                            .saturating_sub(viewport.messages_viewport_height);
                        viewport.scroll_offset =
                            viewport.scroll_offset.saturating_add(step).min(max_scroll);
                    });
                    stop_propagation();
                } else if keybind.match_key("session_parent", key.code, key.modifiers) {
                    let _ = emitter_for_keys.emit_custom_event(
                        crate::event::CustomEvent::SessionNavigationIntent {
                            kind: crate::event::SessionNavigationIntentKind::Parent,
                        },
                    );
                    stop_propagation();
                } else if keybind.match_key("session_attached_open", key.code, key.modifiers) {
                    let _ = emitter_for_keys.emit_custom_event(
                        crate::event::CustomEvent::SessionNavigationIntent {
                            kind: crate::event::SessionNavigationIntentKind::Attached,
                        },
                    );
                    stop_propagation();
                }
            });

            let viewport_for_mouse = viewport_ref.clone();
            let session_view_for_mouse = session_view.clone();
            use_mouse(move |mouse_event| {
                let in_messages = viewport_for_mouse.with(|viewport| {
                    viewport
                        .last_messages_area
                        .is_some_and(|area| point_in_rect(area, mouse_event.column, mouse_event.row))
                });
                let in_scrollbar = viewport_for_mouse.with(|viewport| {
                    viewport
                        .last_scrollbar_area
                        .is_some_and(|area| point_in_rect(area, mouse_event.column, mouse_event.row))
                });
                match mouse_event.kind {
                    MouseEventKind::Down(MouseButton::Left) if in_scrollbar => {
                        viewport_for_mouse.update(|viewport| {
                            viewport.scrollbar_drag_active = true;
                            let max_scroll = viewport
                                .rendered_line_count
                                .saturating_sub(viewport.messages_viewport_height);
                            viewport.scroll_offset = map_scrollbar_row_to_offset(
                                viewport.last_scrollbar_area,
                                mouse_event.row,
                                max_scroll,
                            );
                        });
                        stop_propagation();
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(view) = session_view_for_mouse.as_ref() {
                            if view.handle_click(mouse_event.column, mouse_event.row) {
                                stop_propagation();
                            }
                        }
                    }
                    MouseEventKind::Drag(MouseButton::Left)
                        if viewport_for_mouse.with(|viewport| viewport.scrollbar_drag_active)
                            || in_scrollbar =>
                    {
                        viewport_for_mouse.update(|viewport| {
                            viewport.scrollbar_drag_active = true;
                            let max_scroll = viewport
                                .rendered_line_count
                                .saturating_sub(viewport.messages_viewport_height);
                            viewport.scroll_offset = map_scrollbar_row_to_offset(
                                viewport.last_scrollbar_area,
                                mouse_event.row,
                                max_scroll,
                            );
                        });
                        stop_propagation();
                    }
                    MouseEventKind::Up(MouseButton::Left)
                        if viewport_for_mouse.with(|viewport| viewport.scrollbar_drag_active) =>
                    {
                        viewport_for_mouse.update(|viewport| {
                            viewport.scrollbar_drag_active = false;
                        });
                        stop_propagation();
                    }
                    MouseEventKind::ScrollUp if in_messages || in_scrollbar => {
                        viewport_for_mouse.update(|viewport| {
                            viewport.scroll_offset =
                                viewport.scroll_offset.saturating_sub(MOUSE_SCROLL_LINES);
                        });
                        stop_propagation();
                    }
                    MouseEventKind::ScrollDown if in_messages || in_scrollbar => {
                        viewport_for_mouse.update(|viewport| {
                            let max_scroll = viewport
                                .rendered_line_count
                                .saturating_sub(viewport.messages_viewport_height);
                            viewport.scroll_offset = viewport
                                .scroll_offset
                                .saturating_add(MOUSE_SCROLL_LINES)
                                .min(max_scroll);
                        });
                        stop_propagation();
                    }
                    MouseEventKind::Moved => {
                        if let Some(view) = session_view_for_mouse.as_ref() {
                            if view.handle_mouse_move(mouse_event.column, mouse_event.row) {
                                stop_propagation();
                            }
                        }
                    }
                    _ => {}
                }
            });
        }
        let output = render_session_messages_child(
            self.area,
            &self.snapshot,
            &viewport_ref.get(),
            &self.reasoning,
            &message_cache,
            &render_model_cache,
            buffer,
        );

        set_message_cache.set_if_changed(output.message_cache.clone());
        set_render_model_cache.set_if_changed(output.render_model_cache.clone());

        *self.output.lock() = Some(output);
    }
}

fn point_in_rect(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && col < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

impl Component for SessionMessageViewportComponent {
    fn render(&self, _area: Rect, buffer: &mut Buffer) {
        let visible_ranges = use_memo(
            || {
                collect_visible_chunk_ranges(
                    &self.model.chunks,
                    self.scroll_offset,
                    self.viewport_height,
                )
            },
            Some(build_session_viewport_content_memo_key(
                self.model.memo_key,
                self.scroll_offset,
                self.viewport_height,
            )),
        );
        render_session_message_viewport_widgets(
            buffer,
            self.messages_area,
            self.scrollbar_area,
            &self.theme,
            &self.model,
            &visible_ranges,
            self.model.rendered_line_count,
            self.scroll_offset,
            self.viewport_height,
        );
    }
}

impl Component for SessionMessageItemComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let output = match &self.input.props {
            SessionMessageRenderProps::User(props) => build_user_message_output(props),
            SessionMessageRenderProps::Assistant(props) => {
                let output = Arc::new(Mutex::new(None));
                Element::component(AssistantMessageOutputComponent {
                    props: props.clone(),
                    output: output.clone(),
                })
                .with_key(format!(
                    "session-message-assistant:{}",
                    self.input.message_id
                ))
                .render(area, buffer);
                let next_output = output
                    .lock()
                    .take()
                    .unwrap_or_else(|| MessageRenderOutput::new(Vec::new()));
                next_output
            }
            SessionMessageRenderProps::Plain(props) => build_plain_message_output(props),
        };
        *self.output.lock() = Some(SessionMessageRenderItem {
            message_id: self.input.message_id.clone(),
            gap_before: self.input.gap_before,
            gap_after: self.input.gap_after,
            output,
        });
    }
}

impl Component for AssistantMessageOutputComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let style = AssistantMessageRenderStyle {
            marker: assistant_marker_color(
                self.props.msg.agent.as_deref(),
                &self.props.context.theme,
            ),
            background: self.props.context.theme.background,
            border: self.props.context.assistant_border,
        };
        let mut output = MessageRenderOutput::new(Vec::new());
        for segment in render_assistant_block_outputs(area, buffer, &self.props, style) {
            output.append_segment(segment);
        }
        *self.output.lock() = Some(output);
    }
}
