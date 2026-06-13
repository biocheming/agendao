use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph,
        Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};
use reratui::{Buffer, Component};

use crate::branding::{APP_NAME, APP_SHORT_NAME, APP_VERSION_DATE};
use crate::components::usage_resolver::resolve_usage;
use crate::context::session_context::fold_messages;
use crate::file_index::FileIndex;
use crate::render::RenderSurface;
use crate::state::{
    AppContext, LspConnectionStatus, McpConnectionStatus, MessageRole, ProviderInfo,
    SidebarLifecycleState, SidebarMode, SidebarTab, TodoStatus,
};
use crate::theme::Theme;
use agendao_core::process_registry::ProcessKind;
use agendao_core::process_registry::ProcessInfo;
use agendao_types::{SessionContextClosureContract, SessionUsage, SessionUsageBooks};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use reratui::hooks::{stop_propagation, use_context, use_keyboard_press, use_mouse};

const SIDEBAR_WORKSPACE_INDEX_MAX_DEPTH: usize = 8;

#[derive(Clone)]
pub struct Sidebar {
    session_id: String,
}

struct SidebarComponent {
    sidebar: Sidebar,
    render_inputs: SidebarRenderInputs,
    area: Rect,
    state: Arc<Mutex<SidebarRenderState>>,
    lifecycle: Arc<Mutex<SidebarLifecycleState>>,
    floating: bool,
    bg_override: Option<ratatui::style::Color>,
    chrome: SidebarChromeProps,
}

#[derive(Clone)]
struct SidebarRenderSnapshot {
    session_ctx: crate::context::SessionContext,
    mcp_servers: Vec<crate::context::McpServerStatus>,
    lsp_status: Vec<crate::context::LspStatus>,
    providers: Vec<ProviderInfo>,
    processes: Vec<ProcessInfo>,
    directory: String,
    attached_sessions: Vec<crate::context::AttachedSessionInfo>,
    execution_topology: Option<crate::api::SessionExecutionTopology>,
    session_usage: Option<SessionUsage>,
    session_usage_books: Option<SessionUsageBooks>,
    context_closure_contract: Option<SessionContextClosureContract>,
    current_context_tokens: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct SidebarRenderSeed {
    theme: Theme,
    session_ctx: crate::context::SessionContext,
    mcp_servers: Vec<crate::context::McpServerStatus>,
    lsp_status: Vec<crate::context::LspStatus>,
    providers: Vec<ProviderInfo>,
    processes: Vec<ProcessInfo>,
    directory: String,
    attached_sessions: Vec<crate::context::AttachedSessionInfo>,
    execution_topology: Option<crate::api::SessionExecutionTopology>,
    session_usage: Option<SessionUsage>,
    session_usage_books: Option<SessionUsageBooks>,
    context_closure_contract: Option<SessionContextClosureContract>,
    current_context_tokens: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct SidebarRenderInputs {
    theme: Theme,
    snapshot: SidebarRenderSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarChromeMode {
    Docked,
    Overlay,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SidebarChromeProps {
    pub mode: SidebarChromeMode,
    pub container_area: Rect,
    pub layout_width: u16,
    pub open_button_area: Option<Rect>,
    pub close_button_area: Option<Rect>,
    pub backdrop_area: Option<Rect>,
}

#[derive(Clone, PartialEq, Eq)]
struct SidebarToggleHit {
    line_index: usize,
    section_key: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SidebarTabHit {
    tab: SidebarTab,
    area: Rect,
}

#[derive(Clone, PartialEq, Eq)]
struct WorkspaceVisibleNode {
    path: String,
    label: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
    is_modified: bool,
    is_current: bool,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SidebarRenderState {
    collapsed_sections: HashMap<&'static str, bool>,
    scroll_offset: usize,
    content_lines: usize,
    viewport_lines: usize,
    sidebar_area: Option<Rect>,
    tabs_area: Option<Rect>,
    sections_area: Option<Rect>,
    tab_hits: Vec<SidebarTabHit>,
    toggle_hits: Vec<SidebarToggleHit>,
    /// Maps rendered line index → process list index (for click selection).
    process_line_hits: Vec<(usize, usize)>,
    /// Maps rendered line index → attached session list index (for click selection).
    attached_session_line_hits: Vec<(usize, usize)>,
    /// Maps rendered line index → workspace visible node index.
    workspace_line_hits: Vec<(usize, usize)>,
    workspace_index: FileIndex,
    workspace_expanded_dirs: HashSet<String>,
    workspace_visible_nodes: Vec<WorkspaceVisibleNode>,
    workspace_selected_path: Option<String>,
    workspace_tooltip: Option<String>,
    workspace_seeded_root: Option<String>,
    session_graph_root_id: Option<String>,
    session_graph_active_id: Option<String>,
    /// Pending navigation target set by click-to-activate on an already-selected attached session.
    /// Consumed (taken) by the app after `handle_click` returns.
    pending_navigate_attached: Option<usize>,
    /// Pending direct session navigation target.
    pending_navigate_session: Option<String>,
    /// Legacy fallback for parent navigation when no explicit graph root is available.
    pending_navigate_parent: bool,
}

impl SidebarRenderState {
    pub fn reset_hidden(&mut self) {
        self.sidebar_area = None;
        self.tabs_area = None;
        self.sections_area = None;
        self.tab_hits.clear();
        self.toggle_hits.clear();
        self.scroll_offset = 0;
        self.content_lines = 0;
        self.viewport_lines = 0;
    }

    fn set_sidebar_area(&mut self, area: Rect) {
        self.sidebar_area = Some(area);
    }

    fn set_tab_layout(&mut self, tabs_area: Rect, tab_hits: Vec<SidebarTabHit>) {
        self.tabs_area = Some(tabs_area);
        self.tab_hits = tab_hits;
    }

    fn set_sections_layout(
        &mut self,
        sections_area: Rect,
        content_lines: usize,
        toggle_hits: Vec<SidebarToggleHit>,
    ) {
        self.sections_area = Some(sections_area);
        self.content_lines = content_lines;
        self.viewport_lines = usize::from(sections_area.height);
        self.toggle_hits = toggle_hits;
        self.clamp_scroll();
    }

    pub fn contains_sidebar_point(&self, col: u16, row: u16) -> bool {
        contains_point(self.sidebar_area, col, row)
    }

    pub fn sidebar_area(&self) -> Option<Rect> {
        self.sidebar_area
    }

    pub fn handle_click(
        &mut self,
        lifecycle: &mut SidebarLifecycleState,
        col: u16,
        row: u16,
    ) -> bool {
        if let Some(hit) = self
            .tab_hits
            .iter()
            .find(|hit| contains_point(Some(hit.area), col, row))
        {
            lifecycle.active_tab = hit.tab;
            lifecycle.process_focus = false;
            lifecycle.attached_session_focus = false;
            lifecycle.workspace_focus = hit.tab == SidebarTab::Workspace;
            if hit.tab != SidebarTab::Workspace {
                self.workspace_tooltip = None;
            }
            self.scroll_offset = 0;
            return true;
        }

        let Some(area) = self.sections_area else {
            return false;
        };
        if !contains_point(Some(area), col, row) {
            return false;
        }

        let relative_row = usize::from(row.saturating_sub(area.y));
        let line_index = self.scroll_offset.saturating_add(relative_row);

        // Check if the click is on a process item
        if let Some((_line_idx, proc_idx)) = self
            .process_line_hits
            .iter()
            .find(|(li, _)| *li == line_index)
        {
            lifecycle.process_selected = *proc_idx;
            lifecycle.active_tab = SidebarTab::Session;
            lifecycle.process_focus = true;
            lifecycle.attached_session_focus = false;
            lifecycle.workspace_focus = false;
            self.workspace_tooltip = None;
            return true;
        }

        // Check if the click is on a attached session item
        if let Some((_line_idx, cs_idx)) = self
            .attached_session_line_hits
            .iter()
            .find(|(li, _)| *li == line_index)
        {
            if *cs_idx == usize::MAX {
                // Root session node should always activate the graph root session.
                if let Some(root_id) = self.session_graph_root_id.as_ref() {
                    self.pending_navigate_session = Some(root_id.clone());
                } else {
                    self.pending_navigate_parent = true;
                }
                self.workspace_tooltip = None;
                return true;
            }
            if lifecycle.attached_session_focus && lifecycle.attached_session_selected == *cs_idx {
                // Already selected and focused — treat as activation (navigate)
                self.pending_navigate_attached = Some(*cs_idx);
            } else {
                // First click — select and focus
                lifecycle.active_tab = SidebarTab::Session;
                lifecycle.attached_session_selected = *cs_idx;
                lifecycle.attached_session_focus = true;
                lifecycle.process_focus = false;
                lifecycle.workspace_focus = false;
            }
            self.workspace_tooltip = None;
            return true;
        }

        if let Some((_line_idx, node_idx)) = self
            .workspace_line_hits
            .iter()
            .find(|(li, _)| *li == line_index)
        {
            let was_selected =
                lifecycle.workspace_focus && lifecycle.workspace_selected == *node_idx;
            lifecycle.active_tab = SidebarTab::Workspace;
            lifecycle.workspace_selected = *node_idx;
            lifecycle.workspace_focus = true;
            lifecycle.process_focus = false;
            lifecycle.attached_session_focus = false;
            if let Some(node) = self.workspace_visible_nodes.get(*node_idx) {
                let tooltip = workspace_popup_text(self.sections_area, node);
                let node_path = node.path.clone();
                let node_is_dir = node.is_dir;
                self.workspace_selected_path = Some(node_path.clone());
                if node_is_dir && was_selected {
                    self.toggle_workspace_dir(&node_path);
                }
                self.workspace_tooltip = tooltip;
            }
            return true;
        }

        let Some(section_key) = self
            .toggle_hits
            .iter()
            .find(|hit| hit.line_index == line_index)
            .map(|hit| hit.section_key)
        else {
            return false;
        };

        let collapsed = self.collapsed_sections.entry(section_key).or_insert(false);
        *collapsed = !*collapsed;
        true
    }

    pub fn scroll_up_at(&mut self, col: u16, row: u16) -> bool {
        if !self.contains_sidebar_point(col, row) {
            return false;
        }
        self.scroll_up();
        true
    }

    pub fn scroll_down_at(&mut self, col: u16, row: u16) -> bool {
        if !self.contains_sidebar_point(col, row) {
            return false;
        }
        self.scroll_down();
        true
    }

    fn is_collapsed(&self, section_key: &'static str) -> bool {
        self.collapsed_sections
            .get(section_key)
            .copied()
            .unwrap_or(false)
    }

    fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    fn scroll_down(&mut self) {
        let max_scroll = self.max_scroll();
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
        }
    }

    fn max_scroll(&self) -> usize {
        self.content_lines.saturating_sub(self.viewport_lines)
    }

    fn clamp_scroll(&mut self) {
        let max_scroll = self.max_scroll();
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
    }

    /// Take the pending attached session navigation index, if any.
    /// Returns the index into the attached_sessions list that was clicked for activation.
    pub fn take_pending_navigate_attached(&mut self) -> Option<usize> {
        self.pending_navigate_attached.take()
    }

    /// Take the pending direct session navigation target, if any.
    pub fn take_pending_navigate_session(&mut self) -> Option<String> {
        self.pending_navigate_session.take()
    }

    /// Take the pending parent/root session navigation flag.
    pub fn take_pending_navigate_parent(&mut self) -> bool {
        std::mem::take(&mut self.pending_navigate_parent)
    }

    pub fn refresh_workspace_index(&mut self, root: &PathBuf) {
        self.workspace_index
            .refresh(root, SIDEBAR_WORKSPACE_INDEX_MAX_DEPTH);
    }

    fn set_workspace_visible_nodes(&mut self, nodes: Vec<WorkspaceVisibleNode>) {
        self.workspace_visible_nodes = nodes;
    }

    pub fn sync_workspace_selection(
        &mut self,
        lifecycle: &mut SidebarLifecycleState,
        preferred_path: Option<&str>,
    ) {
        if self.workspace_visible_nodes.is_empty() {
            lifecycle.workspace_selected = 0;
            self.workspace_selected_path = None;
            return;
        }

        let selected_index = self
            .workspace_selected_path
            .as_deref()
            .and_then(|path| {
                self.workspace_visible_nodes
                    .iter()
                    .position(|node| node.path == path)
            })
            .or_else(|| {
                preferred_path.and_then(|path| {
                    self.workspace_visible_nodes
                        .iter()
                        .position(|node| node.path == path)
                })
            })
            .unwrap_or_else(|| {
                lifecycle
                    .workspace_selected
                    .min(self.workspace_visible_nodes.len().saturating_sub(1))
            });
        lifecycle.workspace_selected = selected_index;
        self.workspace_selected_path = self
            .workspace_visible_nodes
            .get(selected_index)
            .map(|node| node.path.clone());
    }

    pub fn workspace_visible_count(&self) -> usize {
        self.workspace_visible_nodes.len()
    }

    pub fn set_workspace_selected_index(
        &mut self,
        lifecycle: &mut SidebarLifecycleState,
        index: usize,
    ) {
        if let Some(node) = self.workspace_visible_nodes.get(index) {
            lifecycle.workspace_selected = index;
            self.workspace_selected_path = Some(node.path.clone());
        } else {
            lifecycle.workspace_selected = 0;
            self.workspace_selected_path = None;
        }
    }

    pub fn expand_selected_workspace_dir(&mut self, lifecycle: &mut SidebarLifecycleState) -> bool {
        let Some(node) = self
            .workspace_visible_nodes
            .get(lifecycle.workspace_selected)
        else {
            return false;
        };
        if !node.is_dir || node.expanded {
            return false;
        }
        self.workspace_expanded_dirs.insert(node.path.clone());
        true
    }

    pub fn collapse_selected_workspace_dir(
        &mut self,
        lifecycle: &mut SidebarLifecycleState,
    ) -> bool {
        let Some(node) = self
            .workspace_visible_nodes
            .get(lifecycle.workspace_selected)
        else {
            return false;
        };
        if node.is_dir && node.expanded {
            self.workspace_expanded_dirs.remove(&node.path);
            return true;
        }
        let Some(parent) = workspace_parent_path(&node.path) else {
            return false;
        };
        if let Some(index) = self
            .workspace_visible_nodes
            .iter()
            .position(|candidate| candidate.path == parent)
        {
            lifecycle.workspace_selected = index;
            self.workspace_selected_path = Some(parent);
            return true;
        }
        false
    }

    fn toggle_workspace_dir(&mut self, path: &str) {
        if self.workspace_expanded_dirs.contains(path) {
            self.workspace_expanded_dirs.remove(path);
        } else {
            self.workspace_expanded_dirs.insert(path.to_string());
        }
    }
}

fn clamp_sidebar_process_selection(lifecycle: &mut SidebarLifecycleState, count: usize) {
    if count == 0 {
        lifecycle.process_selected = 0;
    } else if lifecycle.process_selected >= count {
        lifecycle.process_selected = count - 1;
    }
}

fn clamp_sidebar_attached_session_selection(lifecycle: &mut SidebarLifecycleState, count: usize) {
    if count == 0 {
        lifecycle.attached_session_selected = 0;
    } else if lifecycle.attached_session_selected >= count {
        lifecycle.attached_session_selected = count - 1;
    }
}

fn clamp_sidebar_workspace_selection(lifecycle: &mut SidebarLifecycleState, count: usize) {
    if count == 0 {
        lifecycle.workspace_selected = 0;
    } else if lifecycle.workspace_selected >= count {
        lifecycle.workspace_selected = count - 1;
    }
}

fn sidebar_is_visible(lifecycle: &SidebarLifecycleState, terminal_width: u16) -> bool {
    match lifecycle.mode {
        SidebarMode::Hide => false,
        SidebarMode::Show => true,
        SidebarMode::Auto => {
            sidebar_is_wide(terminal_width) || lifecycle.visible
        }
    }
}

fn sidebar_is_wide(terminal_width: u16) -> bool {
    terminal_width > crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD
}

fn sidebar_default_visible(terminal_width: u16) -> bool {
    !sidebar_is_wide(terminal_width)
}

struct SidebarSection {
    key: &'static str,
    title: &'static str,
    lines: Vec<Line<'static>>,
    meter: Option<SidebarSectionMeter>,
    attached_session_hit_rows: Option<Vec<Option<usize>>>,
    workspace_hit_rows: Option<Vec<Option<usize>>>,
    summary: Option<String>,
    collapsible: bool,
}

#[derive(Clone)]
struct SidebarSectionMeter {
    label: String,
    ratio: f64,
    style: Style,
}

struct SidebarSectionsDocument {
    rows: Vec<SidebarSectionRow>,
    toggle_hits: Vec<SidebarToggleHit>,
    process_line_hits: Vec<(usize, usize)>,
    attached_session_line_hits: Vec<(usize, usize)>,
    workspace_line_hits: Vec<(usize, usize)>,
}

enum SidebarSectionRow {
    Line(Line<'static>),
    Gauge(SidebarSectionMeter),
}

fn build_sidebar_sections_document(
    theme: &Theme,
    state: &SidebarRenderState,
    sections: Vec<SidebarSection>,
) -> SidebarSectionsDocument {
    let mut rows: Vec<SidebarSectionRow> = Vec::new();
    let mut line_index = 0usize;
    let mut toggle_hits: Vec<SidebarToggleHit> = Vec::new();
    let mut process_line_hits: Vec<(usize, usize)> = Vec::new();
    let mut attached_session_line_hits: Vec<(usize, usize)> = Vec::new();
    let mut workspace_line_hits: Vec<(usize, usize)> = Vec::new();

    for section in sections {
        if !rows.is_empty() {
            rows.push(SidebarSectionRow::Line(Line::from("")));
            line_index += 1;
        }

        let collapsed = section.collapsible && state.is_collapsed(section.key);
        let mut header = Vec::new();
        if section.collapsible {
            toggle_hits.push(SidebarToggleHit {
                line_index,
                section_key: section.key,
            });
            header.push(Span::styled(
                if collapsed { "▶ " } else { "▼ " },
                Style::default().fg(theme.text_muted),
            ));
        } else {
            header.push(Span::styled("  ", Style::default().fg(theme.text_muted)));
        }
        header.push(Span::styled(
            section.title.to_string(),
            Style::default().fg(theme.text).bold(),
        ));
        if let Some(summary) = section.summary {
            header.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
            header.push(Span::styled(summary, Style::default().fg(theme.text_muted)));
        }
        rows.push(SidebarSectionRow::Line(Line::from(header)));
        line_index += 1;

        if !collapsed {
            let is_processes = section.key == "processes";
            let child_hit_rows = section.attached_session_hit_rows.as_ref();
            let workspace_hit_rows = section.workspace_hit_rows.as_ref();
            rows.push(SidebarSectionRow::Line(Line::from(Span::styled(
                "  ",
                Style::default().fg(theme.border_subtle),
            ))));
            line_index += 1;
            for (row_idx, row) in section.lines.into_iter().enumerate() {
                if is_processes {
                    process_line_hits.push((line_index, row_idx));
                }
                if let Some(hit_rows) = child_hit_rows {
                    if let Some(Some(child_index)) = hit_rows.get(row_idx) {
                        attached_session_line_hits.push((line_index, *child_index));
                    }
                }
                if let Some(hit_rows) = workspace_hit_rows {
                    if let Some(Some(node_index)) = hit_rows.get(row_idx) {
                        workspace_line_hits.push((line_index, *node_index));
                    }
                }
                let mut spans = Vec::with_capacity(row.spans.len() + 1);
                spans.push(Span::styled("  ", Style::default().fg(theme.border_subtle)));
                spans.extend(
                    row.spans
                        .into_iter()
                        .map(|span| Span::styled(span.content, span.style)),
                );
                rows.push(SidebarSectionRow::Line(Line::from(spans)));
                line_index += 1;
            }
            if let Some(meter) = section.meter {
                rows.push(SidebarSectionRow::Gauge(meter));
                line_index += 1;
            }
        }
    }

    SidebarSectionsDocument {
        rows,
        toggle_hits,
        process_line_hits,
        attached_session_line_hits,
        workspace_line_hits,
    }
}

impl Component for SidebarComponent {
    fn render(&self, _area: Rect, buffer: &mut Buffer) {
        let render_state_ref = Arc::new(Mutex::new(self.state.lock().clone()));
        let lifecycle_ref = Arc::new(Mutex::new(self.lifecycle.lock().clone()));
        let event_emitter = use_context::<crate::bridge::ReactiveUiEventEmitter>().0;
        let keybind = use_context::<crate::context::KeybindRegistry>();
        let terminal_width = self.chrome.layout_width;
        let process_count = self.render_inputs.snapshot.processes.len();
        let attached_sessions = self.render_inputs.snapshot.attached_sessions.clone();
        let render_state_for_keys = render_state_ref.clone();
        let lifecycle_for_keys = lifecycle_ref.clone();
        let emitter_for_keys = event_emitter.clone();
        use_keyboard_press(move |key_event| {
            let key = crate::context::normalize_key_event(key_event);
            let mut render_state = render_state_for_keys.lock();
            let mut lifecycle = lifecycle_for_keys.lock();

            if key.code == KeyCode::Esc {
                let had_focus = lifecycle.process_focus
                    || lifecycle.attached_session_focus
                    || lifecycle.workspace_focus;
                lifecycle.process_focus = false;
                lifecycle.attached_session_focus = false;
                lifecycle.workspace_focus = false;
                if had_focus {
                    stop_propagation();
                }
            } else if keybind.match_key("session_attached_focus", key.code, key.modifiers) {
                if sidebar_is_visible(&lifecycle, terminal_width) {
                    lifecycle.active_tab = SidebarTab::Session;
                    lifecycle.attached_session_focus = !lifecycle.attached_session_focus;
                    if lifecycle.attached_session_focus {
                        lifecycle.process_focus = false;
                        lifecycle.workspace_focus = false;
                    }
                    stop_propagation();
                }
            } else if keybind.match_key("session_workspace_focus", key.code, key.modifiers) {
                if sidebar_is_visible(&lifecycle, terminal_width) {
                    lifecycle.active_tab = SidebarTab::Workspace;
                    lifecycle.workspace_focus = !lifecycle.workspace_focus;
                    if lifecycle.workspace_focus {
                        lifecycle.process_focus = false;
                        lifecycle.attached_session_focus = false;
                    }
                    stop_propagation();
                }
            } else if keybind.match_key("sidebar_toggle", key.code, key.modifiers) {
                if sidebar_is_visible(&lifecycle, terminal_width) {
                    if sidebar_is_wide(terminal_width) {
                        lifecycle.mode = crate::context::SidebarMode::Hide;
                    }
                    lifecycle.visible = false;
                    lifecycle.process_focus = false;
                    lifecycle.attached_session_focus = false;
                    lifecycle.workspace_focus = false;
                } else {
                    lifecycle.mode = crate::context::SidebarMode::Auto;
                    lifecycle.visible = sidebar_default_visible(terminal_width);
                }
                stop_propagation();
            } else if key.code == KeyCode::Char('p') && key.modifiers.is_empty() {
                if sidebar_is_visible(&lifecycle, terminal_width) {
                    lifecycle.active_tab = SidebarTab::Session;
                    lifecycle.process_focus = !lifecycle.process_focus;
                    if lifecycle.process_focus {
                        lifecycle.attached_session_focus = false;
                        lifecycle.workspace_focus = false;
                    }
                    stop_propagation();
                }
            } else if lifecycle.process_focus {
                match key.code {
                    KeyCode::Up => {
                        lifecycle.process_selected = lifecycle.process_selected.saturating_sub(1);
                        stop_propagation();
                    }
                    KeyCode::Down => {
                        if process_count > 0 {
                            lifecycle.process_selected =
                                (lifecycle.process_selected + 1).min(process_count - 1);
                        }
                        stop_propagation();
                    }
                    KeyCode::Char('d') | KeyCode::Delete => {
                        let _ = emitter_for_keys.emit_custom_event(
                            crate::event::CustomEvent::SessionSidebarIntent {
                                kind: crate::event::SessionSidebarIntentKind::KillSelectedProcess,
                            },
                        );
                        stop_propagation();
                    }
                    _ => {}
                }
            } else if lifecycle.workspace_focus {
                match key.code {
                    KeyCode::Up => {
                        let next_index = lifecycle.workspace_selected.saturating_sub(1);
                        render_state.set_workspace_selected_index(&mut lifecycle, next_index);
                        stop_propagation();
                    }
                    KeyCode::Down => {
                        let count = render_state.workspace_visible_count();
                        if count > 0 {
                            let next_index = (lifecycle.workspace_selected + 1).min(count - 1);
                            render_state.set_workspace_selected_index(&mut lifecycle, next_index);
                        }
                        stop_propagation();
                    }
                    KeyCode::Left => {
                        if render_state.collapse_selected_workspace_dir(&mut lifecycle) {
                            stop_propagation();
                        }
                    }
                    KeyCode::Right => {
                        if render_state.expand_selected_workspace_dir(&mut lifecycle) {
                            stop_propagation();
                        }
                    }
                    _ => {}
                }
            } else if lifecycle.attached_session_focus {
                match key.code {
                    KeyCode::Up => {
                        lifecycle.attached_session_selected =
                            lifecycle.attached_session_selected.saturating_sub(1);
                        stop_propagation();
                    }
                    KeyCode::Down => {
                        if !attached_sessions.is_empty() {
                            lifecycle.attached_session_selected =
                                (lifecycle.attached_session_selected + 1)
                                    .min(attached_sessions.len() - 1);
                        }
                        stop_propagation();
                    }
                    KeyCode::Enter => {
                        let selected = lifecycle.attached_session_selected;
                        if let Some(child) = attached_sessions.get(selected) {
                            let _ = emitter_for_keys.emit_custom_event(
                                crate::event::CustomEvent::SessionNavigationIntent {
                                    kind: crate::event::SessionNavigationIntentKind::Session(
                                        child.session_id.clone(),
                                    ),
                                },
                            );
                            stop_propagation();
                        }
                    }
                    _ => {}
                }
            }
        });

        let render_state_for_mouse = render_state_ref.clone();
        let lifecycle_for_mouse = lifecycle_ref.clone();
        let emitter_for_mouse = event_emitter.clone();
        let attached_sessions_for_mouse = self.render_inputs.snapshot.attached_sessions.clone();
        let chrome = self.chrome;
        use_mouse(move |mouse_event| {
            let mut render_state = render_state_for_mouse.lock();
            let mut lifecycle = lifecycle_for_mouse.lock();
            match mouse_event.kind {
                MouseEventKind::ScrollUp => {
                    if render_state.scroll_up_at(mouse_event.column, mouse_event.row) {
                        stop_propagation();
                    }
                }
                MouseEventKind::ScrollDown => {
                    if render_state.scroll_down_at(mouse_event.column, mouse_event.row) {
                        stop_propagation();
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if handle_sidebar_chrome_mouse_down(
                        chrome,
                        &mut render_state,
                        &mut lifecycle,
                        mouse_event.column,
                        mouse_event.row,
                    ) {
                        let pending_attached = render_state.take_pending_navigate_attached();
                        let pending_session = render_state.take_pending_navigate_session();
                        let pending_parent = render_state.take_pending_navigate_parent();

                        if let Some(session_id) = pending_session {
                            let _ = emitter_for_mouse.emit_custom_event(
                                crate::event::CustomEvent::SessionNavigationIntent {
                                    kind: crate::event::SessionNavigationIntentKind::Session(
                                        session_id,
                                    ),
                                },
                            );
                        }
                        if pending_parent {
                            let _ = emitter_for_mouse.emit_custom_event(
                                crate::event::CustomEvent::SessionNavigationIntent {
                                    kind: crate::event::SessionNavigationIntentKind::Parent,
                                },
                            );
                        }
                        if let Some(cs_idx) = pending_attached {
                            if let Some(child) = attached_sessions_for_mouse.get(cs_idx) {
                                let _ = emitter_for_mouse.emit_custom_event(
                                    crate::event::CustomEvent::SessionNavigationIntent {
                                        kind: crate::event::SessionNavigationIntentKind::Session(
                                            child.session_id.clone(),
                                        ),
                                    },
                                );
                            }
                        }
                        stop_propagation();
                    }
                }
                _ => {}
            }
        });
        if self.chrome.mode != SidebarChromeMode::Hidden {
            let mut surface = crate::ui::BufferSurface::new(buffer);
            self.sidebar.render_with_inputs_and_bg(
                &self.render_inputs,
                &mut surface,
                self.area,
                &mut render_state_ref.lock(),
                &mut lifecycle_ref.lock(),
                self.floating,
                self.bg_override,
            );
        }
        *self.state.lock() = render_state_ref.lock().clone();
        *self.lifecycle.lock() = lifecycle_ref.lock().clone();
    }
}

fn handle_sidebar_chrome_mouse_down(
    chrome: SidebarChromeProps,
    render_state: &mut SidebarRenderState,
    lifecycle: &mut SidebarLifecycleState,
    col: u16,
    row: u16,
) -> bool {
    if !point_in_rect(chrome.container_area, col, row) {
        return false;
    }

    match chrome.mode {
        SidebarChromeMode::Docked => {
            if point_in_optional_rect(chrome.close_button_area, col, row) {
                lifecycle.mode = SidebarMode::Hide;
                lifecycle.visible = false;
                lifecycle.process_focus = false;
                lifecycle.attached_session_focus = false;
                lifecycle.workspace_focus = false;
                return true;
            }
            if point_in_optional_rect(render_state.sidebar_area(), col, row) {
                render_state.handle_click(lifecycle, col, row)
            } else {
                false
            }
        }
        SidebarChromeMode::Overlay => {
            if point_in_optional_rect(chrome.close_button_area, col, row) {
                lifecycle.visible = false;
                lifecycle.process_focus = false;
                lifecycle.attached_session_focus = false;
                lifecycle.workspace_focus = false;
                return true;
            }
            if point_in_optional_rect(chrome.backdrop_area, col, row)
                && !point_in_optional_rect(render_state.sidebar_area(), col, row)
            {
                lifecycle.visible = false;
                lifecycle.process_focus = false;
                lifecycle.attached_session_focus = false;
                lifecycle.workspace_focus = false;
                return true;
            }
            if point_in_optional_rect(render_state.sidebar_area(), col, row) {
                render_state.handle_click(lifecycle, col, row)
            } else {
                lifecycle.visible = false;
                lifecycle.process_focus = false;
                lifecycle.attached_session_focus = false;
                lifecycle.workspace_focus = false;
                true
            }
        }
        SidebarChromeMode::Hidden => {
            if point_in_optional_rect(chrome.open_button_area, col, row) {
                lifecycle.mode = SidebarMode::Auto;
                lifecycle.visible = sidebar_default_visible(chrome.layout_width);
                true
            } else {
                false
            }
        }
    }
}

impl Sidebar {
    pub fn new(session_id: String) -> Self {
        Self { session_id }
    }

    pub(crate) fn capture_render_seed(context: &Arc<AppContext>, session_id: &str) -> SidebarRenderSeed {
        let session_ctx = context.session.read().data.clone();
        let graph_root_id = context.graph_root_session_id(session_id);
        SidebarRenderSeed {
            theme: context.theme.read().clone(),
            session_ctx,
            mcp_servers: context.mcp_servers.read().clone(),
            lsp_status: context.lsp_status.read().clone(),
            providers: context.providers.read().clone(),
            processes: context.processes.read().clone(),
            directory: context.directory.read().clone(),
            attached_sessions: context.attached_sessions_for(&graph_root_id),
            execution_topology: context.execution_topology_for(session_id),
            session_usage: context.session_usage_for(session_id),
            session_usage_books: context.session_usage_books_for(session_id),
            context_closure_contract: context.session_context_closure_contract_for(session_id),
            current_context_tokens: context.current_context_tokens_for(session_id),
        }
    }

    pub(crate) fn render_inputs_from_seed(seed: &SidebarRenderSeed) -> SidebarRenderInputs {
        SidebarRenderInputs {
            theme: seed.theme.clone(),
            snapshot: Self::render_snapshot_from_seed(seed),
        }
    }

    fn render_snapshot_from_seed(seed: &SidebarRenderSeed) -> SidebarRenderSnapshot {
        SidebarRenderSnapshot {
            session_ctx: seed.session_ctx.clone(),
            mcp_servers: seed.mcp_servers.clone(),
            lsp_status: seed.lsp_status.clone(),
            providers: seed.providers.clone(),
            processes: seed.processes.clone(),
            directory: seed.directory.clone(),
            attached_sessions: seed.attached_sessions.clone(),
            execution_topology: seed.execution_topology.clone(),
            session_usage: seed.session_usage.clone(),
            session_usage_books: seed.session_usage_books.clone(),
            context_closure_contract: seed.context_closure_contract.clone(),
            current_context_tokens: seed.current_context_tokens,
        }
    }

    pub(crate) fn render_surface<S: RenderSurface>(
        &self,
        render_inputs: &SidebarRenderInputs,
        surface: &mut S,
        area: Rect,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        floating: bool,
    ) {
        self.render_with_inputs_and_bg(
            render_inputs,
            surface,
            area,
            state,
            lifecycle,
            floating,
            None,
        );
    }

    pub(crate) fn render_with_inputs_and_bg<S: RenderSurface>(
        &self,
        render_inputs: &SidebarRenderInputs,
        surface: &mut S,
        area: Rect,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        floating: bool,
        bg_override: Option<ratatui::style::Color>,
    ) {
        if area.width == 0 || area.height == 0 {
            state.reset_hidden();
            return;
        }

        state.set_sidebar_area(area);
        let theme = render_inputs.theme.clone();
        let panel_bg = bg_override.unwrap_or(theme.background_panel);

        if !floating {
            let block = Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(panel_bg));
            surface.render_widget(block, area);
        }

        let inner = Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner.width == 0 || inner.height == 0 {
            state.reset_hidden();
            return;
        }

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(inner);

        self.render_tabs_with_bg(
            surface, layout[0], &theme, state, lifecycle, floating, panel_bg,
        );
        self.render_sections_with_bg(
            surface,
            layout[1],
            &theme,
            &render_inputs.snapshot,
            state,
            lifecycle,
            floating,
            panel_bg,
        );
        self.render_footer_with_bg(
            surface,
            layout[2],
            &theme,
            &render_inputs.snapshot.directory,
            floating,
            panel_bg,
        );
    }

    pub(crate) fn render_reactive(
        &self,
        render_inputs: SidebarRenderInputs,
        buffer: &mut Buffer,
        area: Rect,
        state: Arc<Mutex<SidebarRenderState>>,
        lifecycle: Arc<Mutex<SidebarLifecycleState>>,
        floating: bool,
        bg_override: Option<ratatui::style::Color>,
        chrome: SidebarChromeProps,
    ) {
        reratui::element::Element::component(SidebarComponent {
            sidebar: self.clone(),
            render_inputs,
            area,
            state,
            lifecycle,
            floating,
            bg_override,
            chrome,
        })
        .with_key(format!("sidebar:{}", self.session_id))
        .render(area, buffer);
    }

    fn render_tabs_with_bg<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        state: &mut SidebarRenderState,
        lifecycle: &SidebarLifecycleState,
        floating: bool,
        panel_bg: ratatui::style::Color,
    ) {
        if area.width == 0 || area.height == 0 {
            state.set_tab_layout(area, Vec::new());
            return;
        }

        let session_area = Rect {
            x: area.x,
            y: area.y,
            width: 9.min(area.width),
            height: area.height,
        };
        let workspace_x = session_area
            .x
            .saturating_add(session_area.width)
            .saturating_add(1);
        let workspace_width = area.right().saturating_sub(workspace_x).min(11);
        let workspace_area = Rect {
            x: workspace_x,
            y: area.y,
            width: workspace_width,
            height: area.height,
        };
        let tab_hits = vec![
            SidebarTabHit {
                tab: SidebarTab::Session,
                area: session_area,
            },
            SidebarTabHit {
                tab: SidebarTab::Workspace,
                area: workspace_area,
            },
        ];
        state.set_tab_layout(area, tab_hits);

        if !floating {
            surface.render_widget(
                Paragraph::new("").style(Style::default().bg(panel_bg)),
                area,
            );
        }

        let tabs = Tabs::new(vec!["Session", "Workspace"])
            .select(match lifecycle.active_tab {
                SidebarTab::Session => 0,
                SidebarTab::Workspace => 1,
            })
            .style(Style::default().fg(theme.text_muted).bg(panel_bg))
            .highlight_style(
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.background_element)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::styled(" ", Style::default().bg(panel_bg)));
        surface.render_widget(tabs, area);
    }

    fn render_sections_with_bg<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        snapshot: &SidebarRenderSnapshot,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        floating: bool,
        panel_bg: ratatui::style::Color,
    ) {
        if area.width == 0 || area.height == 0 {
            state.set_sections_layout(area, 0, Vec::new());
            return;
        }

        let sections: Vec<SidebarSection> = if lifecycle.active_tab == SidebarTab::Workspace {
            state.session_graph_root_id = None;
            state.session_graph_active_id = None;
            self.build_workspace_sections(area, theme, snapshot, state, lifecycle)
        } else {
            let session = snapshot.session_ctx.sessions.get(&self.session_id);
            let messages = snapshot
                .session_ctx
                .messages
                .get(&self.session_id)
                .cloned()
                .unwrap_or_default();

            let title = session
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "New Session".to_string());
            let graph_root_session = session
                .and_then(|session| session.parent_id.as_ref())
                .and_then(|parent_id| snapshot.session_ctx.sessions.get(parent_id))
                .or(session);
            let graph_root_id = graph_root_session
                .map(|session| session.id.as_str())
                .unwrap_or(self.session_id.as_str());
            let graph_root_title = graph_root_session
                .map(|session| session.title.as_str())
                .unwrap_or(title.as_str());
            state.session_graph_root_id = Some(graph_root_id.to_string());
            state.session_graph_active_id = Some(self.session_id.clone());
            let mut session_lines = vec![Line::from(Span::styled(
                truncate_text(&title, area.width as usize),
                Style::default().fg(theme.text).bold(),
            ))];
            if let Some(session_meta) = session.and_then(|s| s.metadata.as_ref()) {
                if let Some(agent) = sidebar_metadata_text(session_meta, "agent") {
                    session_lines.push(sidebar_meta_line(theme, "agent", agent));
                }
                if let Some(model) = sidebar_model_summary(session_meta) {
                    session_lines.push(sidebar_meta_line(theme, "model", model));
                }
                if let Some(scheduler) = sidebar_scheduler_summary(session_meta) {
                    session_lines.push(sidebar_meta_line(theme, "scheduler", scheduler));
                }
            }
            let mut sections: Vec<SidebarSection> = vec![SidebarSection {
                key: "session",
                title: "Session",
                lines: session_lines,
                meter: None,
                attached_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: None,
                collapsible: false,
            }];

            if let Some(share) = session.and_then(|s| s.share.as_ref()) {
                sections.push(SidebarSection {
                    key: "share",
                    title: "Share",
                    lines: vec![Line::from(Span::styled(
                        truncate_text(&share.url, area.width as usize),
                        Style::default().fg(theme.info),
                    ))],
                    meter: None,
                    attached_session_hit_rows: None,
                    workspace_hit_rows: None,
                    summary: None,
                    collapsible: false,
                });
            }

            let message_fold = fold_messages(messages.as_slice());
            let resolved_usage = resolve_usage(
                snapshot.session_usage_books.as_ref(),
                snapshot.session_usage.as_ref(),
                Some(&message_fold),
            );
            let active_model = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::Assistant))
                .and_then(|m| m.model.as_deref());
            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::Assistant));
            let last_user = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User));
            let current_context_tokens = snapshot.current_context_tokens;
            let active_model_info = resolve_model_info_from_providers(&snapshot.providers, active_model);
            sections.push(SidebarSection {
                key: "context",
                title: "Usage",
                lines: {
                    let mut lines = Vec::new();
                    let context_limit =
                        active_model_info.as_ref().map(|model| model.context_window);
                    if let Some(shown_context_tokens) = current_context_tokens {
                        lines.push(sidebar_usage_line(
                            &theme,
                            "Current",
                            shown_context_tokens,
                            context_limit,
                        ));
                        if let Some(note) = context_limit
                            .and_then(|limit| context_usage_percent(shown_context_tokens, limit))
                            .and_then(|percent| {
                                agendao_types::context_pressure_label(Some(percent))
                            })
                        {
                            let note_style = context_usage_style(
                                &theme,
                                context_usage_percent(
                                    shown_context_tokens,
                                    context_limit.unwrap_or(0),
                                ),
                            );
                            lines.push(Line::from(vec![
                                Span::styled("State  ", Style::default().fg(theme.text_muted)),
                                Span::styled(note, note_style),
                            ]));
                        }
                    }
                    if resolved_usage.total_tokens > 0 {
                        lines.push(Line::from(vec![
                            Span::styled("Workflow ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!(
                                    "{} cumulative",
                                    format_compact_number(resolved_usage.total_tokens)
                                ),
                                Style::default().fg(theme.text),
                            ),
                        ]));
                    }
                    if resolved_usage.cache_read_tokens > 0
                        || resolved_usage.cache_miss_tokens > 0
                        || resolved_usage.cache_write_tokens > 0
                    {
                        lines.push(Line::from(vec![
                            Span::styled("Cache  ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!(
                                    "H/M/W {} / {} / {}",
                                    format_compact_number(resolved_usage.cache_read_tokens),
                                    format_compact_number(resolved_usage.cache_miss_tokens),
                                    format_compact_number(resolved_usage.cache_write_tokens)
                                ),
                                Style::default().fg(theme.text),
                            ),
                        ]));
                    }
                    if let Some(turn) = last_assistant.filter(|message| {
                        message.tokens.input > 0
                            || message.tokens.output > 0
                            || message.tokens.reasoning > 0
                            || message.tokens.cache_read > 0
                            || message.tokens.cache_miss > 0
                            || message.tokens.cache_write > 0
                    }) {
                        lines.push(Line::from(vec![
                            Span::styled("Turn   ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!(
                                    "↑{}  ↓{}",
                                    format_compact_number(turn.tokens.input),
                                    format_compact_number(turn.tokens.output)
                                ),
                                Style::default().fg(theme.text),
                            ),
                        ]));
                        if turn.tokens.reasoning > 0 {
                            lines.push(Line::from(vec![
                                Span::styled("Reason ", Style::default().fg(theme.text_muted)),
                                Span::styled(
                                    format_compact_number(turn.tokens.reasoning),
                                    Style::default().fg(theme.text),
                                ),
                            ]));
                        }
                        if turn.tokens.cache_read > 0
                            || turn.tokens.cache_miss > 0
                            || turn.tokens.cache_write > 0
                        {
                            lines.push(Line::from(vec![
                                Span::styled("Cache  ", Style::default().fg(theme.text_muted)),
                                Span::styled(
                                    format!(
                                        "H/M/W {} / {} / {}",
                                        format_compact_number(turn.tokens.cache_read),
                                        format_compact_number(turn.tokens.cache_miss),
                                        format_compact_number(turn.tokens.cache_write)
                                    ),
                                    Style::default().fg(theme.text),
                                ),
                            ]));
                        }
                    }
                    let cache_diagnostic = snapshot
                        .context_closure_contract
                        .as_ref()
                        .and_then(context_closure_cache_diagnostic_label)
                        .or_else(|| {
                            last_assistant
                                .and_then(|message| cache_diagnostic_label(&message.metadata))
                        });
                    if let Some(cache_diagnostic) = cache_diagnostic {
                        lines.push(Line::from(vec![
                            Span::styled("Cache  ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                truncate_text(&cache_diagnostic, area.width as usize),
                                Style::default().fg(theme.warning),
                            ),
                        ]));
                    }
                    if let Some(provider_diagnostic) = last_assistant
                        .and_then(|message| provider_diagnostic_label(&message.metadata))
                    {
                        lines.push(Line::from(vec![
                            Span::styled("Provider", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!(
                                    " {}",
                                    truncate_text(&provider_diagnostic, area.width as usize)
                                ),
                                Style::default().fg(theme.warning),
                            ),
                        ]));
                    }
                    if let Some(ingress_diagnostic) =
                        last_user.and_then(|message| ingress_diagnostic_label(&message.metadata))
                    {
                        lines.push(Line::from(vec![
                            Span::styled("Ingress", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!(
                                    " {}",
                                    truncate_text(&ingress_diagnostic, area.width as usize)
                                ),
                                Style::default().fg(theme.text_muted),
                            ),
                        ]));
                    }
                    if let Some(model) = active_model_info.as_ref() {
                        if let (Some(input_price), Some(output_price)) =
                            (model.cost_per_million_input, model.cost_per_million_output)
                        {
                            lines.push(Line::from(vec![
                                Span::styled("Price  ", Style::default().fg(theme.text_muted)),
                                Span::styled(
                                    format_price_pair(input_price, output_price),
                                    Style::default().fg(theme.text),
                                ),
                            ]));
                        }
                    }
                    lines.push(Line::from(vec![
                        Span::styled("Cost   ", Style::default().fg(theme.text_muted)),
                        Span::styled(
                            format!("${:.4}", resolved_usage.total_cost),
                            Style::default().fg(theme.text),
                        ),
                    ]));
                    lines
                },
                meter: current_context_tokens
                    .zip(active_model_info.as_ref().map(|model| model.context_window))
                    .and_then(|(used, limit)| {
                        (limit > 0).then(|| SidebarSectionMeter {
                            label: "Meter".to_string(),
                            ratio: (used as f64 / limit as f64).clamp(0.0, 1.0),
                            style: context_usage_style(
                                theme,
                                context_usage_percent(used, limit),
                            ),
                        })
                    }),
                attached_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: None,
                collapsible: false,
            });

            let connected_mcp = snapshot
                .mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::Connected))
                .count();
            let failed_mcp = snapshot
                .mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::Failed))
                .count();
            let registration_needed_mcp = snapshot
                .mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::NeedsClientRegistration))
                .count();
            let problematic_mcp = failed_mcp + registration_needed_mcp;
            let mut mcp_lines: Vec<Line<'static>> = Vec::new();
            if snapshot.mcp_servers.is_empty() {
                mcp_lines.push(Line::from(Span::styled(
                    "No MCP servers",
                    Style::default().fg(theme.text_muted),
                )));
            } else {
                for server in snapshot.mcp_servers.iter() {
                    let (status_text, color) = match server.status {
                        McpConnectionStatus::Connected => ("connected", theme.success),
                        McpConnectionStatus::Failed => ("failed", theme.error),
                        McpConnectionStatus::NeedsAuth => ("needs auth", theme.warning),
                        McpConnectionStatus::NeedsClientRegistration => {
                            ("needs client ID", theme.warning)
                        }
                        McpConnectionStatus::Disabled => ("disabled", theme.text_muted),
                        McpConnectionStatus::Disconnected => ("disconnected", theme.text_muted),
                    };
                    mcp_lines.push(Line::from(vec![
                        Span::styled("• ", Style::default().fg(color)),
                        Span::styled(
                            truncate_text(&server.name, area.width.saturating_sub(14) as usize),
                            Style::default().fg(theme.text),
                        ),
                        Span::styled(
                            format!(" {}", status_text),
                            Style::default().fg(theme.text_muted),
                        ),
                    ]));
                }
            }
            sections.push(SidebarSection {
                key: "mcp",
                title: "MCP",
                lines: mcp_lines,
                meter: None,
                attached_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: Some(format!(
                    "{} active, {} errors",
                    connected_mcp, problematic_mcp
                )),
                collapsible: snapshot.mcp_servers.len() > 2,
            });

            let connected_lsp = snapshot
                .lsp_status
                .iter()
                .filter(|s| matches!(s.status, LspConnectionStatus::Connected))
                .count();
            let errored_lsp = snapshot
                .lsp_status
                .iter()
                .filter(|s| matches!(s.status, LspConnectionStatus::Error))
                .count();
            let mut lsp_lines: Vec<Line<'static>> = Vec::new();
            if snapshot.lsp_status.is_empty() {
                lsp_lines.push(Line::from(Span::styled(
                    "No active LSP",
                    Style::default().fg(theme.text_muted),
                )));
            } else {
                for server in snapshot.lsp_status.iter() {
                    let (status_text, color) = match server.status {
                        LspConnectionStatus::Connected => ("connected", theme.success),
                        LspConnectionStatus::Error => ("error", theme.error),
                    };
                    lsp_lines.push(Line::from(vec![
                        Span::styled("• ", Style::default().fg(color)),
                        Span::styled(
                            truncate_text(&server.id, area.width.saturating_sub(14) as usize),
                            Style::default().fg(theme.text),
                        ),
                        Span::styled(
                            format!(" {}", status_text),
                            Style::default().fg(theme.text_muted),
                        ),
                    ]));
                }
            }
            sections.push(SidebarSection {
                key: "lsp",
                title: "LSP",
                lines: lsp_lines,
                meter: None,
                attached_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: Some(format!(
                    "{} connected, {} errors",
                    connected_lsp, errored_lsp
                )),
                collapsible: snapshot.lsp_status.len() > 2,
            });

            if let Some(todos) = snapshot.session_ctx.todos.get(&self.session_id) {
                let pending = todos
                    .iter()
                    .filter(|todo| {
                        !matches!(todo.status, TodoStatus::Completed | TodoStatus::Cancelled)
                    })
                    .collect::<Vec<_>>();
                if !pending.is_empty() {
                    let mut todo_lines: Vec<Line<'static>> = Vec::new();
                    for todo in pending.iter().take(5) {
                        todo_lines.push(Line::from(vec![
                            Span::styled("☐ ", Style::default().fg(theme.warning)),
                            Span::styled(
                                truncate_text(&todo.content, area.width.saturating_sub(2) as usize),
                                Style::default().fg(theme.text_muted),
                            ),
                        ]));
                    }
                    sections.push(SidebarSection {
                        key: "todo",
                        title: "Todo",
                        lines: todo_lines,
                        meter: None,
                        attached_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(format!("{} pending", pending.len())),
                        collapsible: pending.len() > 2,
                    });
                }
            }

            if let Some(entries) = snapshot.session_ctx.session_diff.get(&self.session_id) {
                if !entries.is_empty() {
                    let mut file_lines: Vec<Line<'static>> = Vec::new();
                    for entry in entries.iter().take(8) {
                        file_lines.push(Line::from(vec![
                            Span::styled(
                                truncate_text(&entry.file, area.width.saturating_sub(14) as usize),
                                Style::default().fg(theme.text),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                format!("+{}", entry.additions),
                                Style::default().fg(theme.success),
                            ),
                            Span::raw("/"),
                            Span::styled(
                                format!("-{}", entry.deletions),
                                Style::default().fg(theme.error),
                            ),
                        ]));
                    }
                    sections.push(SidebarSection {
                        key: "diff",
                        title: "Modified Files",
                        lines: file_lines,
                        meter: None,
                        attached_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(format!("{} files changed", entries.len())),
                        collapsible: entries.len() > 2,
                    });
                }
            }

            // Processes section
            let proc_list = snapshot.processes.clone();
            clamp_sidebar_process_selection(lifecycle, proc_list.len());
            if !proc_list.is_empty() {
                let mut proc_lines: Vec<Line<'static>> = Vec::new();
                for (idx, proc) in proc_list.iter().enumerate() {
                    let selected = lifecycle.process_focus && idx == lifecycle.process_selected;
                    let prefix = if selected { "▸ " } else { "  " };
                    let kind_color = match proc.kind {
                        ProcessKind::Plugin => theme.info,
                        ProcessKind::Bash => theme.success,
                        ProcessKind::Agent => theme.warning,
                        ProcessKind::Mcp => theme.info, // Same category as Plugin
                        ProcessKind::Lsp => theme.warning, // Same category as Agent
                    };
                    let name_width = area.width.saturating_sub(18) as usize;
                    let stats = format!("{:4.1}% {:>3}MB", proc.cpu_percent, proc.memory_kb / 1024);
                    let fg = if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    };
                    let row_bg = if selected {
                        Some(theme.background_element)
                    } else {
                        None
                    };
                    let mk_style = |base: Style| -> Style {
                        if let Some(bg) = row_bg {
                            base.bg(bg)
                        } else {
                            base
                        }
                    };
                    proc_lines.push(Line::from(vec![
                        Span::styled(
                            prefix,
                            mk_style(Style::default().fg(if selected {
                                theme.primary
                            } else {
                                theme.text_muted
                            })),
                        ),
                        Span::styled("• ", mk_style(Style::default().fg(kind_color))),
                        Span::styled(
                            truncate_text(&proc.name, name_width),
                            mk_style(Style::default().fg(fg)),
                        ),
                        Span::styled(
                            format!(" {}", stats),
                            mk_style(Style::default().fg(theme.text_muted)),
                        ),
                    ]));
                }
                sections.push(SidebarSection {
                    key: "processes",
                    title: "Processes",
                    lines: proc_lines,
                    meter: None,
                    attached_session_hit_rows: None,
                    workspace_hit_rows: None,
                    summary: Some(format!("{} running", proc_list.len())),
                    collapsible: proc_list.len() > 2,
                });
            }

            // Agents section — sourced from execution topology (server-side)
            {
                let agent_nodes = collect_agent_nodes_from_topology(&snapshot.execution_topology);
                if !agent_nodes.is_empty() {
                    let mut agent_lines: Vec<Line<'static>> = Vec::new();
                    let mut running = 0usize;
                    let mut done = 0usize;
                    for (label, status) in &agent_nodes {
                        let (symbol, color) = match status {
                            crate::api::ExecutionStatus::Running => {
                                running += 1;
                                ("◐", theme.info)
                            }
                            crate::api::ExecutionStatus::Waiting => ("◯", theme.warning),
                            crate::api::ExecutionStatus::Done => {
                                done += 1;
                                ("✓", theme.success)
                            }
                            _ => ("•", theme.text_muted),
                        };
                        let name_width = area.width.saturating_sub(12) as usize;
                        agent_lines.push(Line::from(vec![
                            Span::styled(format!("{} ", symbol), Style::default().fg(color)),
                            Span::styled(
                                truncate_text(label, name_width),
                                Style::default().fg(theme.text),
                            ),
                        ]));
                    }
                    let summary = if done > 0 && running > 0 {
                        format!("{} running, {} done", running, done)
                    } else if running > 0 {
                        format!("{} running", running)
                    } else {
                        format!("{} done", done)
                    };
                    sections.push(SidebarSection {
                        key: "agents",
                        title: "Agents",
                        lines: agent_lines,
                        meter: None,
                        attached_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(summary),
                        collapsible: agent_nodes.len() > 3,
                    });
                }
            }

            // Session Graph section
            let child_list = snapshot.attached_sessions.clone();
            clamp_sidebar_attached_session_selection(lifecycle, child_list.len());
            if !child_list.is_empty() {
                let current_child_index = child_list
                    .iter()
                    .position(|child| child.session_id == self.session_id);
                let selected_child_index = if lifecycle.attached_session_focus {
                    Some(
                        lifecycle
                            .attached_session_selected
                            .min(child_list.len() - 1),
                    )
                } else {
                    current_child_index
                };
                let selected_child = selected_child_index.and_then(|index| child_list.get(index));
                let (cs_lines, attached_session_hit_rows) = build_session_graph_lines(
                    theme,
                    area.width,
                    graph_root_title,
                    graph_root_id,
                    &self.session_id,
                    &child_list,
                    &snapshot.session_ctx.sessions,
                    &snapshot.session_ctx.session_diff,
                    lifecycle,
                    selected_child,
                );
                sections.push(SidebarSection {
                    key: "session_graph",
                    title: "Session Graph",
                    lines: cs_lines,
                    meter: None,
                    attached_session_hit_rows: Some(attached_session_hit_rows),
                    workspace_hit_rows: None,
                    summary: Some(format!("{} sessions", child_list.len())),
                    collapsible: child_list.len() > 2,
                });
            }

            sections
        };

        let document = build_sidebar_sections_document(theme, state, sections);
        let rows = document.rows;

        let has_overflow = rows.len() > usize::from(area.height);
        let sections_text_area = if has_overflow && area.width > 1 {
            Rect {
                x: area.x,
                y: area.y,
                width: area.width.saturating_sub(1),
                height: area.height,
            }
        } else {
            area
        };
        let scrollbar_area = if has_overflow && area.width > 1 {
            Some(Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y,
                width: 1,
                height: area.height,
            })
        } else {
            None
        };

        state.set_sections_layout(sections_text_area, rows.len(), document.toggle_hits);
        state.process_line_hits = document.process_line_hits;
        state.attached_session_line_hits = document.attached_session_line_hits;
        state.workspace_line_hits = document.workspace_line_hits;

        let items = rows
            .iter()
            .map(|row| match row {
                SidebarSectionRow::Line(line) => ListItem::new(line.clone()),
                SidebarSectionRow::Gauge(meter) => ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default().fg(theme.border_subtle)),
                    Span::styled(
                        format!("{:<7}", meter.label),
                        Style::default().fg(theme.text_muted),
                    ),
                ])),
            })
            .collect::<Vec<_>>();
        let list = List::new(items).style(if floating {
            Style::default()
        } else {
            Style::default().bg(panel_bg)
        });
        let mut list_state = ListState::default().with_offset(state.scroll_offset);
        surface.render_stateful_widget(list, sections_text_area, &mut list_state);

        let visible_height = usize::from(sections_text_area.height);
        let start = state.scroll_offset.min(rows.len());
        let end = (start + visible_height).min(rows.len());
        for (visible_idx, row) in rows[start..end].iter().enumerate() {
            let SidebarSectionRow::Gauge(meter) = row else {
                continue;
            };
            let row_area = Rect {
                x: sections_text_area.x.saturating_add(9),
                y: sections_text_area.y.saturating_add(visible_idx as u16),
                width: sections_text_area.width.saturating_sub(10),
                height: 1,
            };
            if row_area.width == 0 {
                continue;
            }
            let gauge = Gauge::default()
                .ratio(meter.ratio.clamp(0.0, 1.0))
                .gauge_style(meter.style)
                .style(if floating {
                    Style::default()
                } else {
                    Style::default().bg(panel_bg)
                })
                .use_unicode(true);
            surface.render_widget(gauge, row_area);
        }

        if let Some(scroll_area) = scrollbar_area {
            let mut scrollbar_state = ScrollbarState::new(state.content_lines)
                .position(state.scroll_offset)
                .viewport_content_length(state.viewport_lines.max(1));
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::default().fg(theme.border_subtle))
                .thumb_symbol("█")
                .thumb_style(Style::default().fg(theme.primary));
            surface.render_stateful_widget(scrollbar, scroll_area, &mut scrollbar_state);
        }
        self.render_workspace_tooltip(surface, sections_text_area, theme, state);
    }

    fn build_workspace_sections(
        &self,
        area: Rect,
        theme: &Theme,
        snapshot: &SidebarRenderSnapshot,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
    ) -> Vec<SidebarSection> {
        let workspace_root = workspace_root_path(&snapshot.directory);
        state.refresh_workspace_index(&workspace_root);
        if state.workspace_seeded_root.as_deref() != Some(snapshot.directory.as_str()) {
            state.workspace_expanded_dirs =
                top_level_workspace_dirs(state.workspace_index.entries());
            state.workspace_seeded_root = Some(snapshot.directory.clone());
            state.workspace_selected_path = None;
            state.workspace_tooltip = None;
        }

        let modified_paths = snapshot
            .session_ctx
            .session_diff
            .get(&self.session_id)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| normalize_workspace_path(&workspace_root, &entry.file))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let current_path = snapshot
            .session_ctx
            .session_diff
            .get(&self.session_id)
            .and_then(|entries| entries.last())
            .and_then(|entry| normalize_workspace_path(&workspace_root, &entry.file));
        let reveal_path = state
            .workspace_selected_path
            .as_deref()
            .or(current_path.as_deref());
        let visible_nodes = build_workspace_visible_nodes(
            state.workspace_index.entries(),
            &state.workspace_expanded_dirs,
            &modified_paths,
            current_path.as_deref(),
            reveal_path,
        );
        state.set_workspace_visible_nodes(visible_nodes);
        state.sync_workspace_selection(lifecycle, current_path.as_deref());
        clamp_sidebar_workspace_selection(lifecycle, state.workspace_visible_count());

        let (root_prefix, root_leaf) = split_path_segments(snapshot.directory.as_str());
        let workspace_label = if root_leaf.is_empty() {
            snapshot.directory.clone()
        } else {
            root_leaf
        };
        let tree_summary = format!(
            "{} indexed · {} touched",
            state.workspace_index.entries().len(),
            modified_paths.len()
        );
        let mut sections = vec![SidebarSection {
            key: "workspace_root",
            title: "Workspace",
            lines: vec![
                Line::from(vec![
                    Span::styled(root_prefix, Style::default().fg(theme.text_muted)),
                    Span::styled(workspace_label, Style::default().fg(theme.text).bold()),
                ]),
                Line::from(vec![Span::styled(
                    tree_summary.clone(),
                    Style::default().fg(theme.text_muted),
                )]),
            ],
            meter: None,
            attached_session_hit_rows: None,
            workspace_hit_rows: None,
            summary: None,
            collapsible: false,
        }];
        let (tree_lines, workspace_hit_rows) = build_workspace_tree_lines(
            theme,
            area.width,
            &state.workspace_visible_nodes,
            lifecycle,
        );
        sections.push(SidebarSection {
            key: "workspace_tree",
            title: "Files",
            lines: tree_lines,
            meter: None,
            attached_session_hit_rows: None,
            workspace_hit_rows: Some(workspace_hit_rows),
            summary: Some(tree_summary),
            collapsible: false,
        });
        sections
    }

    fn render_workspace_tooltip<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        state: &SidebarRenderState,
    ) {
        let Some(text) = state.workspace_tooltip.as_ref() else {
            return;
        };
        let Some(selected_path) = state.workspace_selected_path.as_ref() else {
            return;
        };
        let Some((line_index, _)) = state.workspace_line_hits.iter().find(|(_, idx)| {
            state
                .workspace_visible_nodes
                .get(*idx)
                .map(|node| &node.path == selected_path)
                .unwrap_or(false)
        }) else {
            return;
        };
        if *line_index < state.scroll_offset {
            return;
        }
        let visible_row = line_index.saturating_sub(state.scroll_offset) as u16;
        if visible_row >= area.height {
            return;
        }

        let popup_width = area.width.saturating_sub(1).clamp(18, area.width);
        if popup_width < 8 {
            return;
        }
        let popup_height = 3.min(area.height.max(1));
        let popup_y = if visible_row.saturating_add(popup_height) < area.height {
            area.y.saturating_add(visible_row.saturating_add(1))
        } else {
            area.y
                .saturating_add(visible_row.saturating_sub(popup_height.saturating_sub(1)))
        };
        let popup = Rect {
            x: area.x,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };
        surface.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.primary))
            .style(Style::default().bg(theme.background_panel));
        let inner = block.inner(popup);
        surface.render_widget(block, popup);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        surface.render_widget(
            Paragraph::new(text.clone())
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(theme.text).bg(theme.background_panel)),
            inner,
        );
    }

    fn render_footer_with_bg<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        directory: &str,
        floating: bool,
        panel_bg: ratatui::style::Color,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let (prefix, leaf) = split_path_segments(directory);
        let lines = vec![
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(theme.text_muted)),
                Span::styled(leaf, Style::default().fg(theme.text)),
            ]),
            Line::from(vec![
                Span::styled("• ", Style::default().fg(theme.success)),
                Span::styled(
                    format!("{} ({}) ", APP_NAME, APP_SHORT_NAME),
                    Style::default().fg(theme.text).bold(),
                ),
                Span::styled(APP_VERSION_DATE, Style::default().fg(theme.text_muted)),
            ]),
        ];

        let mut paragraph = Paragraph::new(lines);
        if !floating {
            paragraph = paragraph.style(Style::default().bg(panel_bg));
        }
        surface.render_widget(paragraph, area);
    }
}

fn contains_point(area: Option<Rect>, col: u16, row: u16) -> bool {
    let Some(area) = area else {
        return false;
    };
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    col >= area.x && col < max_x && row >= area.y && row < max_y
}

fn point_in_optional_rect(area: Option<Rect>, col: u16, row: u16) -> bool {
    contains_point(area, col, row)
}

fn point_in_rect(area: Rect, col: u16, row: u16) -> bool {
    contains_point(Some(area), col, row)
}

fn resolve_model_info_from_providers(
    providers: &[ProviderInfo],
    model_ref: Option<&str>,
) -> Option<crate::state::ModelInfo> {
    let model_ref = model_ref?.trim();
    if model_ref.is_empty() {
        return None;
    }

    if let Some((provider_id, model_id)) = model_ref.split_once('/') {
        return providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .and_then(|provider| provider.models.iter().find(|model| model.id == model_id))
            .cloned();
    }

    providers
        .iter()
        .flat_map(|provider| provider.models.iter())
        .find(|model| model.id == model_ref || model.name == model_ref)
        .cloned()
}

#[derive(Clone, Default, PartialEq, Eq)]
struct WorkspaceTreeDir {
    dirs: BTreeMap<String, WorkspaceTreeDir>,
    files: Vec<String>,
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_chars + 1);
    for ch in text.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn workspace_root_path(directory: &str) -> PathBuf {
    if directory.trim().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(directory)
    }
}

fn normalize_workspace_path(root: &PathBuf, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if PathBuf::from(trimmed).is_absolute() {
        let absolute = PathBuf::from(trimmed);
        absolute
            .strip_prefix(root)
            .ok()
            .map(|relative| relative.to_string_lossy().to_string())?
    } else {
        trimmed.to_string()
    };
    let normalized = normalized
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string();
    (!normalized.is_empty()).then_some(normalized)
}

fn workspace_parent_path(path: &str) -> Option<String> {
    path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

fn build_workspace_visible_nodes(
    entries: &[String],
    expanded_dirs: &HashSet<String>,
    modified_paths: &HashSet<String>,
    current_path: Option<&str>,
    reveal_path: Option<&str>,
) -> Vec<WorkspaceVisibleNode> {
    let tree = build_workspace_tree(entries);
    let mut nodes = Vec::new();
    flatten_workspace_tree(
        &tree,
        None,
        0,
        expanded_dirs,
        modified_paths,
        current_path,
        reveal_path,
        &mut nodes,
    );
    nodes
}

fn build_workspace_tree(entries: &[String]) -> WorkspaceTreeDir {
    let mut root = WorkspaceTreeDir::default();
    for entry in entries {
        let segments = entry
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        insert_workspace_entry(&mut root, &segments);
    }
    root
}

fn top_level_workspace_dirs(entries: &[String]) -> HashSet<String> {
    entries
        .iter()
        .filter_map(|entry| entry.split('/').next())
        .filter(|segment| {
            !segment.is_empty()
                && entries
                    .iter()
                    .any(|entry| entry.starts_with(&format!("{segment}/")))
        })
        .map(ToOwned::to_owned)
        .collect()
}

fn insert_workspace_entry(node: &mut WorkspaceTreeDir, segments: &[&str]) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        node.files.push((*first).to_string());
        return;
    }
    insert_workspace_entry(node.dirs.entry((*first).to_string()).or_default(), rest);
}

#[allow(clippy::too_many_arguments)]
fn flatten_workspace_tree(
    tree: &WorkspaceTreeDir,
    parent_path: Option<&str>,
    depth: usize,
    expanded_dirs: &HashSet<String>,
    modified_paths: &HashSet<String>,
    current_path: Option<&str>,
    reveal_path: Option<&str>,
    out: &mut Vec<WorkspaceVisibleNode>,
) {
    for (dir_name, child) in &tree.dirs {
        let path = join_workspace_path(parent_path, dir_name);
        let expanded = expanded_dirs.contains(&path)
            || reveal_path
                .map(|candidate| candidate.starts_with(&(path.clone() + "/")))
                .unwrap_or(false);
        out.push(WorkspaceVisibleNode {
            path: path.clone(),
            label: dir_name.clone(),
            depth,
            is_dir: true,
            expanded,
            is_modified: modified_paths
                .iter()
                .any(|entry| entry.starts_with(&(path.clone() + "/"))),
            is_current: current_path
                .map(|candidate| candidate.starts_with(&(path.clone() + "/")))
                .unwrap_or(false),
        });
        if expanded {
            flatten_workspace_tree(
                child,
                Some(&path),
                depth + 1,
                expanded_dirs,
                modified_paths,
                current_path,
                reveal_path,
                out,
            );
        }
    }
    for file_name in &tree.files {
        let path = join_workspace_path(parent_path, file_name);
        out.push(WorkspaceVisibleNode {
            path: path.clone(),
            label: file_name.clone(),
            depth,
            is_dir: false,
            expanded: false,
            is_modified: modified_paths.contains(&path),
            is_current: current_path == Some(path.as_str()),
        });
    }
}

fn join_workspace_path(parent_path: Option<&str>, segment: &str) -> String {
    parent_path
        .map(|parent| format!("{parent}/{segment}"))
        .unwrap_or_else(|| segment.to_string())
}

fn build_workspace_tree_lines(
    theme: &Theme,
    width: u16,
    nodes: &[WorkspaceVisibleNode],
    lifecycle: &SidebarLifecycleState,
) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    if nodes.is_empty() {
        return (
            vec![Line::from(Span::styled(
                "No indexed files yet",
                Style::default().fg(theme.text_muted),
            ))],
            vec![None],
        );
    }

    let mut lines = Vec::new();
    let mut hit_rows = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        let selected = lifecycle.workspace_focus && lifecycle.workspace_selected == idx;
        let row_bg = selected.then_some(theme.background_element);
        let mk_style = |style: Style| row_bg.map_or(style, |bg| style.bg(bg));
        let indent = "  ".repeat(node.depth.min(6));
        let caret = if node.is_dir {
            if node.expanded {
                "▾"
            } else {
                "▸"
            }
        } else {
            "•"
        };
        let accent = if node.is_current {
            theme.primary
        } else if node.is_modified {
            theme.success
        } else if node.is_dir {
            theme.text
        } else {
            theme.text_muted
        };
        let suffix = if node.is_current {
            Some(" current")
        } else if node.is_modified {
            Some(" touched")
        } else {
            None
        };
        let reserved = indent.len() + 6 + suffix.map_or(0, str::len);
        let label = truncate_text(&node.label, width.saturating_sub(reserved as u16) as usize);
        let mut spans = vec![
            Span::styled(indent, mk_style(Style::default().fg(theme.text_muted))),
            Span::styled(
                format!("{caret} "),
                mk_style(Style::default().fg(if selected { theme.primary } else { accent })),
            ),
            Span::styled(label, mk_style(Style::default().fg(accent))),
        ];
        if let Some(suffix) = suffix {
            spans.push(Span::styled(
                suffix,
                mk_style(Style::default().fg(theme.text_muted)),
            ));
        }
        lines.push(Line::from(spans));
        hit_rows.push(Some(idx));
    }
    (lines, hit_rows)
}

fn workspace_popup_text(
    sections_area: Option<Rect>,
    node: &WorkspaceVisibleNode,
) -> Option<String> {
    let area = sections_area?;
    let available_width = usize::from(area.width.saturating_sub(4)).max(1);
    let needs_popup = node.path.chars().count() > available_width
        || node.label.chars().count()
            > available_width.saturating_sub(node.depth.saturating_mul(2));
    needs_popup.then(|| node.path.clone())
}

pub(crate) fn sidebar_metadata_text(
    metadata: &HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn sidebar_metadata_bool(metadata: &HashMap<String, serde_json::Value>, key: &str) -> bool {
    metadata
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn cache_diagnostic_label(metadata: &Option<HashMap<String, serde_json::Value>>) -> Option<String> {
    metadata
        .as_ref()
        .and_then(agendao_provider::cache::cache_evidence_from_metadata)
        .and_then(|summary| cache_evidence_status_label(&summary))
}

fn context_closure_cache_diagnostic_label(
    contract: &agendao_types::SessionContextClosureContract,
) -> Option<String> {
    contract.coarse_diagnostic_label()
}

fn cache_evidence_status_label(
    summary: &agendao_provider::cache::CacheEvidenceSummary,
) -> Option<String> {
    if !summary.should_surface() {
        return None;
    }

    let has_cause = summary
        .primary_cause
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    Some(
        if has_cause {
            agendao_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: true,
                source: agendao_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        } else {
            agendao_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: false,
                source: agendao_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        }
        .to_string(),
    )
}

fn provider_diagnostic_label(
    metadata: &Option<HashMap<String, serde_json::Value>>,
) -> Option<String> {
    metadata
        .as_ref()
        .and_then(agendao_provider::provider_diagnostic_from_metadata)
        .map(|summary| agendao_provider::provider_diagnostic_label(&summary).to_string())
}

fn ingress_diagnostic_label(
    metadata: &Option<HashMap<String, serde_json::Value>>,
) -> Option<String> {
    let metadata = metadata.as_ref()?;
    let source = metadata
        .get("ingress_source")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let stabilization = metadata
        .get("ingress_stabilization")
        .and_then(|value| value.as_object())?;
    let policy = stabilization
        .get("policy")
        .and_then(|value| value.as_str())
        .unwrap_or("metadata_only");
    let batch_count = stabilization
        .get("batch_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    if batch_count > 1 {
        Some(format!("{source} · {policy} · batch {batch_count}"))
    } else {
        Some(format!("{source} · {policy}"))
    }
}

pub(crate) fn sidebar_model_summary(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<String> {
    let provider = sidebar_metadata_text(metadata, "model_provider");
    let model_id = sidebar_metadata_text(metadata, "model_id");
    match (provider, model_id) {
        (Some(provider), Some(model_id)) => Some(format!("{}/{}", provider, model_id)),
        (None, Some(model_id)) => Some(model_id),
        _ => None,
    }
}

pub(crate) fn sidebar_scheduler_summary(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<String> {
    if !sidebar_metadata_bool(metadata, "scheduler_applied") {
        return None;
    }

    let profile = sidebar_metadata_text(metadata, "scheduler_profile");
    let root_agent = sidebar_metadata_text(metadata, "scheduler_root_agent");
    let skill_tree_applied = sidebar_metadata_bool(metadata, "scheduler_skill_tree_applied");

    let mut parts = Vec::new();
    if let Some(profile) = profile {
        parts.push(profile);
    } else {
        parts.push("active".to_string());
    }
    if let Some(root_agent) = root_agent {
        parts.push(format!("root={}", root_agent));
    }
    if skill_tree_applied {
        parts.push("skill-tree".to_string());
    }
    Some(parts.join(" · "))
}

fn sidebar_meta_line(theme: &Theme, label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{} ", label.to_uppercase()),
            Style::default().fg(theme.text_muted),
        ),
        Span::styled(value, Style::default().fg(theme.text)),
    ])
}

fn build_session_graph_lines(
    theme: &Theme,
    width: u16,
    current_title: &str,
    current_session_id: &str,
    active_session_id: &str,
    child_list: &[crate::context::AttachedSessionInfo],
    sessions: &std::collections::HashMap<String, crate::context::Session>,
    session_diff: &std::collections::HashMap<String, Vec<crate::context::DiffEntry>>,
    lifecycle: &SidebarLifecycleState,
    selected_child: Option<&crate::context::AttachedSessionInfo>,
) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    let mut lines = Vec::new();
    let mut hit_rows = Vec::new();

    let spine_style = Style::default().fg(theme.border_subtle);
    lines.push(Line::from(Span::styled("│", spine_style)));
    hit_rows.push(None);

    let root_label = format!(
        "• {}  {}",
        truncate_text(current_title, width.saturating_sub(12) as usize),
        short_session_id(current_session_id)
    );
    lines.push(Line::from(vec![
        Span::styled("│ ", spine_style),
        Span::styled(root_label, Style::default().fg(theme.text).bold()),
    ]));
    hit_rows.push(Some(usize::MAX));

    lines.push(Line::from(Span::styled("│", spine_style)));
    hit_rows.push(None);

    for (idx, child) in child_list.iter().enumerate() {
        let selected = (lifecycle.attached_session_focus
            && idx == lifecycle.attached_session_selected)
            || child.session_id == active_session_id;
        let branch = if idx + 1 == child_list.len() {
            "╰"
        } else {
            "├"
        };
        let branch_continues = idx + 1 != child_list.len();
        let (status_symbol, status_color) = session_status_badge(theme, &child.status);
        let label = child_graph_label(child, sessions.get(&child.session_id), width);
        let row_bg = selected.then_some(theme.background_element);
        let mk_style = |style: Style| {
            if let Some(bg) = row_bg {
                style.bg(bg)
            } else {
                style
            }
        };
        lines.push(Line::from(vec![
            Span::styled("│ ", mk_style(Style::default().fg(theme.border_subtle))),
            Span::styled(
                format!("{branch}─"),
                mk_style(Style::default().fg(if selected {
                    theme.primary
                } else {
                    theme.border_subtle
                })),
            ),
            Span::styled(
                format!("{} ", status_symbol),
                mk_style(Style::default().fg(status_color)),
            ),
            Span::styled(
                label,
                mk_style(Style::default().fg(if selected {
                    theme.text
                } else {
                    theme.text_muted
                })),
            ),
        ]));
        hit_rows.push(Some(idx));

        if branch_continues {
            lines.push(Line::from(vec![Span::styled(
                "│",
                mk_style(Style::default().fg(theme.border_subtle)),
            )]));
            hit_rows.push(None);
        }
    }

    if let Some(child) = selected_child {
        lines.push(Line::from(""));
        hit_rows.push(None);

        let detail_title = sessions
            .get(&child.session_id)
            .map(|session| session.title.as_str())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(child.stage_title.as_str());
        lines.push(Line::from(vec![
            Span::styled("Selected ", Style::default().fg(theme.text_muted)),
            Span::styled(
                truncate_text(detail_title, width.saturating_sub(10) as usize),
                Style::default().fg(theme.text).bold(),
            ),
        ]));
        hit_rows.push(None);

        lines.push(Line::from(vec![
            Span::styled("Session ", Style::default().fg(theme.text_muted)),
            Span::styled(
                short_session_id(&child.session_id),
                Style::default().fg(theme.text),
            ),
            Span::styled("  Stage ", Style::default().fg(theme.text_muted)),
            Span::styled(format_stage_badge(child), Style::default().fg(theme.text)),
        ]));
        hit_rows.push(None);

        let (_, status_color) = session_status_badge(theme, &child.status);
        lines.push(Line::from(vec![
            Span::styled("Status  ", Style::default().fg(theme.text_muted)),
            Span::styled(child.status.clone(), Style::default().fg(status_color)),
        ]));
        hit_rows.push(None);

        if let Some(entries) = session_diff.get(&child.session_id) {
            if entries.is_empty() {
                lines.push(Line::from(Span::styled(
                    "Modified files unavailable",
                    Style::default().fg(theme.text_muted),
                )));
                hit_rows.push(None);
            } else {
                lines.push(Line::from(vec![
                    Span::styled("Files ", Style::default().fg(theme.text_muted)),
                    Span::styled(
                        format!("{} changed", entries.len()),
                        Style::default().fg(theme.text),
                    ),
                ]));
                hit_rows.push(None);
                for entry in entries.iter().take(4) {
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default().fg(theme.text_muted)),
                        Span::styled(
                            truncate_text(&entry.file, width.saturating_sub(16) as usize),
                            Style::default().fg(theme.text_muted),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!("+{}", entry.additions),
                            Style::default().fg(theme.success),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!("-{}", entry.deletions),
                            Style::default().fg(theme.error),
                        ),
                    ]));
                    hit_rows.push(None);
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "Open node to sync modified files",
                Style::default().fg(theme.text_muted),
            )));
            hit_rows.push(None);
        }
    }

    (lines, hit_rows)
}

fn child_graph_label(
    child: &crate::context::AttachedSessionInfo,
    session: Option<&crate::context::Session>,
    width: u16,
) -> String {
    let title = session
        .map(|session| session.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(child.stage_title.as_str());
    let label = match (child.stage_index, child.stage_total) {
        (Some(index), Some(total)) => format!("{title} [{index}/{total}]"),
        (Some(index), None) => format!("{title} [{index}]"),
        _ => title.to_string(),
    };
    let suffix = format!(" {}", short_session_id(&child.session_id));
    let max_label_chars = width.saturating_sub(16) as usize;
    format!("{}{}", truncate_text(&label, max_label_chars), suffix)
}

fn format_stage_badge(child: &crate::context::AttachedSessionInfo) -> String {
    match (child.stage_index, child.stage_total) {
        (Some(index), Some(total)) => format!("{index}/{total}"),
        (Some(index), None) => index.to_string(),
        _ => child.stage_name.clone(),
    }
}

fn short_session_id(session_id: &str) -> String {
    if session_id.len() > 7 {
        session_id[..7].to_string()
    } else {
        session_id.to_string()
    }
}

fn session_status_badge(theme: &Theme, status: &str) -> (&'static str, ratatui::style::Color) {
    match status {
        "running" => ("◐", theme.info),
        "done" => ("•", theme.success),
        "cancelled" => ("•", theme.error),
        "waiting" => ("◯", theme.warning),
        _ => ("•", theme.text_muted),
    }
}

fn split_path_segments(path: &str) -> (String, String) {
    if path.is_empty() {
        return (String::new(), String::new());
    }

    if let Some((prefix, leaf)) = path.rsplit_once('/') {
        if prefix.is_empty() {
            return ("/".to_string(), leaf.to_string());
        }
        return (format!("{}/", prefix), leaf.to_string());
    }

    if let Some((prefix, leaf)) = path.rsplit_once('\\') {
        if prefix.is_empty() {
            return ("\\".to_string(), leaf.to_string());
        }
        return (format!("{}\\", prefix), leaf.to_string());
    }

    (String::new(), path.to_string())
}

fn format_compact_number(value: u64) -> String {
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

fn context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    agendao_types::context_usage_percent(used, limit)
}

fn context_usage_bar(percent: Option<u64>, width: usize) -> String {
    agendao_types::context_usage_bar(percent, width)
}

fn context_usage_style(theme: &Theme, percent: Option<u64>) -> Style {
    let color = match agendao_types::context_pressure_for_percent(percent) {
        agendao_types::ContextPressure::Critical => theme.error,
        agendao_types::ContextPressure::AutoCompactSoon
        | agendao_types::ContextPressure::Warning => theme.warning,
        agendao_types::ContextPressure::Normal if percent.is_some() => theme.success,
        agendao_types::ContextPressure::Normal => theme.text_muted,
    };
    Style::default().fg(color)
}

fn sidebar_usage_line(theme: &Theme, label: &str, used: u64, limit: Option<u64>) -> Line<'static> {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return Line::from(vec![
            Span::styled(format!("{label:<7}"), Style::default().fg(theme.text_muted)),
            Span::styled(format_compact_number(used), Style::default().fg(theme.text)),
        ]);
    };

    let percent = context_usage_percent(used, limit);
    let accent = context_usage_style(theme, percent);
    let percent_label = percent.map_or_else(|| "--".to_string(), |pct| format!("{pct}%"));

    Line::from(vec![
        Span::styled(format!("{label:<7}"), Style::default().fg(theme.text_muted)),
        Span::styled(
            format!(
                "{}/{}",
                format_compact_number(used),
                format_compact_number(limit)
            ),
            Style::default().fg(theme.text),
        ),
        Span::styled(" ", Style::default().fg(theme.text_muted)),
        Span::styled(context_usage_bar(percent, 8), accent),
        Span::styled(" ", Style::default().fg(theme.text_muted)),
        Span::styled(percent_label, accent),
    ])
}

fn format_price_pair(input: f64, output: f64) -> String {
    format!("${}/{} /1M", format_price(input), format_price(output))
}

fn format_price(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else if value >= 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

/// Walk the execution topology tree and collect all AgentTask nodes as (label, status) pairs.
fn collect_agent_nodes_from_topology(
    topo: &Option<crate::api::SessionExecutionTopology>,
) -> Vec<(String, crate::api::ExecutionStatus)> {
    let Some(topo) = topo.as_ref() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    fn walk(
        node: &crate::api::SessionExecutionNode,
        out: &mut Vec<(String, crate::api::ExecutionStatus)>,
    ) {
        if node.kind == crate::api::ExecutionKind::AgentTask {
            let label = node.label.clone().unwrap_or_else(|| node.id.clone());
            out.push((label, node.status.clone()));
        }
        for child in &node.children {
            walk(child, out);
        }
    }
    for root in &topo.roots {
        walk(root, &mut result);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AttachedSessionInfo, Session};
    use agendao_stage_protocol::StageSummary;
    use chrono::Utc;

    #[test]
    fn session_graph_hit_rows_only_map_to_child_nodes() {
        let child = AttachedSessionInfo {
            session_id: "attached-session-1".to_string(),
            stage_name: "review".to_string(),
            stage_title: "Review".to_string(),
            stage_id: Some("stg_1".to_string()),
            stage_index: Some(1),
            stage_total: Some(2),
            status: "running".to_string(),
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            child.session_id.clone(),
            Session {
                id: child.session_id.clone(),
                title: "Review follow-up".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_id: Some("root-session".to_string()),
                share: None,
                metadata: None,
            },
        );
        let mut diffs = HashMap::new();
        diffs.insert(
            child.session_id.clone(),
            vec![crate::context::DiffEntry {
                file: "src/lib.rs".to_string(),
                additions: 12,
                deletions: 3,
            }],
        );
        let lifecycle = SidebarLifecycleState {
            attached_session_selected: 0,
            attached_session_focus: true,
            ..Default::default()
        };

        let (_lines, hit_rows) = build_session_graph_lines(
            &Theme::dark(),
            42,
            "Root Session",
            "root-session",
            "attached-session-1",
            &[child.clone()],
            &sessions,
            &diffs,
            &lifecycle,
            Some(&child),
        );

        assert_eq!(hit_rows.first(), Some(&None));
        let first_child_row = hit_rows.iter().position(|row| row == &Some(0));
        assert_eq!(first_child_row, Some(3));
        assert!(hit_rows.iter().skip(4).all(|row| row.is_none()));
    }

    #[test]
    fn workspace_tree_reveals_current_file_and_maps_hit_rows() {
        let entries = vec![
            "src/main.rs".to_string(),
            "src/ui/app.rs".to_string(),
            "README.md".to_string(),
        ];
        let modified = HashSet::from(["src/ui/app.rs".to_string()]);
        let nodes = build_workspace_visible_nodes(
            &entries,
            &HashSet::new(),
            &modified,
            Some("src/ui/app.rs"),
            Some("src/ui/app.rs"),
        );

        assert!(nodes.iter().any(|node| node.path == "src"));
        assert!(nodes.iter().any(|node| node.path == "src/ui"));
        assert!(nodes
            .iter()
            .any(|node| node.path == "src/ui/app.rs" && node.is_current));

        let lifecycle = SidebarLifecycleState {
            active_tab: SidebarTab::Workspace,
            workspace_selected: nodes
                .iter()
                .position(|node| node.path == "src/ui/app.rs")
                .expect("workspace node"),
            workspace_focus: true,
            ..Default::default()
        };
        let (_lines, hit_rows) = build_workspace_tree_lines(&Theme::dark(), 42, &nodes, &lifecycle);
        let selected_row = hit_rows
            .iter()
            .position(|row| row.is_some_and(|idx| nodes[idx].path == "src/ui/app.rs"));
        assert!(selected_row.is_some());
    }

    #[test]
    fn workspace_tree_root_nodes_respect_expanded_state() {
        let entries = vec![
            "src/main.rs".to_string(),
            "src/ui/app.rs".to_string(),
            "docs/guide.md".to_string(),
        ];
        let nodes =
            build_workspace_visible_nodes(&entries, &HashSet::new(), &HashSet::new(), None, None);

        assert!(nodes
            .iter()
            .any(|node| node.path == "src" && node.is_dir && !node.expanded));
        assert!(nodes
            .iter()
            .any(|node| node.path == "docs" && node.is_dir && !node.expanded));
        assert!(!nodes.iter().any(|node| node.path == "src/ui"));
        assert!(!nodes.iter().any(|node| node.path == "src/main.rs"));
    }

    #[test]
    fn workspace_popup_only_appears_for_long_paths() {
        let area = Some(Rect::new(0, 0, 20, 10));
        let short = WorkspaceVisibleNode {
            path: "src/app.rs".to_string(),
            label: "app.rs".to_string(),
            depth: 1,
            is_dir: false,
            expanded: false,
            is_modified: false,
            is_current: false,
        };
        let long = WorkspaceVisibleNode {
            path: "very-long-directory-name/another-long-segment/file.rs".to_string(),
            label: "file.rs".to_string(),
            depth: 2,
            is_dir: false,
            expanded: false,
            is_modified: false,
            is_current: false,
        };

        assert_eq!(workspace_popup_text(area, &short), None);
        assert_eq!(
            workspace_popup_text(area, &long),
            Some("very-long-directory-name/another-long-segment/file.rs".to_string())
        );
    }

    #[test]
    fn usage_section_does_not_treat_cumulative_tokens_as_current_context() {
        let context = Arc::new(AppContext::new());
        let session_id = {
            let mut session = context.session.write();
            let session_id = session.create_session(Some("Token Session".to_string()));
            session_id
        };
        context.navigate_session(session_id.clone());
        context.apply_session_projection_snapshot(
            &session_id,
            None,
            Vec::new(),
            Some(agendao_types::SessionUsage {
                input_tokens: 150_000,
                output_tokens: 150_000,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 100_000,
                cache_miss_tokens: 50_000,
                context_tokens: 0,
                total_cost: 0.0,
            }),
            None,
            None,
            None,
            None,
            None,
        );
        context
            .providers
            .write()
            .push(crate::context::ProviderInfo {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                models: vec![crate::context::ModelInfo {
                    id: "openai/gpt-5".to_string(),
                    name: "GPT-5".to_string(),
                    context_window: 200_000,
                    max_output_tokens: 16_000,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: None,
                    cost_per_million_output: None,
                }],
            });
        let sidebar_seed = Sidebar::capture_render_seed(&context, &session_id);
        let sidebar_inputs = Sidebar::render_inputs_from_seed(&sidebar_seed);
        let sidebar = Sidebar::new(session_id);
        let mut state = SidebarRenderState::default();
        let mut lifecycle = SidebarLifecycleState::default();
        let area = Rect::new(0, 0, 64, 24);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        let mut surface = crate::ui::BufferSurface::new(&mut buffer);

        sidebar.render_surface(
            &sidebar_inputs,
            &mut surface,
            area,
            &mut state,
            &mut lifecycle,
            false,
        );

        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");

        assert!(!rendered.contains("Current"));
        assert!(!rendered.contains("compact now"));
        assert!(rendered.contains("300K cumulative"));
        assert!(!rendered.contains("450K cumulative"));
        assert!(rendered.contains("H/M/W 100K / 50K / 0"));
    }

    #[test]
    fn sidebar_uses_explicit_session_authority_not_current_route() {
        let context = Arc::new(AppContext::new());
        let (root_session_id, child_session_id) = {
            let mut session = context.session.write();
            let root = session.create_session(Some("Root Session".to_string()));
            let child = session.create_session(Some("Child Session".to_string()));
            session
                .sessions
                .get_mut(&child)
                .expect("child session")
                .parent_id = Some(root.clone());
            (root, child)
        };
        context.navigate_session(root_session_id.clone());
        context.set_attached_sessions(
            &root_session_id,
            vec![AttachedSessionInfo {
                session_id: child_session_id.clone(),
                stage_name: "review".to_string(),
                stage_title: "Review".to_string(),
                stage_id: Some("stg_1".to_string()),
                stage_index: Some(1),
                stage_total: Some(1),
                status: "running".to_string(),
            }],
        );
        context.apply_session_projection_snapshot(
            &child_session_id,
            None,
            Vec::<StageSummary>::new(),
            Some(agendao_types::SessionUsage {
                input_tokens: 1_000,
                output_tokens: 2_000,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                cache_miss_tokens: 0,
                context_tokens: 0,
                total_cost: 0.25,
            }),
            None,
            None,
            None,
            None,
            None,
        );
        context
            .providers
            .write()
            .push(crate::context::ProviderInfo {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                models: vec![crate::context::ModelInfo {
                    id: "openai/gpt-5".to_string(),
                    name: "GPT-5".to_string(),
                    context_window: 200_000,
                    max_output_tokens: 16_000,
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: None,
                    cost_per_million_output: None,
                }],
            });

        let sidebar_seed = Sidebar::capture_render_seed(&context, &child_session_id);
        let sidebar_inputs = Sidebar::render_inputs_from_seed(&sidebar_seed);
        let sidebar = Sidebar::new(child_session_id);
        let mut state = SidebarRenderState::default();
        let mut lifecycle = SidebarLifecycleState::default();
        let area = Rect::new(0, 0, 64, 24);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        let mut surface = crate::ui::BufferSurface::new(&mut buffer);

        sidebar.render_surface(
            &sidebar_inputs,
            &mut surface,
            area,
            &mut state,
            &mut lifecycle,
            false,
        );

        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");

        assert!(rendered.contains("3K cumulative"));
        assert!(rendered.contains("1 sessions"));
        assert!(rendered.contains("Child Session"));
    }

    #[test]
    fn cache_diagnostic_label_reads_message_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert(
            agendao_provider::cache::CACHE_EVIDENCE_METADATA_KEY.to_string(),
            serde_json::json!({
                "status": "degraded",
                "severity": "MediumChange",
                "primary_cause": "prefix changed before the stable boundary",
                "change_count": 1,
            }),
        );

        let label = cache_diagnostic_label(&Some(metadata)).expect("label");

        assert_eq!(label, "cache explained");
    }

    #[test]
    fn context_closure_cache_diagnostic_uses_narrow_status_words() {
        let contract = agendao_types::SessionContextClosureContract {
            prefix_stability: agendao_types::SessionPrefixStabilityContract {
                basis: agendao_types::SessionCacheSemanticsBasis::ApiView,
                tracked_on_api_view: true,
                api_view_messages: 9,
                trimmed_model_visible_messages: 2,
                prefix_change_detected: true,
                explanation: None,
            },
            compaction_boundary: agendao_types::SessionCompactionBoundaryContract {
                boundary_recorded: true,
                phase: None,
                trigger: None,
                reason: None,
                lifecycle_status: None,
                governance_status: None,
                request_pressure_percent: None,
                live_pressure_percent: None,
                compaction_attempted: true,
                compaction_succeeded: true,
                blocking: false,
                installed: None,
            },
            cache_explainability: agendao_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: true,
                source: agendao_types::SessionCacheExplainabilitySource::BoundaryEvidence,
                severity: Some(agendao_types::SessionCacheSeverity::MediumChange),
                explanation: None,
            },
            child_history_isolation: agendao_types::SessionChildHistoryIsolationContract {
                attached_subtree_session_count: 0,
                owner_session_cumulative_tokens: 0,
                workflow_cumulative_tokens: 0,
                attached_subtree_cumulative_tokens: 0,
                owner_live_context_tokens: Some(0),
                owner_local_live_prefix: true,
                child_history_in_live_prefix_detected: false,
                explanation: "isolated".to_string(),
            },
        };

        assert_eq!(
            context_closure_cache_diagnostic_label(&contract).as_deref(),
            Some("cache explained · prefix changed")
        );
    }

    #[test]
    fn provider_diagnostic_label_reads_message_metadata() {
        let mut metadata = HashMap::new();
        agendao_provider::ProviderDiagnosticSummary {
            severity: agendao_provider::ProviderDiagnosticSeverity::HardFail,
            source: agendao_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "rejected replay".to_string(),
        }
        .attach_to_metadata(&mut metadata);

        let label = provider_diagnostic_label(&Some(metadata)).expect("label");

        assert_eq!(label, "thinking replay rejected");
    }

    #[test]
    fn sidebar_usage_line_includes_context_meter_bar() {
        let theme = Theme::default();
        let line = sidebar_usage_line(&theme, "Current", 12_450, Some(200_000));
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");

        assert!(rendered.contains("12.4K/200K"));
        assert!(rendered.contains("[█░░░░░░░]"));
        assert!(rendered.contains("6%"));
    }

    #[test]
    fn root_session_hit_navigates_to_graph_root_when_viewing_child_session() {
        let mut state = SidebarRenderState {
            sections_area: Some(Rect::new(0, 0, 20, 8)),
            attached_session_line_hits: vec![(0, usize::MAX)],
            session_graph_root_id: Some("root-session".to_string()),
            session_graph_active_id: Some("child-session".to_string()),
            ..Default::default()
        };
        let mut lifecycle = SidebarLifecycleState::default();

        assert!(state.handle_click(&mut lifecycle, 0, 0));
        assert_eq!(
            state.take_pending_navigate_session().as_deref(),
            Some("root-session")
        );
        assert!(!state.take_pending_navigate_parent());
    }

    #[test]
    fn root_session_hit_stays_on_graph_root_when_already_viewing_root() {
        let mut state = SidebarRenderState {
            sections_area: Some(Rect::new(0, 0, 20, 8)),
            attached_session_line_hits: vec![(0, usize::MAX)],
            session_graph_root_id: Some("root-session".to_string()),
            session_graph_active_id: Some("root-session".to_string()),
            ..Default::default()
        };
        let mut lifecycle = SidebarLifecycleState::default();

        assert!(state.handle_click(&mut lifecycle, 0, 0));
        assert_eq!(
            state.take_pending_navigate_session().as_deref(),
            Some("root-session")
        );
        assert!(!state.take_pending_navigate_parent());
    }
}
