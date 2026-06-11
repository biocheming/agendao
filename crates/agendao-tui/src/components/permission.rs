use std::cell::{Cell, RefCell};

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
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

fn permission_class_label(value: &str) -> String {
    match value {
        "inspect_read" => "Inspect read".to_string(),
        "workspace_write" => "Workspace write".to_string(),
        "external_access" => "External access".to_string(),
        "dangerous_exec" => "Dangerous execution".to_string(),
        other => other.replace('_', " "),
    }
}

fn lifetime_hint(
    scope: Option<&str>,
    supported_lifetimes: &[PermissionLifetime],
) -> Option<String> {
    if supported_lifetimes.is_empty() {
        return None;
    }

    let mut parts = vec!["once = this request".to_string()];
    if supported_lifetimes.contains(&PermissionLifetime::Turn) {
        parts.push(match scope {
            Some(scope) => format!("turn = current turn for {scope}"),
            None => "turn = current turn".to_string(),
        });
    }
    if supported_lifetimes.contains(&PermissionLifetime::Session) {
        parts.push(match scope {
            Some(scope) => format!("session = this session for {scope}"),
            None => "session = this session".to_string(),
        });
    }
    Some(parts.join("  |  "))
}

fn permission_action_chip_with_text(
    label: &'static str,
    bg: ratatui::style::Color,
) -> Span<'static> {
    Span::styled(
        format!(" {} ", label),
        Style::default()
            .fg(ratatui::style::Color::Black)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    )
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub id: String,
    pub permission_type: PermissionType,
    pub resource: String,
    pub tool_name: String,
    pub permission_class: Option<String>,
    pub scope_key: Option<String>,
    pub scope_label: Option<String>,
    pub matcher_label: Option<String>,
    pub grant_target_summary: Option<String>,
    pub risk_tags: Vec<String>,
    pub supported_lifetimes: Vec<PermissionLifetime>,
    pub is_submitting: bool,
    pub submit_error: Option<String>,
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
    click_targets: RefCell<Vec<PermissionClickTarget>>,
}

#[derive(Clone, Debug, PartialEq)]
struct PermissionClickTarget {
    row: u16,
    start_col: u16,
    end_col: u16,
    action: PermissionAction,
}

impl PermissionPrompt {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            current_index: 0,
            is_open: false,
            last_rendered_area: Cell::new(None),
            pending_action: None,
            click_targets: RefCell::new(Vec::new()),
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

    pub fn requests(&self) -> &[PermissionRequest] {
        &self.requests
    }

    fn clone_current_request(&self) -> Option<PermissionRequest> {
        self.current_request().cloned()
    }

    pub fn approve_once(&mut self) -> Option<PermissionRequest> {
        self.clone_current_request()
    }

    pub fn approve_turn(&mut self) -> Option<PermissionRequest> {
        self.clone_current_request()
    }

    pub fn approve_session(&mut self) -> Option<PermissionRequest> {
        self.clone_current_request()
    }

    pub fn deny(&mut self) -> Option<PermissionRequest> {
        self.clone_current_request()
    }

    pub fn mark_submitting(&mut self, request_id: &str) -> bool {
        let Some(request) = self
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        else {
            return false;
        };
        request.is_submitting = true;
        request.submit_error = None;
        true
    }

    pub fn mark_submit_failed(&mut self, request_id: &str, message: String) -> bool {
        let Some(request) = self
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        else {
            return false;
        };
        request.is_submitting = false;
        request.submit_error = Some(message);
        true
    }

    pub fn clear_submit_state(&mut self, request_id: &str) -> bool {
        let Some(request) = self
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        else {
            return false;
        };
        let changed = request.is_submitting || request.submit_error.is_some();
        request.is_submitting = false;
        request.submit_error = None;
        changed
    }

    pub fn is_current_request_submitting(&self) -> bool {
        self.current_request()
            .map(|request| request.is_submitting)
            .unwrap_or(false)
    }

    pub fn submitting_count(&self) -> usize {
        self.requests
            .iter()
            .filter(|request| request.is_submitting)
            .count()
    }

    pub fn last_submit_error(&self) -> Option<&str> {
        self.requests
            .iter()
            .rev()
            .find_map(|request| request.submit_error.as_deref())
    }

    pub fn close(&mut self) {
        self.requests.clear();
        self.current_index = 0;
        self.is_open = false;
        self.click_targets.borrow_mut().clear();
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
        if self.is_current_request_submitting() {
            return;
        }
        if let Some(target) = self
            .click_targets
            .borrow()
            .iter()
            .cloned()
            .find(|target| row == target.row && col >= target.start_col && col <= target.end_col)
        {
            self.pending_action = Some(target.action);
        }
    }

    pub fn take_pending_action(&mut self) -> Option<PermissionAction> {
        self.pending_action.take()
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.is_open || self.requests.is_empty() {
            return;
        }
        self.click_targets.borrow_mut().clear();

        let request = match self.current_request() {
            Some(r) => r,
            None => return,
        };

        let width = area.width.saturating_sub(2).min(80);
        let actions = if request.is_submitting {
            vec![Span::styled(
                "Submitting permission reply...",
                Style::default().fg(theme.warning),
            )]
        } else {
            let mut actions = vec![permission_action_chip_with_text("[1] Once", theme.success)];
            if request
                .supported_lifetimes
                .contains(&PermissionLifetime::Turn)
            {
                actions.push(Span::raw(" "));
                actions.push(permission_action_chip_with_text("[2] Turn", theme.primary));
            }
            if request
                .supported_lifetimes
                .contains(&PermissionLifetime::Session)
            {
                actions.push(Span::raw(" "));
                actions.push(permission_action_chip_with_text("[3] Session", theme.primary));
            }
            actions.push(Span::raw(" "));
            actions.push(permission_action_chip_with_text("[0] Deny", theme.error));
            actions
        };

        let title = format!(
            "{} {} - Permission Request",
            request.permission_type.icon(),
            request.permission_type.label()
        );

        let mut content = vec![
            Line::from(Span::styled(
                &title,
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.tool_name, Style::default().fg(theme.text)),
            ]),
        ];
        if let Some(class) = request.permission_class.as_ref() {
            content.push(Line::from(vec![
                Span::styled("Class: ", Style::default().fg(theme.text_muted)),
                Span::styled(
                    permission_class_label(class),
                    Style::default().fg(theme.text),
                ),
            ]));
        }
        if let Some(scope_label) = request.scope_label.as_ref().or(request.scope_key.as_ref()) {
            content.push(Line::from(vec![
                Span::styled("Scope: ", Style::default().fg(theme.text_muted)),
                Span::styled(scope_label, Style::default().fg(theme.text)),
            ]));
        }
        if let Some(target) = request.grant_target_summary.as_ref() {
            content.push(Line::from(vec![
                Span::styled("Target: ", Style::default().fg(theme.text_muted)),
                Span::styled(target, Style::default().fg(theme.text)),
            ]));
        }
        if let Some(matcher) = request.matcher_label.as_ref() {
            content.push(Line::from(vec![
                Span::styled("Match: ", Style::default().fg(theme.text_muted)),
                Span::styled(matcher, Style::default().fg(theme.text)),
            ]));
        }
        if let Some(hint) = lifetime_hint(
            request
                .grant_target_summary
                .as_deref()
                .or(request.scope_label.as_deref())
                .or(request.scope_key.as_deref()),
            &request.supported_lifetimes,
        ) {
            content.push(Line::from(vec![
                Span::styled("Grant: ", Style::default().fg(theme.text_muted)),
                Span::styled(hint, Style::default().fg(theme.text)),
            ]));
        }
        if !request.risk_tags.is_empty() {
            content.push(Line::from(vec![
                Span::styled("Risk: ", Style::default().fg(theme.text_muted)),
                Span::styled(
                    request.risk_tags.join(", "),
                    Style::default().fg(theme.text),
                ),
            ]));
        }
        content.extend([
            Line::from(vec![
                Span::styled("Resource: ", Style::default().fg(theme.text_muted)),
                Span::styled(&request.resource, Style::default().fg(theme.text)),
            ]),
            Line::from(""),
            Line::from(actions),
        ]);
        if let Some(error) = request.submit_error.as_ref() {
            content.push(Line::from(""));
            content.push(Line::from(vec![
                Span::styled("Last error: ", Style::default().fg(theme.error)),
                Span::styled(error, Style::default().fg(theme.text)),
            ]));
        }

        let height = (content.len() as u16)
            .saturating_add(2)
            .min(area.height.saturating_sub(1));

        // Render inline at the bottom of the session area
        let popup_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        );

        self.last_rendered_area.set(Some(popup_area));
        if !request.is_submitting {
            let mut click_targets = Vec::new();
            let button_row = popup_area.y + popup_area.height.saturating_sub(2);
            let mut cursor_col = popup_area.x + 1;

            let once_label = " [1] Once ";
            click_targets.push(PermissionClickTarget {
                row: button_row,
                start_col: cursor_col,
                end_col: cursor_col + once_label.len() as u16 - 1,
                action: PermissionAction::ApproveOnce,
            });
            cursor_col += once_label.len() as u16;

            if request
                .supported_lifetimes
                .contains(&PermissionLifetime::Turn)
            {
                cursor_col += 1;
                let turn_label = " [2] Turn ";
                click_targets.push(PermissionClickTarget {
                    row: button_row,
                    start_col: cursor_col,
                    end_col: cursor_col + turn_label.len() as u16 - 1,
                    action: PermissionAction::ApproveTurn,
                });
                cursor_col += turn_label.len() as u16;
            }

            if request
                .supported_lifetimes
                .contains(&PermissionLifetime::Session)
            {
                cursor_col += 1;
                let session_label = " [3] Session ";
                click_targets.push(PermissionClickTarget {
                    row: button_row,
                    start_col: cursor_col,
                    end_col: cursor_col + session_label.len() as u16 - 1,
                    action: PermissionAction::ApproveSession,
                });
                cursor_col += session_label.len() as u16;
            }

            cursor_col += 1;
            let deny_label = " [0] Deny ";
            click_targets.push(PermissionClickTarget {
                row: button_row,
                start_col: cursor_col,
                end_col: cursor_col + deny_label.len() as u16 - 1,
                action: PermissionAction::Deny,
            });
            self.click_targets.replace(click_targets);
        }

        // Clear underlying content so no text bleeds through
        surface.render_widget(Clear, popup_area);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{theme::Theme, ui::BufferSurface};
    use ratatui::buffer::Buffer;

    fn sample_request() -> PermissionRequest {
        PermissionRequest {
            id: "perm-1".to_string(),
            permission_type: PermissionType::Bash,
            resource: "cargo test".to_string(),
            tool_name: "bash".to_string(),
            permission_class: Some("dangerous_exec".to_string()),
            scope_key: None,
            scope_label: None,
            matcher_label: None,
            grant_target_summary: None,
            risk_tags: Vec::new(),
            supported_lifetimes: vec![PermissionLifetime::Once],
            is_submitting: false,
            submit_error: None,
        }
    }

    #[test]
    fn approve_keeps_request_until_authority_clears_it() {
        let mut prompt = PermissionPrompt::new();
        prompt.add_request(sample_request());

        let approved = prompt.approve_once().expect("request should exist");
        assert_eq!(approved.id, "perm-1");
        assert_eq!(prompt.pending_count(), 1);
        assert!(prompt.current_request().is_some());

        let removed = prompt
            .remove_request("perm-1")
            .expect("request should remove");
        assert_eq!(removed.id, "perm-1");
        assert_eq!(prompt.pending_count(), 0);
    }

    #[test]
    fn submitting_and_failed_states_are_retained_locally() {
        let mut prompt = PermissionPrompt::new();
        prompt.add_request(sample_request());

        assert!(prompt.mark_submitting("perm-1"));
        assert!(prompt.is_current_request_submitting());
        assert_eq!(
            prompt
                .current_request()
                .and_then(|req| req.submit_error.as_deref()),
            None
        );

        assert!(prompt.mark_submit_failed("perm-1", "network down".to_string()));
        let request = prompt
            .current_request()
            .expect("request should still exist");
        assert!(!request.is_submitting);
        assert_eq!(request.submit_error.as_deref(), Some("network down"));

        assert!(prompt.clear_submit_state("perm-1"));
        let request = prompt
            .current_request()
            .expect("request should still exist");
        assert!(!request.is_submitting);
        assert!(request.submit_error.is_none());
    }

    #[test]
    fn clicking_rendered_once_button_sets_pending_action() {
        let mut prompt = PermissionPrompt::new();
        prompt.add_request(sample_request());

        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
        let mut surface = BufferSurface::new(&mut buffer);
        prompt.render(&mut surface, Rect::new(0, 0, 80, 20), &Theme::default());

        let target = prompt
            .click_targets
            .borrow()
            .iter()
            .cloned()
            .find(|target| target.action == PermissionAction::ApproveOnce)
            .expect("once button target should exist");
        prompt.handle_click(target.start_col, target.row);

        assert_eq!(
            prompt.take_pending_action(),
            Some(PermissionAction::ApproveOnce)
        );
    }
}
