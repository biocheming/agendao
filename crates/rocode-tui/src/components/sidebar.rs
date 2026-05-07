use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::branding::{APP_NAME, APP_SHORT_NAME, APP_VERSION_DATE};
use crate::context::{
    AppContext, LspConnectionStatus, McpConnectionStatus, MessageRole, SidebarLifecycleState,
    SidebarTab, TodoStatus,
};
use crate::file_index::FileIndex;
use crate::theme::Theme;
use crate::ui::RenderSurface;
use rocode_core::process_registry::ProcessKind;

const SIDEBAR_WORKSPACE_INDEX_MAX_DEPTH: usize = 8;

pub struct Sidebar {
    context: Arc<AppContext>,
    session_id: String,
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
    /// Maps rendered line index → child session list index (for click selection).
    child_session_line_hits: Vec<(usize, usize)>,
    /// Maps rendered line index → workspace visible node index.
    workspace_line_hits: Vec<(usize, usize)>,
    workspace_index: FileIndex,
    workspace_expanded_dirs: HashSet<String>,
    workspace_visible_nodes: Vec<WorkspaceVisibleNode>,
    workspace_selected_path: Option<String>,
    workspace_tooltip: Option<String>,
    workspace_seeded_root: Option<String>,
    /// Pending navigation target set by click-to-activate on an already-selected child session.
    /// Consumed (taken) by the app after `handle_click` returns.
    pending_navigate_child: Option<usize>,
    /// Set when the root session node is double-clicked (navigate back to parent).
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
            lifecycle.child_session_focus = false;
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
            lifecycle.child_session_focus = false;
            lifecycle.workspace_focus = false;
            self.workspace_tooltip = None;
            return true;
        }

        // Check if the click is on a child session item
        if let Some((_line_idx, cs_idx)) = self
            .child_session_line_hits
            .iter()
            .find(|(li, _)| *li == line_index)
        {
            if *cs_idx == usize::MAX {
                // Root session node — navigate to parent
                self.pending_navigate_parent = true;
                self.workspace_tooltip = None;
                return true;
            }
            if lifecycle.child_session_focus && lifecycle.child_session_selected == *cs_idx {
                // Already selected and focused — treat as activation (navigate)
                self.pending_navigate_child = Some(*cs_idx);
            } else {
                // First click — select and focus
                lifecycle.active_tab = SidebarTab::Session;
                lifecycle.child_session_selected = *cs_idx;
                lifecycle.child_session_focus = true;
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
            lifecycle.child_session_focus = false;
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

    /// Take the pending child session navigation index, if any.
    /// Returns the index into the child_sessions list that was clicked for activation.
    pub fn take_pending_navigate_child(&mut self) -> Option<usize> {
        self.pending_navigate_child.take()
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

fn clamp_sidebar_child_session_selection(lifecycle: &mut SidebarLifecycleState, count: usize) {
    if count == 0 {
        lifecycle.child_session_selected = 0;
    } else if lifecycle.child_session_selected >= count {
        lifecycle.child_session_selected = count - 1;
    }
}

fn clamp_sidebar_workspace_selection(lifecycle: &mut SidebarLifecycleState, count: usize) {
    if count == 0 {
        lifecycle.workspace_selected = 0;
    } else if lifecycle.workspace_selected >= count {
        lifecycle.workspace_selected = count - 1;
    }
}

struct SidebarSection {
    key: &'static str,
    title: &'static str,
    lines: Vec<Line<'static>>,
    child_session_hit_rows: Option<Vec<Option<usize>>>,
    workspace_hit_rows: Option<Vec<Option<usize>>>,
    summary: Option<String>,
    collapsible: bool,
}

impl Sidebar {
    pub fn new(context: Arc<AppContext>, session_id: String) -> Self {
        Self {
            context,
            session_id,
        }
    }

    pub fn render<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        floating: bool,
    ) {
        self.render_with_bg(surface, area, state, lifecycle, floating, None);
    }

    pub fn render_with_bg<S: RenderSurface>(
        &self,
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
        let theme = self.context.theme.read().clone();
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
            surface, layout[1], &theme, state, lifecycle, floating, panel_bg,
        );
        self.render_footer_with_bg(surface, layout[2], &theme, floating, panel_bg);
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

        render_sidebar_tab(
            surface,
            session_area,
            theme,
            lifecycle.active_tab == SidebarTab::Session,
            "Session",
        );
        render_sidebar_tab(
            surface,
            workspace_area,
            theme,
            lifecycle.active_tab == SidebarTab::Workspace,
            "Workspace",
        );
    }

    fn render_sections_with_bg<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        floating: bool,
        panel_bg: ratatui::style::Color,
    ) {
        if area.width == 0 || area.height == 0 {
            state.set_sections_layout(area, 0, Vec::new());
            return;
        }

        let session_ctx = self.context.session.read();
        let mcp_servers = self.context.mcp_servers.read();
        let lsp_status = self.context.lsp_status.read();

        let sections: Vec<SidebarSection> = if lifecycle.active_tab == SidebarTab::Workspace {
            self.build_workspace_sections(area, theme, state, lifecycle, &session_ctx)
        } else {
            let session = session_ctx.sessions.get(&self.session_id);
            let messages = session_ctx
                .messages
                .get(&self.session_id)
                .cloned()
                .unwrap_or_default();

            let title = session
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "New Session".to_string());
            let graph_root_session = session
                .and_then(|session| session.parent_id.as_ref())
                .and_then(|parent_id| session_ctx.sessions.get(parent_id))
                .or(session);
            let graph_root_id = graph_root_session
                .map(|session| session.id.as_str())
                .unwrap_or(self.session_id.as_str());
            let graph_root_title = graph_root_session
                .map(|session| session.title.as_str())
                .unwrap_or(title.as_str());
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
                child_session_hit_rows: None,
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
                    child_session_hit_rows: None,
                    workspace_hit_rows: None,
                    summary: None,
                    collapsible: false,
                });
            }

            let total_cost = self
                .context
                .session_usage_books()
                .as_ref()
                .map(|books| books.workflow_cumulative.total_cost)
                .or_else(|| {
                    self.context
                        .session_usage()
                        .as_ref()
                        .map(|usage| usage.total_cost)
                })
                .unwrap_or_else(|| {
                    messages
                        .iter()
                        .filter(|m| matches!(m.role, MessageRole::Assistant))
                        .map(|m| m.cost)
                        .sum()
                });
            let total_tokens = self
                .context
                .session_usage_books()
                .as_ref()
                .map(|books| books.workflow_cumulative.total_tokens())
                .or_else(|| {
                    self.context
                        .session_usage()
                        .as_ref()
                        .map(total_session_tokens)
                })
                .unwrap_or_else(|| {
                    messages
                        .iter()
                        .filter(|m| matches!(m.role, MessageRole::Assistant))
                        .map(|m| m.tokens.input + m.tokens.output + m.tokens.reasoning)
                        .sum::<u64>()
                });
            let (session_cache_read_tokens, session_cache_miss_tokens, session_cache_write_tokens) =
                self.context
                    .session_usage_books()
                    .as_ref()
                    .map(|books| {
                        (
                            books.workflow_cumulative.cache_read_tokens,
                            books.workflow_cumulative.cache_miss_tokens,
                            books.workflow_cumulative.cache_write_tokens,
                        )
                    })
                    .or_else(|| {
                        self.context.session_usage().as_ref().map(|usage| {
                            (
                                usage.cache_read_tokens,
                                usage.cache_miss_tokens,
                                usage.cache_write_tokens,
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        messages
                            .iter()
                            .filter(|m| matches!(m.role, MessageRole::Assistant))
                            .fold((0u64, 0u64, 0u64), |(read, miss, write), message| {
                                (
                                    read.saturating_add(message.tokens.cache_read),
                                    miss.saturating_add(message.tokens.cache_miss),
                                    write.saturating_add(message.tokens.cache_write),
                                )
                            })
                    });
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
            let current_context_tokens = self.context.current_context_tokens();
            let active_model_info = self.context.resolve_model_info(active_model);
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
                            .and_then(|percent| context_pressure_note(Some(percent)))
                        {
                            lines.push(Line::from(vec![
                                Span::styled("State  ", Style::default().fg(theme.text_muted)),
                                Span::styled(note, Style::default().fg(theme.warning)),
                            ]));
                        }
                    }
                    if total_tokens > 0 {
                        lines.push(Line::from(vec![
                            Span::styled("Workflow ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                format!("{} cumulative", format_compact_number(total_tokens)),
                                Style::default().fg(theme.text),
                            ),
                        ]));
                    }
                    if session_cache_read_tokens > 0
                        || session_cache_miss_tokens > 0
                        || session_cache_write_tokens > 0
                    {
                        lines.push(Line::from(vec![
                            Span::styled("Cache  ", Style::default().fg(theme.text_muted)),
                            Span::styled(
                                if session_cache_miss_tokens > 0 {
                                    format!(
                                        "H/M {} / {}",
                                        format_compact_number(session_cache_read_tokens),
                                        format_compact_number(session_cache_miss_tokens)
                                    )
                                } else {
                                    format!(
                                        "R/W {} / {}",
                                        format_compact_number(session_cache_read_tokens),
                                        format_compact_number(session_cache_write_tokens)
                                    )
                                },
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
                                    if turn.tokens.cache_miss > 0 {
                                        format!(
                                            "H/M {} / {}",
                                            format_compact_number(turn.tokens.cache_read),
                                            format_compact_number(turn.tokens.cache_miss)
                                        )
                                    } else {
                                        format!(
                                            "R/W {} / {}",
                                            format_compact_number(turn.tokens.cache_read),
                                            format_compact_number(turn.tokens.cache_write)
                                        )
                                    },
                                    Style::default().fg(theme.text),
                                ),
                            ]));
                        }
                    }
                    let cache_diagnostic = self
                        .context
                        .session_cache_semantics()
                        .and_then(|summary| summary.label)
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
                            format!("${:.4}", total_cost),
                            Style::default().fg(theme.text),
                        ),
                    ]));
                    lines
                },
                child_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: None,
                collapsible: false,
            });

            let connected_mcp = mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::Connected))
                .count();
            let failed_mcp = mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::Failed))
                .count();
            let registration_needed_mcp = mcp_servers
                .iter()
                .filter(|s| matches!(s.status, McpConnectionStatus::NeedsClientRegistration))
                .count();
            let problematic_mcp = failed_mcp + registration_needed_mcp;
            let mut mcp_lines: Vec<Line<'static>> = Vec::new();
            if mcp_servers.is_empty() {
                mcp_lines.push(Line::from(Span::styled(
                    "No MCP servers",
                    Style::default().fg(theme.text_muted),
                )));
            } else {
                for server in mcp_servers.iter() {
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
                child_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: Some(format!(
                    "{} active, {} errors",
                    connected_mcp, problematic_mcp
                )),
                collapsible: mcp_servers.len() > 2,
            });

            let connected_lsp = lsp_status
                .iter()
                .filter(|s| matches!(s.status, LspConnectionStatus::Connected))
                .count();
            let errored_lsp = lsp_status
                .iter()
                .filter(|s| matches!(s.status, LspConnectionStatus::Error))
                .count();
            let mut lsp_lines: Vec<Line<'static>> = Vec::new();
            if lsp_status.is_empty() {
                lsp_lines.push(Line::from(Span::styled(
                    "No active LSP",
                    Style::default().fg(theme.text_muted),
                )));
            } else {
                for server in lsp_status.iter() {
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
                child_session_hit_rows: None,
                workspace_hit_rows: None,
                summary: Some(format!(
                    "{} connected, {} errors",
                    connected_lsp, errored_lsp
                )),
                collapsible: lsp_status.len() > 2,
            });

            if let Some(todos) = session_ctx.todos.get(&self.session_id) {
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
                        child_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(format!("{} pending", pending.len())),
                        collapsible: pending.len() > 2,
                    });
                }
            }

            if let Some(entries) = session_ctx.session_diff.get(&self.session_id) {
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
                        child_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(format!("{} files changed", entries.len())),
                        collapsible: entries.len() > 2,
                    });
                }
            }

            // Processes section
            let proc_list = self.context.processes.read().clone();
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
                        Span::styled("● ", mk_style(Style::default().fg(kind_color))),
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
                    child_session_hit_rows: None,
                    workspace_hit_rows: None,
                    summary: Some(format!("{} running", proc_list.len())),
                    collapsible: proc_list.len() > 2,
                });
            }

            // Agents section — sourced from execution topology (server-side)
            {
                let topology = self.context.execution_topology();
                let agent_nodes = collect_agent_nodes_from_topology(&topology);
                if !agent_nodes.is_empty() {
                    let mut agent_lines: Vec<Line<'static>> = Vec::new();
                    let mut running = 0usize;
                    let mut done = 0usize;
                    for (label, status) in &agent_nodes {
                        let (symbol, color) = match status {
                            crate::api::ExecutionStatus::Running => {
                                running += 1;
                                ("●", theme.info)
                            }
                            crate::api::ExecutionStatus::Waiting => ("◯", theme.warning),
                            crate::api::ExecutionStatus::Done => {
                                done += 1;
                                ("✓", theme.success)
                            }
                            _ => ("●", theme.text_muted),
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
                        child_session_hit_rows: None,
                        workspace_hit_rows: None,
                        summary: Some(summary),
                        collapsible: agent_nodes.len() > 3,
                    });
                }
            }

            // Session Graph section
            let child_list = self.context.child_sessions();
            clamp_sidebar_child_session_selection(lifecycle, child_list.len());
            if !child_list.is_empty() {
                let current_child_index = child_list
                    .iter()
                    .position(|child| child.session_id == self.session_id);
                let selected_child_index = if lifecycle.child_session_focus {
                    Some(lifecycle.child_session_selected.min(child_list.len() - 1))
                } else {
                    current_child_index
                };
                let selected_child = selected_child_index.and_then(|index| child_list.get(index));
                let (cs_lines, child_session_hit_rows) = build_session_graph_lines(
                    theme,
                    area.width,
                    graph_root_title,
                    graph_root_id,
                    &self.session_id,
                    &child_list,
                    &session_ctx.sessions,
                    &session_ctx.session_diff,
                    lifecycle,
                    selected_child,
                );
                sections.push(SidebarSection {
                    key: "session_graph",
                    title: "Session Graph",
                    lines: cs_lines,
                    child_session_hit_rows: Some(child_session_hit_rows),
                    workspace_hit_rows: None,
                    summary: Some(format!("{} sessions", child_list.len())),
                    collapsible: child_list.len() > 2,
                });
            }

            sections
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut line_index = 0usize;
        let mut toggle_hits: Vec<SidebarToggleHit> = Vec::new();
        let mut process_line_hits: Vec<(usize, usize)> = Vec::new();
        let mut child_session_line_hits: Vec<(usize, usize)> = Vec::new();
        let mut workspace_line_hits: Vec<(usize, usize)> = Vec::new();
        for section in sections {
            if !lines.is_empty() {
                lines.push(Line::from(""));
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
            }
            header.push(Span::styled(
                section.title.to_string(),
                Style::default().fg(theme.text).bold(),
            ));
            if collapsed {
                if let Some(summary) = section.summary {
                    header.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
                    header.push(Span::styled(summary, Style::default().fg(theme.text_muted)));
                }
            }
            lines.push(Line::from(header));
            line_index += 1;

            if !collapsed {
                let is_processes = section.key == "processes";
                let child_hit_rows = section.child_session_hit_rows.as_ref();
                let workspace_hit_rows = section.workspace_hit_rows.as_ref();
                for (row_idx, row) in section.lines.into_iter().enumerate() {
                    if is_processes {
                        process_line_hits.push((line_index, row_idx));
                    }
                    if let Some(hit_rows) = child_hit_rows {
                        if let Some(Some(child_index)) = hit_rows.get(row_idx) {
                            child_session_line_hits.push((line_index, *child_index));
                        }
                    }
                    if let Some(hit_rows) = workspace_hit_rows {
                        if let Some(Some(node_index)) = hit_rows.get(row_idx) {
                            workspace_line_hits.push((line_index, *node_index));
                        }
                    }
                    lines.push(row);
                    line_index += 1;
                }
            }
        }

        let has_overflow = lines.len() > usize::from(area.height);
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

        state.set_sections_layout(sections_text_area, lines.len(), toggle_hits);
        state.process_line_hits = process_line_hits;
        state.child_session_line_hits = child_session_line_hits;
        state.workspace_line_hits = workspace_line_hits;

        let mut paragraph = Paragraph::new(lines)
            .scroll((state.scroll_offset.min(usize::from(u16::MAX)) as u16, 0));
        if !floating {
            paragraph = paragraph
                .block(Block::default().borders(Borders::NONE))
                .style(Style::default().bg(panel_bg));
        }
        surface.render_widget(paragraph, sections_text_area);

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
        state: &mut SidebarRenderState,
        lifecycle: &mut SidebarLifecycleState,
        session_ctx: &crate::context::SessionContext,
    ) -> Vec<SidebarSection> {
        let directory = self.context.directory.read().clone();
        let workspace_root = workspace_root_path(&directory);
        state.refresh_workspace_index(&workspace_root);
        if state.workspace_seeded_root.as_deref() != Some(directory.as_str()) {
            state.workspace_expanded_dirs =
                top_level_workspace_dirs(state.workspace_index.entries());
            state.workspace_seeded_root = Some(directory.clone());
            state.workspace_selected_path = None;
            state.workspace_tooltip = None;
        }

        let modified_paths = session_ctx
            .session_diff
            .get(&self.session_id)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| normalize_workspace_path(&workspace_root, &entry.file))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let current_path = session_ctx
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

        let (root_prefix, root_leaf) = split_path_segments(directory.as_str());
        let workspace_label = if root_leaf.is_empty() {
            directory.clone()
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
            child_session_hit_rows: None,
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
            child_session_hit_rows: None,
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
        floating: bool,
        panel_bg: ratatui::style::Color,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let directory = self.context.directory.read().clone();
        let (prefix, leaf) = split_path_segments(&directory);
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

#[derive(Clone, Default, PartialEq, Eq)]
struct WorkspaceTreeDir {
    dirs: BTreeMap<String, WorkspaceTreeDir>,
    files: Vec<String>,
}

fn render_sidebar_tab<S: RenderSurface>(
    surface: &mut S,
    area: Rect,
    theme: &Theme,
    active: bool,
    label: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let style = if active {
        Style::default()
            .bg(theme.background_element)
            .fg(theme.text)
            .bold()
    } else {
        Style::default().fg(theme.text_muted)
    };
    let text = format!(" {label} ");
    surface.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
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
        .and_then(rocode_provider::cache::cache_bust_summary_from_metadata)
        .and_then(|summary| rocode_provider::cache::cache_bust_summary_label(&summary))
}

fn provider_diagnostic_label(
    metadata: &Option<HashMap<String, serde_json::Value>>,
) -> Option<String> {
    metadata
        .as_ref()
        .and_then(rocode_provider::provider_diagnostic_from_metadata)
        .map(|summary| rocode_provider::provider_diagnostic_label(&summary).to_string())
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
    child_list: &[crate::context::ChildSessionInfo],
    sessions: &std::collections::HashMap<String, crate::context::Session>,
    session_diff: &std::collections::HashMap<String, Vec<crate::context::DiffEntry>>,
    lifecycle: &SidebarLifecycleState,
    selected_child: Option<&crate::context::ChildSessionInfo>,
) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    let mut lines = Vec::new();
    let mut hit_rows = Vec::new();

    let spine_style = Style::default().fg(theme.border_subtle);
    lines.push(Line::from(Span::styled("│", spine_style)));
    hit_rows.push(None);

    let root_label = format!(
        "● {}  {}",
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
        let selected = (lifecycle.child_session_focus && idx == lifecycle.child_session_selected)
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
    child: &crate::context::ChildSessionInfo,
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

fn format_stage_badge(child: &crate::context::ChildSessionInfo) -> String {
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
        "running" => ("●", theme.info),
        "done" => ("●", theme.success),
        "cancelled" => ("●", theme.error),
        "waiting" => ("◯", theme.warning),
        _ => ("●", theme.text_muted),
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
    rocode_types::context_usage_percent(used, limit)
}

fn context_usage_style(theme: &Theme, percent: Option<u64>) -> Style {
    let color = match rocode_types::context_pressure_for_percent(percent) {
        rocode_types::ContextPressure::Critical => theme.error,
        rocode_types::ContextPressure::AutoCompactSoon | rocode_types::ContextPressure::Warning => {
            theme.warning
        }
        rocode_types::ContextPressure::Normal if percent.is_some() => theme.success,
        rocode_types::ContextPressure::Normal => theme.text_muted,
    };
    Style::default().fg(color)
}

fn context_pressure_note(percent: Option<u64>) -> Option<&'static str> {
    rocode_types::context_pressure_label(percent)
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
        Span::styled(percent_label, accent),
    ])
}

fn total_session_tokens(usage: &rocode_session::SessionUsage) -> u64 {
    usage.input_tokens + usage.output_tokens + usage.reasoning_tokens
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
    use crate::context::{ChildSessionInfo, Session};
    use chrono::Utc;

    #[test]
    fn session_graph_hit_rows_only_map_to_child_nodes() {
        let child = ChildSessionInfo {
            session_id: "child-session-1".to_string(),
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
            child_session_selected: 0,
            child_session_focus: true,
            ..Default::default()
        };

        let (_lines, hit_rows) = build_session_graph_lines(
            &Theme::dark(),
            42,
            "Root Session",
            "root-session",
            "child-session-1",
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
            session.session_usage = Some(rocode_session::SessionUsage {
                input_tokens: 150_000,
                output_tokens: 150_000,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 100_000,
                cache_miss_tokens: 50_000,
                context_tokens: 0,
                total_cost: 0.0,
            });
            session_id
        };
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
        let sidebar = Sidebar::new(context, session_id);
        let mut state = SidebarRenderState::default();
        let mut lifecycle = SidebarLifecycleState::default();
        let area = Rect::new(0, 0, 64, 24);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        let mut surface = crate::ui::BufferSurface::new(&mut buffer);

        sidebar.render(&mut surface, area, &mut state, &mut lifecycle, false);

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
        assert!(rendered.contains("H/M 100K / 50K"));
    }

    #[test]
    fn cache_diagnostic_label_reads_message_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert(
            rocode_provider::cache::CACHE_BUST_SUMMARY_METADATA_KEY.to_string(),
            serde_json::json!({
                "status": "degraded",
                "severity": "LikelyBust",
                "primary_cause": "messagePrefixHash changed: message prefix changed before the stable boundary",
                "change_count": 1,
            }),
        );

        let label = cache_diagnostic_label(&Some(metadata)).expect("label");

        assert!(label.contains("likely bust"));
        assert!(label.contains("messagePrefixHash"));
    }

    #[test]
    fn provider_diagnostic_label_reads_message_metadata() {
        let mut metadata = HashMap::new();
        rocode_provider::ProviderDiagnosticSummary {
            severity: rocode_provider::ProviderDiagnosticSeverity::HardFail,
            source: rocode_provider::ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-reasoner".to_string()),
            message: "rejected replay".to_string(),
        }
        .attach_to_metadata(&mut metadata);

        let label = provider_diagnostic_label(&Some(metadata)).expect("label");

        assert_eq!(label, "thinking replay rejected");
    }
}
