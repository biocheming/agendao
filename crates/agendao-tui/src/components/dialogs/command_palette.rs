use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use reratui::hooks::use_context;
use reratui::Component;

use agendao_command::{CommandRegistry, UiActionId};

use crate::command::fuzzy_match;
use crate::context::MessageDensity;
use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

pub struct VisibilityLabels {
    pub show_thinking: bool,
    pub show_tool_details: bool,
    pub density: MessageDensity,
    pub semantic_highlight: bool,
    pub show_header: bool,
    pub show_scrollbar: bool,
    pub tips_hidden: bool,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub action_id: UiActionId,
    pub title: String,
    pub keybind: Option<String>,
    pub category: String,
}

#[derive(Clone)]
pub struct CommandPalette {
    commands: Vec<Command>,
    filtered: Vec<usize>,
    query: String,
    state: ListState,
    open: bool,
}

impl CommandPalette {
    pub fn new() -> Self {
        let registry = CommandRegistry::new();
        let commands = registry
            .ui_palette_commands()
            .into_iter()
            .map(|spec| Command {
                action_id: spec.action_id,
                title: spec.title.to_string(),
                keybind: spec.keybind.map(str::to_string),
                category: spec.category.label().to_string(),
            })
            .collect::<Vec<_>>();

        let filtered = (0..commands.len()).collect();
        let mut state = ListState::default();
        state.select(Some(0));

        Self {
            commands,
            filtered,
            query: String::new(),
            state,
            open: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.filtered = (0..self.commands.len()).collect();
        self.state.select(Some(0));
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.filter_commands();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.filter_commands();
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
            }
        }
    }

    pub fn selected_command(&self) -> Option<&Command> {
        self.state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .and_then(|&idx| self.commands.get(idx))
    }

    pub fn selected_action(&self) -> Option<UiActionId> {
        self.selected_command().map(|cmd| cmd.action_id)
    }

    pub fn sync_visibility_labels(&mut self, state: VisibilityLabels) {
        let VisibilityLabels {
            show_thinking,
            show_tool_details,
            density,
            semantic_highlight,
            show_header,
            show_scrollbar,
            tips_hidden,
        } = state;

        for command in &mut self.commands {
            if matches!(command.action_id, UiActionId::ToggleHeader) {
                command.title = if show_header {
                    "Hide header".to_string()
                } else {
                    "Show header".to_string()
                };
            }
            if matches!(command.action_id, UiActionId::ToggleScrollbar) {
                command.title = if show_scrollbar {
                    "Hide scrollbar".to_string()
                } else {
                    "Show scrollbar".to_string()
                };
            }
            if matches!(command.action_id, UiActionId::ToggleTips) {
                command.title = if tips_hidden {
                    "Show tips".to_string()
                } else {
                    "Hide tips".to_string()
                };
            }
            if matches!(command.action_id, UiActionId::ToggleThinking) {
                command.title = if show_thinking {
                    "Hide thinking".to_string()
                } else {
                    "Show thinking".to_string()
                };
            }
            if matches!(command.action_id, UiActionId::ToggleToolDetails) {
                command.title = if show_tool_details {
                    "Hide tool details".to_string()
                } else {
                    "Show tool details".to_string()
                };
            }
            if matches!(command.action_id, UiActionId::ToggleDensity) {
                command.title = match density {
                    MessageDensity::Compact => "Switch to cozy density".to_string(),
                    MessageDensity::Cozy => "Switch to compact density".to_string(),
                };
            }
            if matches!(command.action_id, UiActionId::ToggleSemanticHighlight) {
                command.title = if semantic_highlight {
                    "Disable semantic highlight".to_string()
                } else {
                    "Enable semantic highlight".to_string()
                };
            }
        }

        self.filter_commands();
    }

    fn filter_commands(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.commands.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let title_score = fuzzy_match(&self.query, &cmd.title);
                    let cat_score = fuzzy_match(&self.query, &cmd.category);
                    let best = title_score.into_iter().chain(cat_score).max();
                    best.map(|s| (i, s))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_width = 60;
        let dialog_height = (self.filtered.len() + 4).min(20) as u16;

        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Commands ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        let inner_area = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.primary)),
            Span::styled(&self.query, Style::default().fg(theme.text)),
            Span::styled("▏", Style::default().fg(theme.primary)),
        ]);

        let search_paragraph = Paragraph::new(search_line);
        surface.render_widget(
            search_paragraph,
            Rect {
                x: inner_area.x,
                y: inner_area.y,
                width: inner_area.width,
                height: 1,
            },
        );

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .filter_map(|&idx| {
                self.commands.get(idx).map(|cmd| {
                    let style = if Some(idx)
                        == self
                            .state
                            .selected()
                            .and_then(|s| self.filtered.get(s))
                            .copied()
                    {
                        Style::default()
                            .fg(theme.text)
                            .bg(theme.background_element)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    };

                    let mut spans = vec![Span::styled(&cmd.title, style)];

                    if let Some(ref keybind) = cmd.keybind {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            keybind.clone(),
                            Style::default().fg(theme.text_muted),
                        ));
                    }

                    ListItem::new(Line::from(spans))
                })
            })
            .collect();

        let list = List::new(items).style(Style::default().fg(theme.text));

        let list_area = Rect {
            x: inner_area.x,
            y: inner_area.y + 2,
            width: inner_area.width,
            height: inner_area.height.saturating_sub(2),
        };

        surface.render_stateful_widget(list, list_area, &mut self.state.clone());
    }
}

impl Component for CommandPalette {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area, &theme);
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn command_palette_renders_to_buffer_surface() {
        let mut dialog = CommandPalette::new();
        dialog.open();

        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render_surface(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }
}
