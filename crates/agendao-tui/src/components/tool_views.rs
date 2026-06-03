use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::diff::{DiffMode, DiffView};
use super::tool_call::ToolCallStatus;
use crate::context::TodoStatus;
use crate::theme::Theme;

pub struct GlobToolView {
    pub pattern: String,
    pub matches: Option<Vec<String>>,
}

impl GlobToolView {
    pub fn new(pattern: String) -> Self {
        Self {
            pattern,
            matches: None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let count = self.matches.as_ref().map(|m| m.len()).unwrap_or(0);

        let line = Line::from(vec![
            Span::styled("glob ", Style::default().fg(theme.primary)),
            Span::styled(&self.pattern, Style::default().fg(theme.text)),
            Span::styled(
                format!(" ({} matches)", count),
                Style::default().fg(theme.text_muted),
            ),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct GrepToolView {
    pub pattern: String,
    pub path: Option<String>,
    pub matches: Option<u32>,
}

impl GrepToolView {
    pub fn new(pattern: String) -> Self {
        Self {
            pattern,
            path: None,
            matches: None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let count = self.matches.unwrap_or(0);

        let line = Line::from(vec![
            Span::styled("grep ", Style::default().fg(theme.primary)),
            Span::styled(&self.pattern, Style::default().fg(theme.text)),
            if let Some(p) = &self.path {
                Span::styled(format!(" in {}", p), Style::default().fg(theme.text_muted))
            } else {
                Span::raw("")
            },
            Span::styled(
                format!(" ({} matches)", count),
                Style::default().fg(theme.text_muted),
            ),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct ListToolView {
    pub path: String,
}

impl ListToolView {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let line = Line::from(vec![
            Span::styled("list ", Style::default().fg(theme.primary)),
            Span::styled(&self.path, Style::default().fg(theme.text)),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct WebfetchToolView {
    pub url: String,
}

impl WebfetchToolView {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let display_url = if self.url.chars().count() > 50 {
            format!("{}...", self.url.chars().take(47).collect::<String>())
        } else {
            self.url.clone()
        };

        let line = Line::from(vec![
            Span::styled("webfetch ", Style::default().fg(theme.primary)),
            Span::styled(display_url, Style::default().fg(theme.text)),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct WebsearchToolView {
    pub query: String,
    pub results: Option<u32>,
}

impl WebsearchToolView {
    pub fn new(query: String) -> Self {
        Self {
            query,
            results: None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let count = self.results.unwrap_or(0);

        let line = Line::from(vec![
            Span::styled("websearch ", Style::default().fg(theme.primary)),
            Span::styled(&self.query, Style::default().fg(theme.text)),
            Span::styled(
                format!(" ({} results)", count),
                Style::default().fg(theme.text_muted),
            ),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct TaskToolView {
    pub task_name: String,
    pub category: Option<String>,
    pub status: ToolCallStatus,
}

impl TaskToolView {
    pub fn new(task_name: String) -> Self {
        Self {
            task_name,
            category: None,
            status: ToolCallStatus::Running,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let status_icon = match self.status {
            ToolCallStatus::Pending => "◯",
            ToolCallStatus::Running => "◐",
            ToolCallStatus::Completed => "●",
            ToolCallStatus::Failed => "✗",
        };

        let status_color = match self.status {
            ToolCallStatus::Pending => theme.text_muted,
            ToolCallStatus::Running => theme.warning,
            ToolCallStatus::Completed => theme.success,
            ToolCallStatus::Failed => theme.error,
        };

        let lines = vec![Line::from(vec![
            Span::styled(status_icon, Style::default().fg(status_color)),
            Span::raw(" "),
            Span::styled("task ", Style::default().fg(theme.primary)),
            Span::styled(
                &self.task_name,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            if let Some(cat) = &self.category {
                Span::styled(format!(" [{}]", cat), Style::default().fg(theme.text_muted))
            } else {
                Span::raw("")
            },
        ])];

        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(status_color)),
        );

        frame.render_widget(paragraph, area);
    }
}

pub struct SkillToolView {
    pub skill_name: String,
}

impl SkillToolView {
    pub fn new(skill_name: String) -> Self {
        Self { skill_name }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let line = Line::from(vec![
            Span::styled("skill ", Style::default().fg(theme.primary)),
            Span::styled(&self.skill_name, Style::default().fg(theme.text)),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, area);
    }
}

pub struct EditToolView {
    pub file_path: String,
    pub diff_content: String,
    pub diagnostics: Vec<String>,
}

impl EditToolView {
    pub fn new(file_path: String, diff_content: String) -> Self {
        Self {
            file_path,
            diff_content,
            diagnostics: Vec::new(),
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let header_line = Line::from(vec![
            Span::styled(
                "edit ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&self.file_path, Style::default().fg(theme.text)),
        ]);

        // Diagnostics lines (if any)
        let mut diag_lines: Vec<Line<'static>> = Vec::new();
        if !self.diagnostics.is_empty() {
            diag_lines.push(Line::from(""));
            diag_lines.push(Line::from(Span::styled(
                "Diagnostics:",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )));
            for diag in &self.diagnostics {
                diag_lines.push(Line::from(Span::styled(
                    format!("  {}", diag),
                    Style::default().fg(theme.warning),
                )));
            }
        }

        let header_height = 1u16;
        let diag_height = diag_lines.len() as u16;
        let non_diff_height = header_height + diag_height;

        if self.diff_content.is_empty() || area.height <= non_diff_height + 2 {
            // Not enough room for split diff — fall back to unified inline
            let mut lines = vec![header_line];
            if !self.diff_content.is_empty() {
                let diff_view = DiffView::new().with_content(&self.diff_content);
                lines.extend(diff_view.to_lines(theme));
            }
            lines.extend(diag_lines);
            let paragraph = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme.info)),
            );
            frame.render_widget(paragraph, area);
            return;
        }

        // Layout: header | diff (split) | diagnostics
        let diff_height = area.height.saturating_sub(non_diff_height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Length(diff_height),
                Constraint::Length(diag_height),
            ])
            .split(area);

        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        let diff_view = DiffView::new()
            .with_content(&self.diff_content)
            .with_mode(DiffMode::Split);
        diff_view.render(frame, chunks[1], theme);

        if !diag_lines.is_empty() {
            frame.render_widget(Paragraph::new(diag_lines), chunks[2]);
        }
    }
}

pub struct ApplyPatchToolView {
    pub files: Vec<String>,
    pub diff_content: String,
}

impl ApplyPatchToolView {
    pub fn new(diff_content: String) -> Self {
        // Extract file paths from diff headers
        let files: Vec<String> = diff_content
            .lines()
            .filter_map(|line| {
                line.strip_prefix("+++ b/")
                    .or_else(|| line.strip_prefix("+++ "))
                    .map(|s| s.to_string())
            })
            .collect();

        Self {
            files,
            diff_content,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let header_line = Line::from(vec![
            Span::styled(
                "apply_patch ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({} files)", self.files.len()),
                Style::default().fg(theme.text_muted),
            ),
        ]);

        // File list lines
        let mut file_lines: Vec<Line<'static>> = Vec::new();
        for file in &self.files {
            file_lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(file.clone(), Style::default().fg(theme.info)),
            ]));
        }

        let header_height = (1 + file_lines.len()) as u16;

        if self.diff_content.is_empty() || area.height <= header_height + 2 {
            let mut lines = vec![header_line];
            lines.extend(file_lines);
            if !self.diff_content.is_empty() {
                lines.push(Line::from(""));
                let diff_view = DiffView::new().with_content(&self.diff_content);
                lines.extend(diff_view.to_lines(theme));
            }
            let paragraph = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(theme.info)),
            );
            frame.render_widget(paragraph, area);
            return;
        }

        let diff_height = area.height.saturating_sub(header_height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Length(diff_height),
            ])
            .split(area);

        let mut header_lines = vec![header_line];
        header_lines.extend(file_lines);
        frame.render_widget(Paragraph::new(header_lines), chunks[0]);

        let diff_view = DiffView::new()
            .with_content(&self.diff_content)
            .with_mode(DiffMode::Split);
        diff_view.render(frame, chunks[1], theme);
    }
}

pub struct TodoWriteToolView {
    pub items: Vec<(String, TodoStatus)>,
}

impl TodoWriteToolView {
    pub fn new(items: Vec<(String, TodoStatus)>) -> Self {
        Self { items }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let mut lines = vec![Line::from(Span::styled(
            "Todo List",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))];

        for (content, status) in &self.items {
            let (icon, color) = match status {
                TodoStatus::Pending => ("○", theme.text_muted),
                TodoStatus::InProgress => (super::spinner::progress_circle_icon(), theme.warning),
                TodoStatus::Completed => ("●", theme.success),
                TodoStatus::Cancelled => ("○", theme.text_muted),
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(content.as_str(), Style::default().fg(theme.text)),
            ]));
        }

        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(theme.primary)),
        );

        frame.render_widget(paragraph, area);
    }
}
