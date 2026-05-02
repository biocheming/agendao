const SIDEBAR_WIDTH: u16 = 42;
const HEADER_NARROW_THRESHOLD: u16 = 80;
const THINKING_PREVIEW_LINES: usize = 2;
const MOUSE_SCROLL_LINES: usize = 3;
const MESSAGE_BLOCK_RIGHT_PADDING: usize = 1;
const SIDEBAR_CLOSE_BUTTON_WIDTH: u16 = 3;
const SIDEBAR_OPEN_BUTTON_WIDTH: u16 = 3;
const SEMANTIC_HIGHLIGHT_MAX_CHARS: usize = 8_000;

#[derive(Clone, PartialEq, Eq)]
struct ThinkingToggleHit {
    line_index: usize,
    reasoning_id: String,
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

#[derive(Clone, Default, PartialEq, Eq)]
struct SessionMessagesRenderState {
    messages_area: Option<Rect>,
    scrollbar_area: Option<Rect>,
    rendered_line_count: usize,
    viewport_height: usize,
}

struct SessionMessagesComponent {
    area: Rect,
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
    show_scrollbar: bool,
    show_timestamps: bool,
    show_thinking: bool,
    show_tool_details: bool,
    semantic_hl: bool,
    fallback_model: Option<String>,
}

impl SessionMessagesSnapshot {
    fn capture(context: &Arc<AppContext>, session_id: &str) -> Self {
        let theme = context.theme.read().clone();
        let show_scrollbar = context.show_scrollbar_enabled();
        let show_timestamps = context.show_timestamps_enabled();
        let show_thinking = context.show_thinking_enabled();
        let show_tool_details = context.show_tool_details_enabled();
        let semantic_hl = context.semantic_highlight_enabled();
        let fallback_model = context.current_model();
        let (messages, revert_info) = {
            let session_ctx = context.session.read();
            (
                session_ctx
                    .messages
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
                session_ctx.revert.get(session_id).cloned(),
            )
        };

        Self {
            theme,
            messages,
            revert_info,
            show_scrollbar,
            show_timestamps,
            show_thinking,
            show_tool_details,
            semantic_hl,
            fallback_model,
        }
    }
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
struct SessionFooterSnapshot {
    directory: String,
    context_meter: Option<String>,
    context_meter_percent: Option<u64>,
    permission_count: usize,
    connected_lsp: usize,
    connected_mcp: usize,
    has_mcp_failures: bool,
    has_mcp_registration_needed: bool,
    show_connect_hint: bool,
}

#[derive(Clone)]
struct SessionRenderSnapshot {
    theme: crate::theme::Theme,
    show_header: bool,
    header: SessionHeaderSnapshot,
    footer: SessionFooterSnapshot,
}

impl SessionRenderSnapshot {
    fn capture(context: &Arc<AppContext>, session_id: &str) -> Self {
        let theme = context.theme.read().clone();
        let show_header = context.show_header_enabled();
        let directory = context.directory.read().clone();
        let permission_count = *context.pending_permissions.read();
        let has_connected_provider = *context.has_connected_provider.read();

        let selection = context.selection_state();
        let (
            parent_title,
            title,
            subtitle,
            usage,
            footer_context_meter,
            footer_context_meter_percent,
            status_label,
            status_running,
            status_retrying,
        ) = {
            let session_ctx = context.session.read();
            let session = session_ctx.sessions.get(session_id);
            let title = session
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "New Session".to_string());
            let parent_title = session
                .and_then(|session| session.parent_id.as_ref())
                .and_then(|parent_id| session_ctx.sessions.get(parent_id))
                .map(|session| session.title.clone());
            let messages = session_ctx
                .messages
                .get(session_id)
                .cloned()
                .unwrap_or_default();
            let subtitle = build_session_header_subtitle(
                session.and_then(|session| session.metadata.as_ref()),
                &selection,
            );
            let status = session_ctx
                .session_status
                .get(session_id)
                .cloned()
                .unwrap_or_default();

            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::Assistant) && m.tokens.output > 0);

            let total_cost = context
                .session_usage()
                .as_ref()
                .map(|usage| usage.total_cost)
                .unwrap_or_else(|| {
                    messages
                        .iter()
                        .filter(|m| matches!(m.role, MessageRole::Assistant))
                        .map(|m| m.cost)
                        .sum()
                });
            let current_context_tokens = context.current_context_tokens();

            let usage = last_assistant.and_then(|assistant_msg| {
                let total_tokens = context
                    .session_usage()
                    .as_ref()
                    .map(total_session_tokens)
                    .unwrap_or_else(|| {
                        let t = &assistant_msg.tokens;
                        t.input + t.output + t.reasoning
                    });
                if total_tokens == 0 {
                    return None;
                }

                let mut parts = Vec::new();
                parts.push(format!("session {} total", format_compact_number(total_tokens)));
                parts.push(format!("${:.4}", total_cost));
                Some(parts.join("  ·  "))
            });
            let footer_context_meter = last_assistant.and_then(|assistant_msg| {
                let current_context_tokens = current_context_tokens?;
                let total_tokens = context
                    .session_usage()
                    .as_ref()
                    .map(total_session_tokens)
                    .unwrap_or_else(|| {
                        let t = &assistant_msg.tokens;
                        t.input + t.output + t.reasoning
                    });
                if total_tokens == 0 {
                    return None;
                }
                let model = context.resolve_model_info(assistant_msg.model.as_deref());
                format_context_usage_meter(
                    current_context_tokens,
                    model.as_ref().map(|model| model.context_window),
                )
            });

            let (status_label, status_running, status_retrying) = match status {
                crate::context::SessionStatus::Running => {
                    (Some("RUNNING".to_string()), true, false)
                }
                crate::context::SessionStatus::Retrying { attempt, .. } => {
                    (Some(format!("RETRY {}", attempt)), true, true)
                }
                crate::context::SessionStatus::Idle => (None, false, false),
            };

            (
                parent_title,
                title,
                subtitle,
                usage,
                footer_context_meter
                    .as_ref()
                    .map(|(label, _)| label.clone()),
                footer_context_meter.and_then(|(_, percent)| percent),
                status_label,
                status_running,
                status_retrying,
            )
        };

        let (connected_lsp, connected_mcp, has_mcp_failures, has_mcp_registration_needed) = {
            let lsp_status = context.lsp_status.read();
            let mcp_servers = context.mcp_servers.read();
            let connected_lsp = lsp_status
                .iter()
                .filter(|s| matches!(s.status, crate::context::LspConnectionStatus::Connected))
                .count();
            let connected_mcp = mcp_servers
                .iter()
                .filter(|s| matches!(s.status, crate::context::McpConnectionStatus::Connected))
                .count();
            let has_mcp_failures = mcp_servers
                .iter()
                .any(|s| matches!(s.status, crate::context::McpConnectionStatus::Failed));
            let has_mcp_registration_needed = mcp_servers.iter().any(|s| {
                matches!(
                    s.status,
                    crate::context::McpConnectionStatus::NeedsClientRegistration
                )
            });
            (
                connected_lsp,
                connected_mcp,
                has_mcp_failures,
                has_mcp_registration_needed,
            )
        };

        Self {
            theme,
            show_header,
            header: SessionHeaderSnapshot {
                parent_title,
                title,
                subtitle,
                usage,
                status_label,
                status_running,
                status_retrying,
            },
            footer: SessionFooterSnapshot {
                directory,
                context_meter: footer_context_meter,
                context_meter_percent: footer_context_meter_percent,
                permission_count,
                connected_lsp,
                connected_mcp,
                has_mcp_failures,
                has_mcp_registration_needed,
                show_connect_hint: !has_connected_provider
                    && Utc::now().timestamp().rem_euclid(15) >= 10,
            },
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
    footer_area: Rect,
    prompt_area: Rect,
    show_header: bool,
    show_prompt: bool,
}

struct SessionRenderResources<'a> {
    theme: crate::theme::Theme,
    messages: &'a [Message],
    terminal_messages: &'a [TerminalMessage],
    revert_info: Option<crate::context::RevertInfo>,
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

struct SessionRenderModel {
    memo_key: u64,
    chunks: Vec<SessionRenderChunk>,
    rendered_line_count: usize,
    message_first_lines: HashMap<String, usize>,
    toggle_hits: Vec<ThinkingToggleHit>,
    visible_reasoning_ids: HashSet<String>,
}

#[derive(Clone)]
struct AssistantSegmentRenderOutput {
    lines: Vec<Line<'static>>,
    toggle_line_offsets: Vec<ThinkingToggleHitOffset>,
    visible_reasoning_ids: HashSet<String>,
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
    visible_reasoning_ids: HashSet<String>,
}

struct SessionMessageRenderItem {
    message_id: String,
    gap_before: bool,
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

#[derive(Clone, Default)]
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
    memo_key: u64,
    props: SessionMessageRenderProps,
}

struct SessionMessageItemComponent {
    input: SessionMessageRenderInput,
    output: Arc<Mutex<Option<SessionMessageRenderItem>>>,
}

struct UserMessageOutputComponent {
    props: UserMessageRenderProps,
    output: Arc<Mutex<Option<MessageRenderOutput>>>,
}

struct AssistantMessageOutputComponent {
    props: AssistantMessageRenderProps,
    output: Arc<Mutex<Option<MessageRenderOutput>>>,
}

struct PlainMessageOutputComponent {
    props: PlainMessageRenderProps,
    output: Arc<Mutex<Option<MessageRenderOutput>>>,
}

#[derive(Clone)]
struct AssistantBlockRenderInput {
    block_key: String,
    memo_key: u64,
    item: AssistantMessageItem,
}

struct AssistantSpacerBlockComponent {
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantTextBlockComponent {
    msg: Message,
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    item: AssistantTextItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantThinkingBlockComponent {
    msg: Message,
    context: MessageRenderContext,
    expanded_reasoning: HashSet<String>,
    memo_key: u64,
    item: AssistantThinkingItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantToolOutputComponent {
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    item: AssistantToolBlockItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantFileBlockComponent {
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    item: AssistantFileItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantImageBlockComponent {
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    item: AssistantImageItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

struct AssistantFooterBlockComponent {
    context: MessageRenderContext,
    style: AssistantMessageRenderStyle,
    memo_key: u64,
    item: AssistantFooterItem,
    output: Arc<Mutex<Option<AssistantSegmentRenderOutput>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct ThinkingToggleHitOffset {
    line_offset: usize,
    reasoning_id: String,
}

#[derive(Clone)]
struct AssistantTextItem {
    text: String,
}

#[derive(Clone)]
struct AssistantThinkingItem {
    part_index: usize,
    text: String,
}

#[derive(Clone)]
struct AssistantToolBlockItem {
    name: String,
    arguments: String,
    state: rocode_command::terminal_presentation::TerminalToolState,
    result: Option<rocode_command::terminal_presentation::TerminalToolResultInfo>,
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
            visible_reasoning_ids: HashSet::new(),
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

        *self.bindings.lock() = SessionReactiveBindings {
            viewport: next_viewport,
            viewport_setter: Some(viewport_setter),
            reasoning: next_reasoning,
            reasoning_setter: Some(reasoning_setter),
            sidebar,
            sidebar_setter: Some(sidebar_setter),
        };
    }
}

impl Component for SessionMessagesComponent {
    fn render(&self, _area: Rect, buffer: &mut Buffer) {
        let context = use_context::<ReactiveAppContextHandle>().0;
        let session = use_context::<ReactiveSessionContext>();
        let (render_state, set_render_state) = use_state(SessionMessagesRenderState::default);
        let (message_cache, set_message_cache) = use_state(SessionMessageOutputCache::default);
        let (render_model_cache, set_render_model_cache) =
            use_state(SessionRenderModelCache::default);
        let snapshot = SessionMessagesSnapshot::capture(&context, &session.session_id);
        let output = render_session_messages_child(
            self.area,
            &snapshot,
            &self.viewport,
            &self.reasoning,
            &message_cache,
            &render_model_cache,
            buffer,
        );

        let next_render_state = SessionMessagesRenderState {
            messages_area: output.viewport.last_messages_area,
            scrollbar_area: output.viewport.last_scrollbar_area,
            rendered_line_count: output.viewport.rendered_line_count,
            viewport_height: output.viewport.messages_viewport_height,
        };
        if render_state != next_render_state {
            set_render_state.set_if_changed(next_render_state);
        }
        if message_cache != output.message_cache {
            set_message_cache.set(output.message_cache.clone());
        }
        if render_model_cache.memo_key != output.render_model_cache.memo_key {
            set_render_model_cache.set(output.render_model_cache.clone());
        }

        *self.output.lock() = Some(output);
    }
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
        let output = Arc::new(Mutex::new(None));
        let child = match &self.input.props {
            SessionMessageRenderProps::User(props) => {
                Element::component(UserMessageOutputComponent {
                    props: props.clone(),
                    output: output.clone(),
                })
                .with_key(format!("session-message-user:{}", self.input.message_id))
            }
            SessionMessageRenderProps::Assistant(props) => {
                Element::component(AssistantMessageOutputComponent {
                    props: props.clone(),
                    output: output.clone(),
                })
                .with_key(format!(
                    "session-message-assistant:{}",
                    self.input.message_id
                ))
            }
            SessionMessageRenderProps::Plain(props) => {
                Element::component(PlainMessageOutputComponent {
                    props: props.clone(),
                    output: output.clone(),
                })
                .with_key(format!("session-message-plain:{}", self.input.message_id))
            }
        };
        child.render(area, buffer);
        let next_output = output.lock().take();
        let output = next_output.unwrap_or_else(|| MessageRenderOutput::new(Vec::new()));
        *self.output.lock() = Some(SessionMessageRenderItem {
            message_id: self.input.message_id.clone(),
            gap_before: self.input.gap_before,
            output,
        });
    }
}

impl Component for UserMessageOutputComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || build_user_message_output(&self.props),
            Some(build_user_message_memo_key(&self.props)),
        );
        *self.output.lock() = Some(output);
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

impl Component for PlainMessageOutputComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || build_plain_message_output(&self.props),
            Some(build_plain_message_memo_key(&self.props)),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantSpacerBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_spacer_block(self.style, &self.context),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantTextBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_text_block(&self.msg, &self.item, &self.context, self.style),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantThinkingBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || {
                render_assistant_thinking_output(
                    &self.msg,
                    &self.item,
                    &self.context,
                    &self.expanded_reasoning,
                )
            },
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantToolOutputComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_tool_output(&self.item, self.style, &self.context),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantFileBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_file_output(&self.item, self.style, &self.context),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantImageBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_image_output(&self.item, self.style, &self.context),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}

impl Component for AssistantFooterBlockComponent {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let output = use_memo(
            || render_assistant_footer_output(&self.item, self.style, &self.context),
            Some(self.memo_key),
        );
        *self.output.lock() = Some(output);
    }
}
