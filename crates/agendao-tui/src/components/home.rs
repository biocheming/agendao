use std::sync::Arc;

use agendao_command_render::branding::{logo_height, LOGO};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use reratui::element::Element;
use reratui::fiber_tree::with_current_fiber;
use reratui::hooks::{use_context, use_media_query};
use reratui::{Buffer, Component};
use std::sync::Mutex;

use crate::branding::{APP_SHORT_NAME, APP_VERSION_DATE};
use crate::components::{Logo, Prompt};
use crate::context::{AppContext, McpConnectionStatus};
use crate::ui::RenderSurface;

const HOME_TIPS: &[&str] = &[
    "Press {highlight}Tab{/highlight} to cycle modes",
    "Press {highlight}Shift+Tab{/highlight} to cycle modes backward",
    "Press {highlight}Ctrl+P{/highlight} to open command palette",
    "Type {highlight}/help{/highlight} to browse all commands",
    "Use {highlight}/themes{/highlight} to switch visual themes",
    "Use {highlight}/sessions{/highlight} to resume older work",
    "Use {highlight}Alt+Left{/highlight} to leave an attached session",
    "Use {highlight}/timeline{/highlight} to jump to any message",
    "Use {highlight}/status{/highlight} to inspect runtime status",
    "Use {highlight}/mcps{/highlight} to inspect MCP connections",
    "Use {highlight}/editor{/highlight} to write prompts externally",
    "Use {highlight}/compact{/highlight} when context gets too long",
    "Use {highlight}/new{/highlight} to start a clean session",
    "Use {highlight}/copy{/highlight} to copy current session summary",
    "Use {highlight}/fork{/highlight} to branch from a message",
    "Use {highlight}/rename{/highlight} to rename this session",
    "Use {highlight}/share{/highlight} to generate share links",
    "Use {highlight}/unshare{/highlight} to revoke share links",
    "Use {highlight}/timestamps{/highlight} to show or hide times",
    "Use {highlight}/thinking{/highlight} to toggle reasoning blocks",
    "Use {highlight}/density{/highlight} for cozy or compact layout",
    "Use {highlight}/highlight{/highlight} to toggle semantic styling",
    "Use {highlight}/sidebar{/highlight} to toggle sidebar visibility",
    "Use {highlight}/header{/highlight} to toggle session header",
    "Use {highlight}/scrollbar{/highlight} to toggle scrollbar",
    "Use {highlight}/tips.toggle{/highlight} to hide or show tips",
    "Use {highlight}/stash{/highlight} to save current draft",
    "Use {highlight}/export{/highlight} to export chat transcript",
    "Use {highlight}Esc{/highlight} twice to interrupt running tasks",
    "Use {highlight}Alt+Up{/highlight} and {highlight}Alt+Down{/highlight} for prompt history",
    "Use {highlight}Ctrl+V{/highlight} to paste text or clipboard images",
    "Use {highlight}Ctrl+Shift+C{/highlight} to copy selected text",
    "Use {highlight}@path{/highlight} to reference files in prompt",
    "Use {highlight}/image path/to/file.png{/highlight} to attach a local image",
    "Use {highlight}/connect{/highlight} to add a new provider",
    "Use {highlight}/models{/highlight} to switch active model",
    "Use {highlight}/agents{/highlight} to switch active agent",
    "Use {highlight}/skills{/highlight} to inspect installed skills",
];
const HOME_PROMPT_PLACEHOLDERS: &[&str] = &[
    "Fix a TODO in the codebase",
    "What is the tech stack of this project?",
    "Fix broken tests",
];
const HOME_SHELL_PLACEHOLDERS: &[&str] = &["ls -la", "git status", "pwd"];
const TIP_ROTATE_SECONDS: i64 = 12;
const HOME_MAX_CONTENT_WIDTH: u16 = 75;
const HOME_OUTER_H_PADDING: u16 = 2;
const HOME_OUTER_V_PADDING: u16 = 1;
const HOME_NARROW_WIDTH_THRESHOLD: u16 = 96;
const HOME_SHORT_HEIGHT_THRESHOLD: u16 = 22;

#[derive(Clone)]
pub struct HomeView {
    context: Arc<AppContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HomeViewRenderSnapshot {
    directory: String,
    connected_mcp_count: usize,
    has_mcp_errors: bool,
    show_tips: bool,
}

#[derive(Clone)]
struct HomeViewComponent {
    home: HomeView,
    area: Rect,
    prompt: Prompt,
    snapshot: HomeViewRenderSnapshot,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HomeLayoutProfile {
    is_narrow: bool,
    is_short: bool,
}

impl HomeLayoutProfile {
    fn prompt_height(self) -> u16 {
        if self.is_short { 6 } else { 8 }
    }

    fn tips_height(self, show_tips: bool) -> u16 {
        if !show_tips {
            0
        } else if self.is_narrow || self.is_short {
            3
        } else {
            4
        }
    }

    fn footer_height(self) -> u16 {
        if self.is_short { 2 } else { 3 }
    }

    fn tagline_height(self) -> u16 {
        0
    }
}

impl HomeView {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self { context }
    }

    pub fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect) {
        let prompt = Prompt::new(self.context.clone())
            .with_placeholders(HOME_PROMPT_PLACEHOLDERS, HOME_SHELL_PLACEHOLDERS);
        self.render_with_prompt(surface, area, &prompt);
    }

    pub fn render_reactive(&self, buffer: &mut Buffer, area: Rect, prompt: &Prompt) {
        let snapshot = self.capture_render_snapshot();
        let cursor = Arc::new(Mutex::new(None));
        Element::component(HomeViewComponent {
            home: Self {
                context: self.context.clone(),
            },
            area,
            prompt: prompt.clone(),
            snapshot,
            cursor: cursor.clone(),
        })
        .with_key("home-view")
        .render(area, buffer);
        let _ = cursor;
    }

    pub fn render_reactive_with_cursor(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        prompt: &Prompt,
    ) -> Option<(u16, u16)> {
        let snapshot = self.capture_render_snapshot();
        let cursor = Arc::new(Mutex::new(None));
        Element::component(HomeViewComponent {
            home: Self {
                context: self.context.clone(),
            },
            area,
            prompt: prompt.clone(),
            snapshot,
            cursor: cursor.clone(),
        })
        .with_key("home-view")
        .render(area, buffer);
        let cursor_value = *cursor.lock().expect("home cursor lock");
        cursor_value
    }

    pub fn render_with_prompt<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        prompt: &Prompt,
    ) {
        let theme = self.context.theme.read().clone();
        let snapshot = self.capture_render_snapshot();
        self.render_with_prompt_theme_and_snapshot(
            surface,
            area,
            prompt,
            &theme,
            &snapshot,
            None,
        );
    }

    pub fn render_with_prompt_and_theme<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        prompt: &Prompt,
        theme: &crate::theme::Theme,
    ) {
        let snapshot = self.capture_render_snapshot();
        self.render_with_prompt_theme_and_snapshot(
            surface,
            area,
            prompt,
            theme,
            &snapshot,
            None,
        );
    }

    fn render_with_prompt_theme_and_snapshot<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        prompt: &Prompt,
        theme: &crate::theme::Theme,
        snapshot: &HomeViewRenderSnapshot,
        cursor_sink: Option<&Arc<Mutex<Option<(u16, u16)>>>>,
    ) {
        let area = Rect {
            x: area.x.saturating_add(HOME_OUTER_H_PADDING),
            y: area.y.saturating_add(HOME_OUTER_V_PADDING),
            width: area
                .width
                .saturating_sub(HOME_OUTER_H_PADDING.saturating_mul(2)),
            height: area
                .height
                .saturating_sub(HOME_OUTER_V_PADDING.saturating_mul(2)),
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        let layout_profile = if with_current_fiber(|_| ()).is_some() {
            HomeLayoutProfile {
                is_narrow: use_media_query(|(width, _)| {
                    width > 0 && width < HOME_NARROW_WIDTH_THRESHOLD
                }),
                is_short: use_media_query(|(_, height)| {
                    height > 0 && height < HOME_SHORT_HEIGHT_THRESHOLD
                }),
            }
        } else {
            HomeLayoutProfile {
                is_narrow: area.width < HOME_NARROW_WIDTH_THRESHOLD,
                is_short: area.height < HOME_SHORT_HEIGHT_THRESHOLD,
            }
        };
        let tips_height = layout_profile.tips_height(snapshot.show_tips);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(logo_height() as u16),
                Constraint::Length(layout_profile.tagline_height()),
                Constraint::Length(layout_profile.prompt_height()),
                Constraint::Length(tips_height),
                Constraint::Min(1),
                Constraint::Length(layout_profile.footer_height()),
            ])
            .split(area);

        let logo = Logo::new(
            theme.primary,
            theme.primary,
            theme.secondary,
            theme.text_muted,
            theme.text,
        );
        let logo_width = LOGO[0].len() as u16;
        let logo_left = layout[1].x + (layout[1].width.saturating_sub(logo_width)) / 2;
        let logo_area = Rect {
            x: logo_left,
            y: layout[1].y,
            width: logo_width.min(layout[1].width),
            height: layout[1].height,
        };
        if with_current_fiber(|_| ()).is_some() {
            if let Some(buffer) = surface.buffer_mut_opt() {
                Element::component(logo).render(logo_area, buffer);
            } else {
                logo.render_surface(surface, logo_area);
            }
        } else {
            logo.render_surface(surface, logo_area);
        }

        let prompt_width = layout[3].width.min(HOME_MAX_CONTENT_WIDTH);
        let left_pad = (layout[3].width.saturating_sub(prompt_width)) / 2;
        let prompt_area = Rect {
            x: layout[3].x + left_pad,
            y: layout[3].y,
            width: prompt_width,
            height: layout[3].height,
        };

        if with_current_fiber(|_| ()).is_some() {
            if let Some(buffer) = surface.buffer_mut_opt() {
                if let Some(cursor) = cursor_sink {
                    let _ =
                        prompt.render_reactive_with_cursor(buffer, prompt_area, cursor.clone());
                } else {
                    prompt.render_reactive(buffer, prompt_area);
                }
            } else {
                prompt.render_surface(surface, prompt_area);
            }
        } else {
            prompt.render_surface(surface, prompt_area);
        }

        if snapshot.show_tips && layout[4].height > 0 {
            self.render_tips(surface, layout[4], &theme);
        }

        self.render_footer(surface, layout[6], &theme, snapshot);
    }


    fn render_tips<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &crate::theme::Theme,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let tip_width = area.width.min(HOME_MAX_CONTENT_WIDTH);
        let left_pad = (area.width.saturating_sub(tip_width)) / 2;
        let tip_area = Rect {
            x: area.x + left_pad,
            y: area.y,
            width: tip_width,
            height: area.height,
        };
        if tip_area.height == 0 || tip_area.width == 0 {
            return;
        }

        let top_padding = 3u16.min(tip_area.height.saturating_sub(1));
        let tip_render_area = Rect {
            x: tip_area.x,
            y: tip_area.y.saturating_add(top_padding),
            width: tip_area.width,
            height: tip_area.height.saturating_sub(top_padding).max(1),
        };

        let slot = chrono::Utc::now()
            .timestamp()
            .div_euclid(TIP_ROTATE_SECONDS);
        let tip_idx = slot.rem_euclid(HOME_TIPS.len() as i64) as usize;
        let tip = HOME_TIPS
            .get(tip_idx)
            .copied()
            .unwrap_or("Use /help to open command guide");
        let mut spans = vec![Span::styled("• Tip ", Style::default().fg(theme.warning))];
        spans.extend(parse_tip_highlights(tip, theme));
        let paragraph = Paragraph::new(Line::from(spans));

        surface.render_widget(paragraph, tip_render_area);
    }

    fn render_footer<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &crate::theme::Theme,
        snapshot: &HomeViewRenderSnapshot,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let horizontal_padding = 2u16.min(area.width / 2);
        let vertical_padding = if area.height >= 3 { 1 } else { 0 };
        let content_area = Rect {
            x: area.x.saturating_add(horizontal_padding),
            y: area.y.saturating_add(vertical_padding),
            width: area
                .width
                .saturating_sub(horizontal_padding.saturating_mul(2)),
            height: area
                .height
                .saturating_sub(vertical_padding.saturating_mul(2)),
        };
        if content_area.width == 0 || content_area.height == 0 {
            return;
        }

        let mut spans: Vec<Span> = Vec::new();

        // Left: directory
        let dir_text = snapshot.directory.clone();
        spans.push(Span::styled(
            dir_text.clone(),
            Style::default().fg(theme.text_muted),
        ));

        // Middle: MCP status (only when servers exist)
        let mcp_text = if snapshot.connected_mcp_count > 0 || snapshot.has_mcp_errors {
            let dot_color = if snapshot.has_mcp_errors {
                theme.error
            } else if snapshot.connected_mcp_count > 0 {
                theme.success
            } else {
                theme.text_muted
            };
            let label = format!("{} MCP", snapshot.connected_mcp_count);
            Some((dot_color, label))
        } else {
            None
        };

        // Right: branding + date version
        let version_text = format!("{} {}", APP_SHORT_NAME, APP_VERSION_DATE);

        // Calculate padding
        let left_len = dir_text.len();
        let mid_len = mcp_text.as_ref().map(|(_, l)| l.len() + 4).unwrap_or(0);
        let right_len = version_text.len();
        let total_content = left_len + mid_len + right_len;
        let available = content_area.width as usize;

        if let Some((dot_color, ref label)) = mcp_text {
            let left_padding = if available > total_content {
                (available - total_content) / 2
            } else if available > right_len + left_len + 1 {
                1
            } else {
                0
            };
            spans.push(Span::raw(" ".repeat(left_padding)));
            spans.push(Span::styled(
                "• ".to_string(),
                Style::default().fg(dot_color),
            ));
            spans.push(Span::styled(
                label.clone(),
                Style::default().fg(theme.text_muted),
            ));
        }

        // Right-align version
        let used: usize = spans.iter().map(|s| s.content.len()).sum();
        let right_padding = available.saturating_sub(used + right_len);
        spans.push(Span::raw(" ".repeat(right_padding)));
        spans.push(Span::styled(
            version_text,
            Style::default().fg(theme.text_muted),
        ));

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line);
        surface.render_widget(paragraph, content_area);
    }

    fn capture_render_snapshot(&self) -> HomeViewRenderSnapshot {
        let directory = self.context.directory.read().clone();
        let mcp_servers = self.context.mcp_servers.read();
        let connected_mcp_count = mcp_servers
            .iter()
            .filter(|server| matches!(server.status, McpConnectionStatus::Connected))
            .count();
        let has_mcp_errors = mcp_servers.iter().any(|server| {
            matches!(
                server.status,
                McpConnectionStatus::Failed | McpConnectionStatus::NeedsClientRegistration
            )
        });
        let show_tips = {
            let is_first_time_user = self.context.session.read().sessions.is_empty();
            !is_first_time_user && !self.context.tips_hidden()
        };

        HomeViewRenderSnapshot {
            directory,
            connected_mcp_count,
            has_mcp_errors,
            show_tips,
        }
    }
}

impl Component for HomeViewComponent {
    fn render(&self, _area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<crate::theme::Theme>();
        let mut surface = crate::ui::BufferSurface::new(buffer);
        self.home.render_with_prompt_theme_and_snapshot(
            &mut surface,
            self.area,
            &self.prompt,
            &theme,
            &self.snapshot,
            Some(&self.cursor),
        );
        let mut cursor = self.cursor.lock().expect("home cursor lock");
        if cursor.is_none() {
            *cursor = surface.cursor_position();
        }
    }
}

fn parse_tip_highlights(tip: &str, theme: &crate::theme::Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = tip;
    const START: &str = "{highlight}";
    const END: &str = "{/highlight}";

    loop {
        let Some(start_idx) = remaining.find(START) else {
            if !remaining.is_empty() {
                spans.push(Span::styled(
                    remaining.to_string(),
                    Style::default().fg(theme.text_muted),
                ));
            }
            break;
        };

        let (plain, after_start_marker) = remaining.split_at(start_idx);
        if !plain.is_empty() {
            spans.push(Span::styled(
                plain.to_string(),
                Style::default().fg(theme.text_muted),
            ));
        }

        let highlighted_tail = &after_start_marker[START.len()..];
        let Some(end_idx) = highlighted_tail.find(END) else {
            spans.push(Span::styled(
                START.to_string(),
                Style::default().fg(theme.text_muted),
            ));
            if !highlighted_tail.is_empty() {
                spans.push(Span::styled(
                    highlighted_tail.to_string(),
                    Style::default().fg(theme.text_muted),
                ));
            }
            break;
        };

        let (highlighted, after_end_marker) = highlighted_tail.split_at(end_idx);
        if !highlighted.is_empty() {
            spans.push(Span::styled(
                highlighted.to_string(),
                Style::default().fg(theme.text),
            ));
        }
        remaining = &after_end_marker[END.len()..];
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn home_view_renders_to_buffer_surface() {
        let context = Arc::new(AppContext::new());
        let home = HomeView::new(context.clone());
        let prompt = Prompt::new(context)
            .with_placeholders(HOME_PROMPT_PLACEHOLDERS, HOME_SHELL_PLACEHOLDERS);
        let area = Rect::new(0, 0, 80, 24);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        home.render_with_prompt(&mut surface, area, &prompt);

        assert!(surface.cursor_position().is_some());
        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }

    #[test]
    fn home_view_renders_in_short_narrow_area() {
        let context = Arc::new(AppContext::new());
        {
            let mut session = context.session.write();
            let session_id = session.create_session(Some("Existing".to_string()));
            session.set_current_session_id(session_id);
        }
        let home = HomeView::new(context.clone());
        let prompt = Prompt::new(context)
            .with_placeholders(HOME_PROMPT_PLACEHOLDERS, HOME_SHELL_PLACEHOLDERS);
        let area = Rect::new(0, 0, 68, 16);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        home.render_with_prompt(&mut surface, area, &prompt);

        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains(APP_SHORT_NAME));
    }
}
