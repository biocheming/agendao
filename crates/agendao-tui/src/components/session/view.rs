#[derive(Default)]
struct SessionViewState {
    viewport: SessionMessageViewportState,
    reasoning: SessionReasoningState,
    sidebar: SessionSidebarChromeState,
    reactive_bindings: Arc<Mutex<SessionReactiveBindings>>,
    pending_actions: Arc<Mutex<Vec<SessionInteractionAction>>>,
    viewport_dirty: bool,
    reasoning_dirty: bool,
    sidebar_dirty: bool,
    pending_navigate_attached: Option<usize>,
    pending_navigate_session: Option<String>,
    pending_navigate_parent: bool,
}

impl SessionViewState {
    fn update_viewport_state(&mut self, next: SessionMessageViewportState) {
        if self.viewport == next {
            return;
        }
        if let Some(setter) = self.reactive_bindings.lock().viewport_setter {
            setter.set_if_changed(next.clone());
            self.viewport_dirty = true;
        }
        self.viewport = next;
    }

    fn update_reasoning_state(&mut self, next: SessionReasoningState) {
        if self.reasoning == next {
            return;
        }
        if let Some(setter) = self.reactive_bindings.lock().reasoning_setter {
            setter.set_if_changed(next.clone());
            self.reasoning_dirty = true;
        }
        self.reasoning = next;
    }

    fn update_sidebar_state(&mut self, next: SessionSidebarChromeState) {
        if self.sidebar == next {
            return;
        }
        if let Some(setter) = self.reactive_bindings.lock().sidebar_setter {
            setter.set_if_changed(next.clone());
            self.sidebar_dirty = true;
        }
        self.sidebar = next;
    }

    fn max_scroll_offset(&self) -> usize {
        max_scroll_offset_for_viewport(&self.viewport)
    }

    fn queue_interaction_action(&mut self, action: SessionInteractionAction) {
        apply_session_interaction_action(&mut self.viewport, &mut self.reasoning, &action);
        self.pending_actions.lock().push(action);
    }
}

#[derive(Clone)]
pub struct SessionView {
    session_id: String,
    state: Arc<Mutex<SessionViewState>>,
}

impl SessionView {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            state: Arc::new(Mutex::new(SessionViewState {
                reactive_bindings: Arc::new(Mutex::new(SessionReactiveBindings::default())),
                pending_actions: Arc::new(Mutex::new(Vec::new())),
                ..SessionViewState::default()
            })),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn render(
        &self,
        context: &Arc<AppContext>,
        surface: &mut BufferSurface<'_>,
        area: Rect,
        prompt: &Prompt,
    ) {
        let snapshot = SessionRenderSnapshot::capture(context, &self.session_id);
        let mut state = self.state.lock();
        self.bind_reactive_state(&mut state, surface, area);
        let layout = self.plan_render_layout(&mut state, &snapshot, area);
        self.render_main(
            &mut state,
            context,
            &snapshot,
            surface,
            layout.main_area,
            prompt,
        );

        match layout.sidebar {
            SessionSidebarLayout::Docked { sidebar_area } => {
                self.render_sidebar_docked(&mut state, context, surface, sidebar_area);
            }
            SessionSidebarLayout::Overlay => {
                self.render_sidebar_overlay(&mut state, context, surface, area);
            }
            SessionSidebarLayout::Hidden => {
                self.render_sidebar_open_button(&mut state, context, surface, area);
            }
        }
    }

    fn bind_reactive_state(
        &self,
        state: &mut SessionViewState,
        surface: &mut BufferSurface<'_>,
        area: Rect,
    ) {
        if with_current_fiber(|_| ()).is_none() {
            return;
        }

        let binder = Element::component(SessionStateBinderComponent {
            bindings: state.reactive_bindings.clone(),
            pending_actions: state.pending_actions.clone(),
            viewport_seed: state.viewport.clone(),
            reasoning_seed: state.reasoning.clone(),
            sidebar_seed: state.sidebar.clone(),
        })
        .with_key(format!("session-reactive-state:{}", self.session_id));
        binder.render(area, surface.buffer_mut());

        let bindings = state.reactive_bindings.lock().clone();
        if state.viewport_dirty {
            if bindings.viewport == state.viewport {
                state.viewport_dirty = false;
            }
        } else {
            state.viewport = bindings.viewport;
        }
        if state.reasoning_dirty {
            if bindings.reasoning == state.reasoning {
                state.reasoning_dirty = false;
            }
        } else {
            state.reasoning = bindings.reasoning;
        }
        if state.sidebar_dirty {
            if bindings.sidebar == state.sidebar {
                state.sidebar_dirty = false;
            }
        } else {
            state.sidebar = bindings.sidebar;
        }
    }

    fn plan_render_layout(
        &self,
        state: &mut SessionViewState,
        _snapshot: &SessionRenderSnapshot,
        area: Rect,
    ) -> SessionRenderLayout {
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.last_terminal_width = area.width;
        let show_sidebar = session_sidebar_visible(&next_sidebar.lifecycle, area.width);
        let docked_sidebar =
            show_sidebar && area.width > crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD;

        let layout = if docked_sidebar {
            next_sidebar.open_button_area = None;
            next_sidebar.backdrop_area = None;
            next_sidebar.close_button_area = None;
            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(SIDEBAR_WIDTH.min(area.width)),
                ])
                .split(area);
            SessionRenderLayout {
                main_area: layout[0],
                sidebar: SessionSidebarLayout::Docked {
                    sidebar_area: layout[1],
                },
            }
        } else if show_sidebar {
            next_sidebar.open_button_area = None;
            SessionRenderLayout {
                main_area: area,
                sidebar: SessionSidebarLayout::Overlay,
            }
        } else {
            next_sidebar.render_state.reset_hidden();
            next_sidebar.backdrop_area = None;
            next_sidebar.close_button_area = None;
            SessionRenderLayout {
                main_area: area,
                sidebar: SessionSidebarLayout::Hidden,
            }
        };

        state.update_sidebar_state(next_sidebar);
        layout
    }

    fn render_sidebar_docked<S: RenderSurface>(
        &self,
        state: &mut SessionViewState,
        context: &Arc<AppContext>,
        surface: &mut S,
        area: Rect,
    ) {
        if area.width == 0 || area.height == 0 {
            let mut next_sidebar = state.sidebar.clone();
            next_sidebar.close_button_area = None;
            state.update_sidebar_state(next_sidebar);
            return;
        }

        let sidebar = Sidebar::new(context.clone(), self.session_id.clone());
        let theme = context.theme.read();
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.close_button_area = Some(Rect {
            x: area
                .x
                .saturating_add(area.width.saturating_sub(SIDEBAR_CLOSE_BUTTON_WIDTH)),
            y: area.y,
            width: SIDEBAR_CLOSE_BUTTON_WIDTH.min(area.width),
            height: 1,
        });
        sidebar.render(
            surface,
            area,
            &mut next_sidebar.render_state,
            &mut next_sidebar.lifecycle,
            false,
        );
        if let Some(close_area) = next_sidebar.close_button_area {
            let close = Paragraph::new("  ✕  ")
                .style(
                    Style::default()
                        .fg(theme.text_muted)
                        .bg(theme.background_element)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(ratatui::layout::Alignment::Center);
            surface.render_widget(close, close_area);
        }
        state.update_sidebar_state(next_sidebar);
    }

    fn render_sidebar_overlay<S: RenderSurface>(
        &self,
        state: &mut SessionViewState,
        context: &Arc<AppContext>,
        surface: &mut S,
        area: Rect,
    ) {
        let theme = context.theme.read();
        let sidebar = Sidebar::new(context.clone(), self.session_id.clone());
        let mut next_sidebar = state.sidebar.clone();

        let overlay_width = SIDEBAR_WIDTH.min(area.width);
        let sidebar_area = Rect {
            x: area.x + area.width.saturating_sub(overlay_width),
            y: area.y,
            width: overlay_width,
            height: area.height,
        };
        next_sidebar.backdrop_area = Some(area);

        let sidebar_bg = tint_sidebar_overlay(theme.background_panel, theme.primary);
        let backdrop = Block::default().style(Style::default().bg(theme.background_menu));
        surface.render_widget(backdrop, area);

        surface.render_widget(Clear, sidebar_area);
        let underlay = Block::default().style(Style::default().bg(sidebar_bg));
        surface.render_widget(underlay, sidebar_area);

        next_sidebar.close_button_area = Some(Rect {
            x: sidebar_area.x.saturating_add(
                sidebar_area
                    .width
                    .saturating_sub(SIDEBAR_CLOSE_BUTTON_WIDTH),
            ),
            y: sidebar_area.y,
            width: SIDEBAR_CLOSE_BUTTON_WIDTH.min(sidebar_area.width),
            height: 1,
        });

        sidebar.render_with_bg(
            surface,
            sidebar_area,
            &mut next_sidebar.render_state,
            &mut next_sidebar.lifecycle,
            false,
            Some(sidebar_bg),
        );
        if let Some(close_area) = next_sidebar.close_button_area {
            let close = Paragraph::new("  ✕  ")
                .style(Style::default().fg(theme.text_muted).bg(theme.background_element))
                .alignment(ratatui::layout::Alignment::Center);
            surface.render_widget(close, close_area);
        }
        state.update_sidebar_state(next_sidebar);
    }

    fn render_sidebar_open_button<S: RenderSurface>(
        &self,
        state: &mut SessionViewState,
        context: &Arc<AppContext>,
        surface: &mut S,
        area: Rect,
    ) {
        if area.width == 0 || area.height == 0 {
            let mut next_sidebar = state.sidebar.clone();
            next_sidebar.open_button_area = None;
            state.update_sidebar_state(next_sidebar);
            return;
        }
        let theme = context.theme.read();
        let mut next_sidebar = state.sidebar.clone();
        let button = Rect {
            x: area
                .x
                .saturating_add(area.width.saturating_sub(SIDEBAR_OPEN_BUTTON_WIDTH)),
            y: area
                .y
                .saturating_add(1)
                .min(area.y + area.height.saturating_sub(1)),
            width: SIDEBAR_OPEN_BUTTON_WIDTH.min(area.width),
            height: 1,
        };
        next_sidebar.open_button_area = Some(button);
        let glyph = Paragraph::new("  ☰  ")
            .style(
                Style::default()
                    .fg(theme.text_muted)
                    .bg(theme.background_panel)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(ratatui::layout::Alignment::Center);
        surface.render_widget(glyph, button);
        state.update_sidebar_state(next_sidebar);
    }

    pub fn clear_sidebar_focus(&self) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let had_focus = next_sidebar.lifecycle.process_focus
            || next_sidebar.lifecycle.attached_session_focus
            || next_sidebar.lifecycle.workspace_focus;
        next_sidebar.lifecycle.process_focus = false;
        next_sidebar.lifecycle.attached_session_focus = false;
        next_sidebar.lifecycle.workspace_focus = false;
        if had_focus {
            state.update_sidebar_state(next_sidebar);
        }
        had_focus
    }

    pub fn sidebar_process_focus(&self) -> bool {
        self.state.lock().sidebar.lifecycle.process_focus
    }

    pub fn sidebar_attached_session_focus(&self) -> bool {
        self.state.lock().sidebar.lifecycle.attached_session_focus
    }

    pub fn sidebar_workspace_focus(&self) -> bool {
        self.state.lock().sidebar.lifecycle.workspace_focus
    }

    pub fn sidebar_process_selected(&self) -> usize {
        self.state.lock().sidebar.lifecycle.process_selected
    }

    pub fn sidebar_attached_session_selected(&self) -> usize {
        self.state.lock().sidebar.lifecycle.attached_session_selected
    }

    pub fn sidebar_workspace_node_count(&self) -> usize {
        self.state
            .lock()
            .sidebar
            .render_state
            .workspace_visible_count()
    }

    pub fn move_sidebar_process_selection_up(&self) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.lifecycle.process_selected =
            next_sidebar.lifecycle.process_selected.saturating_sub(1);
        state.update_sidebar_state(next_sidebar);
    }

    pub fn move_sidebar_process_selection_down(&self, count: usize) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if count > 0 {
            next_sidebar.lifecycle.process_selected =
                (next_sidebar.lifecycle.process_selected + 1).min(count - 1);
        }
        state.update_sidebar_state(next_sidebar);
    }

    pub fn clamp_sidebar_process_selection(&self, count: usize) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if count == 0 {
            next_sidebar.lifecycle.process_selected = 0;
        } else if next_sidebar.lifecycle.process_selected >= count {
            next_sidebar.lifecycle.process_selected = count - 1;
        }
        state.update_sidebar_state(next_sidebar);
    }

    pub fn move_sidebar_attached_session_selection_up(&self) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.lifecycle.attached_session_selected = next_sidebar
            .lifecycle
            .attached_session_selected
            .saturating_sub(1);
        state.update_sidebar_state(next_sidebar);
    }

    pub fn move_sidebar_attached_session_selection_down(&self, count: usize) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if count > 0 {
            next_sidebar.lifecycle.attached_session_selected =
                (next_sidebar.lifecycle.attached_session_selected + 1).min(count - 1);
        }
        state.update_sidebar_state(next_sidebar);
    }

    pub fn clamp_sidebar_attached_session_selection(&self, count: usize) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if count == 0 {
            next_sidebar.lifecycle.attached_session_selected = 0;
        } else if next_sidebar.lifecycle.attached_session_selected >= count {
            next_sidebar.lifecycle.attached_session_selected = count - 1;
        }
        state.update_sidebar_state(next_sidebar);
    }

    pub fn move_sidebar_workspace_selection_up(&self) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let next_index = next_sidebar.lifecycle.workspace_selected.saturating_sub(1);
        next_sidebar
            .render_state
            .set_workspace_selected_index(&mut next_sidebar.lifecycle, next_index);
        state.update_sidebar_state(next_sidebar);
    }

    pub fn move_sidebar_workspace_selection_down(&self, count: usize) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if count > 0 {
            let next_index = (next_sidebar.lifecycle.workspace_selected + 1).min(count - 1);
            next_sidebar
                .render_state
                .set_workspace_selected_index(&mut next_sidebar.lifecycle, next_index);
        }
        state.update_sidebar_state(next_sidebar);
    }

    pub fn expand_sidebar_workspace_selection(&self) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let changed = next_sidebar
            .render_state
            .expand_selected_workspace_dir(&mut next_sidebar.lifecycle);
        if changed {
            state.update_sidebar_state(next_sidebar);
        }
        changed
    }

    pub fn collapse_sidebar_workspace_selection(&self) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let changed = next_sidebar
            .render_state
            .collapse_selected_workspace_dir(&mut next_sidebar.lifecycle);
        if changed {
            state.update_sidebar_state(next_sidebar);
        }
        changed
    }

    pub fn toggle_sidebar_process_focus(&self, terminal_width: u16) -> bool {
        let mut state = self.state.lock();
        if !session_sidebar_visible(&state.sidebar.lifecycle, terminal_width) {
            return false;
        }
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.lifecycle.active_tab = crate::context::SidebarTab::Session;
        next_sidebar.lifecycle.process_focus = !next_sidebar.lifecycle.process_focus;
        if next_sidebar.lifecycle.process_focus {
            next_sidebar.lifecycle.attached_session_focus = false;
            next_sidebar.lifecycle.workspace_focus = false;
        }
        state.update_sidebar_state(next_sidebar);
        true
    }

    pub fn toggle_sidebar_attached_session_focus(&self, terminal_width: u16) -> bool {
        let mut state = self.state.lock();
        if !session_sidebar_visible(&state.sidebar.lifecycle, terminal_width) {
            return false;
        }
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.lifecycle.active_tab = crate::context::SidebarTab::Session;
        next_sidebar.lifecycle.attached_session_focus = !next_sidebar.lifecycle.attached_session_focus;
        if next_sidebar.lifecycle.attached_session_focus {
            next_sidebar.lifecycle.process_focus = false;
            next_sidebar.lifecycle.workspace_focus = false;
        }
        state.update_sidebar_state(next_sidebar);
        true
    }

    pub fn toggle_sidebar_workspace_focus(&self, terminal_width: u16) -> bool {
        let mut state = self.state.lock();
        if !session_sidebar_visible(&state.sidebar.lifecycle, terminal_width) {
            return false;
        }
        let mut next_sidebar = state.sidebar.clone();
        next_sidebar.lifecycle.active_tab = crate::context::SidebarTab::Workspace;
        next_sidebar.lifecycle.workspace_focus = !next_sidebar.lifecycle.workspace_focus;
        if next_sidebar.lifecycle.workspace_focus {
            next_sidebar.lifecycle.process_focus = false;
            next_sidebar.lifecycle.attached_session_focus = false;
        }
        state.update_sidebar_state(next_sidebar);
        true
    }

    pub fn sidebar_visible(&self, terminal_width: u16) -> bool {
        session_sidebar_visible(&self.state.lock().sidebar.lifecycle, terminal_width)
    }

    pub fn toggle_sidebar(&self, terminal_width: u16) {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if session_sidebar_visible(&next_sidebar.lifecycle, terminal_width) {
            if terminal_width > crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD {
                next_sidebar.lifecycle.mode = SidebarMode::Hide;
            }
            next_sidebar.lifecycle.visible = false;
            next_sidebar.lifecycle.process_focus = false;
            next_sidebar.lifecycle.attached_session_focus = false;
            next_sidebar.lifecycle.workspace_focus = false;
        } else {
            next_sidebar.lifecycle.mode = SidebarMode::Auto;
            next_sidebar.lifecycle.visible =
                terminal_width <= crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD;
        }
        state.update_sidebar_state(next_sidebar);
    }

    fn render_main(
        &self,
        state: &mut SessionViewState,
        context: &Arc<AppContext>,
        snapshot: &SessionRenderSnapshot,
        surface: &mut BufferSurface<'_>,
        area: Rect,
        prompt: &Prompt,
    ) {
        let Some(layout) = self.plan_main_layout(state, snapshot, area, prompt) else {
            return;
        };

        if layout.show_header && layout.header_area.height > 0 {
            self.render_header(snapshot, surface, layout.header_area);
        }
        self.render_messages(state, context, surface, layout.messages_area);
        if layout.show_prompt && layout.prompt_area.height > 0 {
            prompt.render(surface, layout.prompt_area);
        }
    }

    fn plan_main_layout(
        &self,
        state: &SessionViewState,
        snapshot: &SessionRenderSnapshot,
        area: Rect,
        prompt: &Prompt,
    ) -> Option<MainPaneLayout> {
        if area.width == 0 || area.height == 0 {
            return None;
        }

        let show_header = snapshot.show_header;
        let header_height = if show_header { 3u16 } else { 0u16 };
        let desired_prompt_height = prompt.desired_height(area.width);
        let total_height = area.height;
        let available_after_header = total_height.saturating_sub(header_height);
        let prompt_empty = prompt.get_input().trim().is_empty();
        let viewport_height = if state.viewport.messages_viewport_height == 0 {
            usize::from(available_after_header)
        } else {
            state.viewport.messages_viewport_height
        };
        let near_bottom = state.viewport.scroll_offset.saturating_add(viewport_height)
            >= state.viewport.rendered_line_count;
        let show_prompt = !prompt_empty || near_bottom;
        let prompt_height = if show_prompt {
            desired_prompt_height.min(available_after_header)
        } else {
            0
        };

        let layout = if !show_prompt {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Min(0),
                    Constraint::Length(0),
                    Constraint::Length(0),
                ])
                .split(area)
        } else if available_after_header <= prompt_height {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Min(0),
                    Constraint::Length(prompt_height.min(available_after_header)),
                    Constraint::Min(0),
                ])
                .split(area)
        } else {
            let max_messages_height = available_after_header.saturating_sub(prompt_height);
            let desired_messages_height = if state.viewport.rendered_line_count == 0 {
                // First-frame bootstrap: before the message viewport has measured
                // its rendered height, reserve the available body area for the
                // transcript instead of collapsing it to a single row.
                max_messages_height.max(1)
            } else {
                (state.viewport.rendered_line_count as u16)
                    .max(1)
                    .min(max_messages_height)
            };

            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_height),
                    Constraint::Length(desired_messages_height),
                    Constraint::Length(prompt_height),
                    Constraint::Min(0),
                ])
                .split(area)
        };

        Some(MainPaneLayout {
            header_area: layout[0],
            messages_area: layout[1],
            prompt_area: layout[2],
            show_header,
            show_prompt,
        })
    }

    fn render_header<S: RenderSurface>(
        &self,
        snapshot: &SessionRenderSnapshot,
        surface: &mut S,
        area: Rect,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = &snapshot.theme;
        let inner_width = usize::from(area.width.saturating_sub(4));
        let status_accent = if snapshot.header.status_retrying {
            theme.warning
        } else {
            theme.primary
        };
        let header_border = if snapshot.header.status_running {
            if snapshot.header.status_retrying {
                theme.warning
            } else {
                theme.border_active
            }
        } else {
            theme.border_subtle
        };

        let mut title_width = UnicodeWidthStr::width(snapshot.header.title.as_str());
        let mut title_spans = Vec::new();

        if area.width >= HEADER_NARROW_THRESHOLD {
            if let Some(parent_title) = snapshot.header.parent_title.as_ref() {
                title_width = title_width
                    .saturating_add(UnicodeWidthStr::width(parent_title.as_str()))
                    .saturating_add(3);
                title_spans.push(Span::styled(
                    parent_title.clone(),
                    Style::default().fg(theme.text_muted),
                ));
                title_spans.push(Span::styled(" / ", Style::default().fg(theme.border)));
            }
        }

        if snapshot.header.status_running {
            title_width = title_width.saturating_add(2);
            let running_prefix = if snapshot.header.title == "New Session" {
                "☯ "
            } else {
                "◐ "
            };
            title_spans.push(Span::styled(
                running_prefix,
                Style::default().fg(status_accent),
            ));
        }

        title_spans.push(Span::styled(
            snapshot.header.title.clone(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ));

        if let Some(status_label) = snapshot.header.status_label.as_ref() {
            let status_width = UnicodeWidthStr::width(status_label.as_str());
            if inner_width > title_width.saturating_add(status_width).saturating_add(2) {
                let padding = inner_width.saturating_sub(title_width + status_width);
                title_spans.push(Span::raw(" ".repeat(padding)));
                title_spans.push(Span::styled(
                    status_label.clone(),
                    Style::default()
                        .fg(status_accent)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }

        let subtitle = snapshot.header.subtitle.as_ref();
        let usage = snapshot.header.usage.as_ref();
        let subtitle_line = match (subtitle, usage) {
            (Some(left), Some(right))
                if inner_width
                    > UnicodeWidthStr::width(left.as_str())
                        .saturating_add(UnicodeWidthStr::width(right.as_str()))
                        .saturating_add(2) =>
            {
                let padding = inner_width
                    .saturating_sub(UnicodeWidthStr::width(left.as_str()))
                    .saturating_sub(UnicodeWidthStr::width(right.as_str()));
                Line::from(vec![
                    Span::styled(left.clone(), Style::default().fg(theme.text_muted)),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(right.clone(), Style::default().fg(theme.text_muted)),
                ])
            }
            (Some(left), Some(right)) => Line::from(vec![
                Span::styled(left.clone(), Style::default().fg(theme.text_muted)),
                Span::styled("  ·  ", Style::default().fg(theme.border)),
                Span::styled(right.clone(), Style::default().fg(theme.text_muted)),
            ]),
            (Some(left), None) => Line::from(Span::styled(
                left.clone(),
                Style::default().fg(theme.text_muted),
            )),
            (None, Some(right)) => Line::from(Span::styled(
                right.clone(),
                Style::default().fg(theme.text_muted),
            )),
            (None, None) => Line::from(""),
        };
        let content = vec![Line::from(title_spans), subtitle_line];

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(header_border))
                    .padding(ratatui::widgets::Padding::new(1, 1, 0, 0)),
            )
            .style(Style::default().bg(theme.background_panel));

        surface.render_widget(paragraph, area);
    }

    fn render_messages(
        &self,
        state: &mut SessionViewState,
        context: &Arc<AppContext>,
        surface: &mut BufferSurface<'_>,
        area: Rect,
    ) {
        if with_current_fiber(|_| ()).is_none() {
            let snapshot = SessionMessagesSnapshot::capture(context, &self.session_id);
            let output = render_session_messages_child(
                area,
                &snapshot,
                &state.viewport,
                &state.reasoning,
                &SessionMessageOutputCache::default(),
                &SessionRenderModelCache::default(),
                surface.buffer_mut(),
            );
            state.update_reasoning_state(output.reasoning);
            state.update_viewport_state(output.viewport);
            return;
        }

        let output = Arc::new(Mutex::new(None));
        let child = Element::component(SessionMessagesComponent {
            area,
            viewport: state.viewport.clone(),
            reasoning: state.reasoning.clone(),
            output: output.clone(),
        })
        .with_key(format!("session-messages:{}", self.session_id));
        child.render(area, surface.buffer_mut());

        let Some(output) = output.lock().take() else {
            return;
        };
        state.update_reasoning_state(output.reasoning);
        state.update_viewport_state(output.viewport);
    }

    pub fn handle_click(&self, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        let Some(area) = state.viewport.last_messages_area else {
            return false;
        };

        let max_x = area.x.saturating_add(area.width);
        let max_y = area.y.saturating_add(area.height);
        if col < area.x || col >= max_x || row < area.y || row >= max_y {
            return false;
        }

        let line_index = state.viewport.scroll_offset + usize::from(row.saturating_sub(area.y));
        if line_index >= state.viewport.rendered_line_count {
            return false;
        }
        if let Some(reasoning_id) = state
            .reasoning
            .toggle_hits
            .iter()
            .find(|hit| hit.line_index == line_index)
            .map(|hit| hit.reasoning_id.clone())
        {
            state.queue_interaction_action(SessionInteractionAction::ToggleReasoning(reasoning_id));
            return true;
        }

        if let Some(arguments_id) = state
            .reasoning
            .tool_arguments_toggle_hits
            .iter()
            .find(|hit| hit.line_index == line_index)
            .map(|hit| hit.arguments_id.clone())
        {
            state.queue_interaction_action(SessionInteractionAction::ToggleToolArguments(
                arguments_id,
            ));
            return true;
        }

        false
    }

    pub fn handle_scrollbar_click(&self, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        if !point_in_optional_rect(state.viewport.last_scrollbar_area, col, row) {
            return false;
        }
        state.queue_interaction_action(SessionInteractionAction::SetScrollbarDrag(true));
        self.scroll_to_scrollbar_row(&mut state, row);
        true
    }

    pub fn handle_scrollbar_drag(&self, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        if !state.viewport.scrollbar_drag_active
            && !point_in_optional_rect(state.viewport.last_scrollbar_area, col, row)
        {
            return false;
        }
        state.queue_interaction_action(SessionInteractionAction::SetScrollbarDrag(true));
        self.scroll_to_scrollbar_row(&mut state, row);
        true
    }

    pub fn stop_scrollbar_drag(&self) -> bool {
        let mut state = self.state.lock();
        let was_active = state.viewport.scrollbar_drag_active;
        if was_active {
            state.queue_interaction_action(SessionInteractionAction::SetScrollbarDrag(false));
        }
        was_active
    }

    fn scroll_to_scrollbar_row(&self, state: &mut SessionViewState, row: u16) {
        let max_scroll = state.max_scroll_offset();
        let next_offset =
            map_scrollbar_row_to_offset(state.viewport.last_scrollbar_area, row, max_scroll);
        state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
    }

    pub fn handle_sidebar_click(&self, _context: &Arc<AppContext>, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        if point_in_optional_rect(next_sidebar.open_button_area, col, row) {
            next_sidebar.lifecycle.mode = SidebarMode::Auto;
            next_sidebar.lifecycle.visible =
                state.sidebar.last_terminal_width <= crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD;
            next_sidebar.open_button_area = None;
            state.update_sidebar_state(next_sidebar);
            return true;
        }
        if point_in_optional_rect(next_sidebar.close_button_area, col, row) {
            if state.sidebar.last_terminal_width > crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD {
                next_sidebar.lifecycle.mode = SidebarMode::Hide;
            }
            next_sidebar.lifecycle.visible = false;
            next_sidebar.lifecycle.process_focus = false;
            next_sidebar.lifecycle.attached_session_focus = false;
            next_sidebar.lifecycle.workspace_focus = false;
            next_sidebar.close_button_area = None;
            state.update_sidebar_state(next_sidebar);
            return true;
        }
        if point_in_optional_rect(next_sidebar.backdrop_area, col, row)
            && !next_sidebar.render_state.contains_sidebar_point(col, row)
        {
            next_sidebar.lifecycle.visible = false;
            next_sidebar.lifecycle.process_focus = false;
            next_sidebar.lifecycle.attached_session_focus = false;
            next_sidebar.lifecycle.workspace_focus = false;
            next_sidebar.backdrop_area = None;
            next_sidebar.close_button_area = None;
            state.update_sidebar_state(next_sidebar);
            return true;
        }
        if !next_sidebar
            .render_state
            .handle_click(&mut next_sidebar.lifecycle, col, row)
        {
            return false;
        }
        state.pending_navigate_attached = next_sidebar.render_state.take_pending_navigate_attached();
        state.pending_navigate_session = next_sidebar.render_state.take_pending_navigate_session();
        state.pending_navigate_parent = next_sidebar.render_state.take_pending_navigate_parent();
        state.update_sidebar_state(next_sidebar);
        true
    }

    pub fn is_point_in_sidebar(&self, col: u16, row: u16) -> bool {
        self.state
            .lock()
            .sidebar
            .render_state
            .contains_sidebar_point(col, row)
    }

    pub fn selection_area(&self) -> Option<Rect> {
        self.state.lock().viewport.last_messages_area
    }

    pub fn scroll_sidebar_up_at(&self, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let changed = next_sidebar.render_state.scroll_up_at(col, row);
        if changed {
            state.update_sidebar_state(next_sidebar);
        }
        changed
    }

    pub fn scroll_sidebar_down_at(&self, col: u16, row: u16) -> bool {
        let mut state = self.state.lock();
        let mut next_sidebar = state.sidebar.clone();
        let changed = next_sidebar.render_state.scroll_down_at(col, row);
        if changed {
            state.update_sidebar_state(next_sidebar);
        }
        changed
    }

    pub fn take_pending_navigate_attached(&self) -> Option<usize> {
        self.state.lock().pending_navigate_attached.take()
    }

    pub fn take_pending_navigate_session(&self) -> Option<String> {
        self.state.lock().pending_navigate_session.take()
    }

    pub fn take_pending_navigate_parent(&self) -> bool {
        std::mem::take(&mut self.state.lock().pending_navigate_parent)
    }

    pub fn scroll_up(&self) {
        let mut state = self.state.lock();
        if state.viewport.scroll_offset > 0 {
            let next_offset = state.viewport.scroll_offset.saturating_sub(1);
            state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
        }
    }

    pub fn scroll_down(&self) {
        let mut state = self.state.lock();
        let max_scroll = state.max_scroll_offset();
        if state.viewport.scroll_offset < max_scroll {
            let next_offset = state
                .viewport
                .scroll_offset
                .saturating_add(1)
                .min(max_scroll);
            state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
        }
    }

    pub fn scroll_up_by(&self, lines: usize) {
        let lines = lines.max(1);
        let mut state = self.state.lock();
        let next_offset = state.viewport.scroll_offset.saturating_sub(lines);
        state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
    }

    pub fn scroll_down_by(&self, lines: usize) {
        let lines = lines.max(1);
        let mut state = self.state.lock();
        let max_scroll = state.max_scroll_offset();
        let next_offset = state
            .viewport
            .scroll_offset
            .saturating_add(lines)
            .min(max_scroll);
        state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
    }

    pub fn scroll_up_mouse(&self) {
        self.scroll_up_by(MOUSE_SCROLL_LINES);
    }

    pub fn scroll_down_mouse(&self) {
        self.scroll_down_by(MOUSE_SCROLL_LINES);
    }

    pub fn handle_mouse_move(&self, col: u16, row: u16) -> bool {
        let _ = (col, row);
        false
    }

    pub fn next_tooltip_tick_after(&self) -> Option<Duration> {
        None
    }

    pub fn scroll_page_up(&self) {
        let mut state = self.state.lock();
        let step = state
            .viewport
            .messages_viewport_height
            .saturating_sub(1)
            .max(1);
        let next_offset = state.viewport.scroll_offset.saturating_sub(step);
        state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
    }

    pub fn scroll_page_down(&self) {
        let mut state = self.state.lock();
        let step = state
            .viewport
            .messages_viewport_height
            .saturating_sub(1)
            .max(1);
        let max_scroll = state.max_scroll_offset();
        let next_offset = state
            .viewport
            .scroll_offset
            .saturating_add(step)
            .min(max_scroll);
        state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
    }

    pub fn scroll_to_message(&self, context: &Arc<AppContext>, message_id: &str) {
        let mut state = self.state.lock();
        if let Some(first_line) = state.viewport.message_first_lines.get(message_id).copied() {
            let next_offset = first_line.min(state.max_scroll_offset());
            state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(next_offset));
            return;
        }

        let session_ctx = context.session.read();
        if let Some(messages) = session_ctx.messages.get(&self.session_id) {
            if let Some(idx) = messages.iter().position(|m| m.id == message_id) {
                let max_scroll = state.max_scroll_offset();
                state.queue_interaction_action(SessionInteractionAction::SetScrollOffset(
                    idx.saturating_mul(3).min(max_scroll),
                ));
            }
        }
    }
}
