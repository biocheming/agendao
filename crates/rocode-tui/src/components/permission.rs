use std::cell::Cell;

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::theme::Theme;
use crate::ui::RenderSurface;

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionType {
    ReadFile,
    WriteFile,
    Edit,
    ExecuteCommand,
    Bash,
    NetworkRequest,
    Glob,
    Grep,
    List,
    Task,
    WebFetch,
    WebSearch,
    CodeSearch,
    ExternalDirectory,
}

impl PermissionType {
    pub fn label(&self) -> &'static str {
        match self {
            PermissionType::ReadFile => "Read file",
            PermissionType::WriteFile => "Write file",
            PermissionType::Edit => "Edit file",
            PermissionType::ExecuteCommand => "Execute command",
            PermissionType::Bash => "Run shell command",
            PermissionType::NetworkRequest => "Network request",
            PermissionType::Glob => "Glob search",
            PermissionType::Grep => "Grep search",
            PermissionType::List => "List directory",
            PermissionType::Task => "Task operation",
            PermissionType::WebFetch => "Fetch web content",
            PermissionType::WebSearch => "Web search",
            PermissionType::CodeSearch => "Code search",
            PermissionType::ExternalDirectory => "External directory access",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            PermissionType::ReadFile => "[R]",
            PermissionType::WriteFile => "[W]",
            PermissionType::Edit => "[E]",
            PermissionType::ExecuteCommand => "[X]",
            PermissionType::Bash => "[!]",
            PermissionType::NetworkRequest => "[N]",
            PermissionType::Glob => "[G]",
            PermissionType::Grep => "[S]",
            PermissionType::List => "[L]",
            PermissionType::Task => "[T]",
            PermissionType::WebFetch => "[F]",
            PermissionType::WebSearch => "[Q]",
            PermissionType::CodeSearch => "[C]",
            PermissionType::ExternalDirectory => "[D]",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionLifetime {
    Once,
    Turn,
    Session,
}

impl PermissionLifetime {
    pub fn label(&self) -> &'static str {
        match self {
            PermissionLifetime::Once => "once",
            PermissionLifetime::Turn => "turn",
            PermissionLifetime::Session => "session",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: String,
    pub permission_type: PermissionType,
    pub resource: String,
    pub tool_name: String,
    pub permission_class: Option<String>,
    pub supported_lifetimes: Vec<PermissionLifetime>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionAction {
    Deny,
    ApproveOnce,
    ApproveTurn,
    ApproveSession,
}

pub struct PermissionPrompt {
    requests: Vec<PermissionRequest>,
    current_index: usize,
    pub is_open: bool,
    last_rendered_area: Cell<Option<Rect>>,
    pending_action: Option<PermissionAction>,
}

impl PermissionPrompt {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            current_index: 0,
            is_open: false,
            last_rendered_area: Cell::new(None),
            pending_action: None,
        }
    }

    pub fn add_request(&mut self, request: PermissionRequest) {
        self.requests.push(request);
        self.is_open = !self.requests.is_empty();
    }

    pub fn retain_requests<F>(&mut self, mut keep: F)
    where
        F: FnMut(&PermissionRequest) -> bool,
    {
        self.requests.retain(|request| keep(request));
        if self.current_index >= self.requests.len() {
            self.current_index = self.requests.len().saturating_sub(1);
        }
        self.is_open = !self.requests.is_empty();
    }

    pub fn remove_request(&mut self, request_id: &str) -> Option<PermissionRequest> {
        let index = self
            .requests
            .iter()
            .position(|request| request.id == request_id)?;
        let request = self.requests.remove(index);
        if self.current_index >= self.requests.len() {
            self.current_index = self.requests.len().saturating_sub(1);
        }
        self.is_open = !self.requests.is_empty();
        Some(request)
    }

    pub fn current_request(&self) -> Option<&PermissionRequest> {
        self.requests.get(self.current_index)
    }

    fn take_current_request(&mut self) -> Option<PermissionRequest> {
        if self.current_index < self.requests.len() {
            let request = self.requests.remove(self.current_index);
            if self.requests.is_empty() {
                self.is_open = false;
            }
            Some(request)
        } else {
            None
        }
    }

    pub fn approve_once(&mut self) -> Option<PermissionRequest> {
        self.take_current_request()
    }

    pub fn approve_turn(&mut self) -> Option<PermissionRequest> {
        self.take_current_request()
    }

    pub fn approve_session(&mut self) -> Option<PermissionRequest> {
        self.take_current_request()
    }

    pub fn deny(&mut self) -> Option<PermissionRequest> {
        self.take_current_request()
    }

    pub fn close(&mut self) {
        self.requests.clear();
        self.current_index = 0;
        self.is_open = false;
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn pending_count(&self) -> usize {
        self.requests.len()
    }

    pub fn handle_click(&mut self, col: u16, row: u16) {
        if !self.is_open || self.requests.is_empty() {
            return;
        }
        // Button row is the last content line inside the border.
        // We store the last rendered area to check clicks.
        // For simplicity, check if the click is on the button hints row
        // and map x-position to the three buttons.
        if let Some(area) = self.last_rendered_area.get() {
            if row < area.y
                || row >= area.y + area.height
                || col < area.x
                || col >= area.x + area.width
            {
                return;
            }
            // The button line is at the bottom of the content (area.y + area.height - 2 for border)
            let button_row = area.y + area.height - 2;
            if row == button_row {
                // "[1] Once [2] Turn [3] Session [0] Deny"
                let inner_col = col.saturating_sub(area.x + 1);
                let allow_turn = self
                    .current_request()
                    .map(|request| {
                        request
                            .supported_lifetimes
                            .contains(&PermissionLifetime::Turn)
                    })
                    .unwrap_or(false);
                let allow_session = self
                    .current_request()
                    .map(|request| {
                        request
                            .supported_lifetimes
                            .contains(&PermissionLifetime::Session)
                    })
                    .unwrap_or(false);

                if inner_col < 10 {
                    self.pending_action = Some(PermissionAction::ApproveOnce);
                } else if allow_turn && inner_col < 20 {
                    self.pending_action = Some(PermissionAction::ApproveTurn);
                } else if allow_session && inner_col < 34 {
                    self.pending_action = Some(PermissionAction::ApproveSession);
                } else {
                    self.pending_action = Some(PermissionAction::Deny);
                }
            }
        }
    }

    pub fn take_pending_action(&mut self) -> Option<PermissionAction> {
        self.pending_action.take()
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.is_open || self.requests.is_empty() {
            return;
        }

        let request = match self.current_request() {
            Some(r) => r,
            None => return,
        };

        let height = 8u16;
        let width = area.width.saturating_sub(2).min(80);

        // Render inline at the bottom of the session area
        let popup_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        );

        self.last_rendered_area.set(Some(popup_area));

        // Clear underlying content so no text bleeds through
        surface.render_widget(Clear, popup_area);

        let title = format!(
            "{} {} - Permission Request",
            request.permission_type.icon(),
            request.permission_type.label()
        );
        let mut actions = vec![Span::styled("[1] Once  ", Style::default().fg(theme.success))];
        if request
            .supported_lifetimes
            .contains(&PermissionLifetime::Turn)
        {
            actions.push(Span::styled(
                "[2] Turn  ",
                Style::default().fg(theme.primary),
            ));
        }
        if request
            .supported_lifetimes
            .contains(&PermissionLifetime::Session)
        {
            actions.push(Span::styled(
                "[3] Session  ",
                Style::default().fg(theme.primary),
            ));
        }
        actions.push(Span::styled("[0] Deny", Style::default().fg(theme.error)));

        let content = vec![
            Line::from(Span::styled(
                &title,
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.tool_name, Style::default().fg(theme.text)),
            ]),
            Line::from(
                request
                    .permission_class
                    .as_ref()
                    .map(|class| {
                        vec![
                            Span::styled("Class: ", Style::default().fg(theme.text_muted)),
                            Span::styled(class, Style::default().fg(theme.text)),
                        ]
                    })
                    .unwrap_or_default(),
            ),
            Line::from(vec![
                Span::styled("Resource: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.resource, Style::default().fg(theme.text)),
            ]),
            Line::from(""),
            Line::from(actions),
        ];

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Permission ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.warning)),
            )
            .style(Style::default().bg(theme.background_panel));

        surface.render_widget(paragraph, popup_area);
    }
}

impl Default for PermissionPrompt {
    fn default() -> Self {
        Self::new()
    }
}
