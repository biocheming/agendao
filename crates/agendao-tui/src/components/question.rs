use std::cell::{Cell, RefCell};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use reratui::hooks::use_context;
use reratui::Component;

use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

pub const OTHER_OPTION_ID: &str = "__other__";
pub const OTHER_OPTION_LABEL: &str = "Other (type your own)";

#[derive(Clone, Debug, PartialEq)]
pub enum QuestionType {
    Text,
    MultipleChoice,
    SingleChoice,
}

#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct QuestionRequest {
    pub id: String,
    pub question: String,
    pub question_type: QuestionType,
    pub options: Vec<QuestionOption>,
}

#[derive(Clone)]
pub struct QuestionPrompt {
    current_question: Option<QuestionRequest>,
    pub is_open: bool,
    selected_index: usize,
    selected_options: Vec<bool>,
    text_input: String,
    last_rendered_area: Cell<Option<Rect>>,
    click_targets: RefCell<Vec<QuestionClickTarget>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuestionClickTarget {
    start_row: u16,
    end_row: u16,
    option_index: usize,
}

impl QuestionPrompt {
    pub fn new() -> Self {
        Self {
            current_question: None,
            is_open: false,
            selected_index: 0,
            selected_options: Vec::new(),
            text_input: String::new(),
            last_rendered_area: Cell::new(None),
            click_targets: RefCell::new(Vec::new()),
        }
    }

    pub fn ask(&mut self, question: QuestionRequest) {
        let option_count = question.options.len();
        self.current_question = Some(question);
        self.is_open = true;
        self.selected_index = 0;
        self.selected_options = vec![false; option_count];
        self.text_input.clear();
    }

    pub fn current(&self) -> Option<&QuestionRequest> {
        self.current_question.as_ref()
    }

    pub fn close(&mut self) {
        self.current_question = None;
        self.is_open = false;
        self.selected_index = 0;
        self.selected_options.clear();
        self.text_input.clear();
        self.click_targets.borrow_mut().clear();
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if let Some(q) = &self.current_question {
            if !q.options.is_empty() && self.selected_index + 1 < q.options.len() {
                self.selected_index += 1;
            }
        }
    }

    pub fn toggle_selected(&mut self) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::MultipleChoice
                && self.selected_index < self.selected_options.len()
            {
                self.selected_options[self.selected_index] =
                    !self.selected_options[self.selected_index];
            } else if q.question_type == QuestionType::SingleChoice {
                for opt in self.selected_options.iter_mut() {
                    *opt = false;
                }
                if self.selected_index < self.selected_options.len() {
                    self.selected_options[self.selected_index] = true;
                }
            }
        }
    }

    pub fn type_char(&mut self, c: char) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::Text || self.custom_input_active() {
                self.text_input.push(c);
            } else if !q.options.is_empty() {
                let idx = (c as u8).wrapping_sub(b'a') as usize;
                if idx < q.options.len() {
                    if q.question_type == QuestionType::SingleChoice {
                        for opt in self.selected_options.iter_mut() {
                            *opt = false;
                        }
                    }
                    if idx < self.selected_options.len() {
                        self.selected_options[idx] = !self.selected_options[idx];
                    }
                    self.selected_index = idx;
                }
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::Text || self.custom_input_active() {
                self.text_input.pop();
            }
        }
    }

    pub fn handle_space(&mut self) {
        if let Some(q) = &self.current_question {
            if q.question_type == QuestionType::Text || self.custom_input_active() {
                self.text_input.push(' ');
            } else {
                self.toggle_selected();
            }
        }
    }

    pub fn confirm(&mut self) -> Option<(QuestionRequest, Vec<String>)> {
        let q = self.current_question.as_ref()?;
        let answers = if q.question_type == QuestionType::Text {
            vec![self.text_input.trim().to_string()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        } else {
            let mut answers = q
                .options
                .iter()
                .enumerate()
                .filter(|(i, _)| self.selected_options.get(*i).copied().unwrap_or(false))
                .filter(|(_, opt)| !opt.id.eq_ignore_ascii_case(OTHER_OPTION_ID))
                .map(|(_, opt)| opt.id.clone())
                .collect::<Vec<_>>();
            let custom = self.text_input.trim();
            if self.custom_input_active() {
                if custom.is_empty() && answers.is_empty() {
                    return None;
                }
                if !custom.is_empty() {
                    answers.push(custom.to_string());
                }
            }
            answers
        };
        let request = self.current_question.take()?;
        self.is_open = false;
        self.selected_index = 0;
        self.selected_options.clear();
        self.text_input.clear();
        self.click_targets.borrow_mut().clear();
        Some((request, answers))
    }

    pub fn handle_click(&mut self, col: u16, row: u16) {
        if !self.is_open {
            return;
        }
        if let Some(area) = self.last_rendered_area.get() {
            if row < area.y
                || row >= area.y + area.height
                || col < area.x
                || col >= area.x + area.width
            {
                return;
            }
            let target = {
                let click_targets = self.click_targets.borrow();
                click_targets
                    .iter()
                    .find(|target| row >= target.start_row && row <= target.end_row)
                    .copied()
            };
            if let Some(target) = target {
                if self.selected_index != target.option_index
                    || !self
                        .selected_options
                        .get(target.option_index)
                        .copied()
                        .unwrap_or(false)
                {
                    self.selected_index = target.option_index;
                    self.toggle_selected();
                } else if self
                    .current_question
                    .as_ref()
                    .is_some_and(|question| question.question_type == QuestionType::MultipleChoice)
                {
                    self.selected_index = target.option_index;
                    self.toggle_selected();
                }
            }
        }
    }

    fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.is_open {
            return;
        }

        let question = match &self.current_question {
            Some(q) => q,
            None => return,
        };

        let mut content = vec![
            Line::from(Span::styled(
                "Question:",
                Style::default().fg(theme.primary).bold(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                &question.question,
                Style::default().fg(theme.text),
            )),
            Line::from(""),
        ];

        let mut option_line_ranges = Vec::new();

        if !question.options.is_empty() {
            for (i, opt) in question.options.iter().enumerate() {
                let option_start = content.len();
                let is_selected = self.selected_options.get(i).copied().unwrap_or(false);
                let is_highlighted = i == self.selected_index;
                let marker = if is_selected { "[x]" } else { "[ ]" };
                let key = (b'a' + i as u8) as char;
                let label_style = if is_highlighted {
                    Style::default().fg(theme.primary).bold()
                } else {
                    Style::default().fg(theme.text)
                };
                content.push(Line::from(vec![
                    Span::styled(
                        format!("{} ({}) ", marker, key),
                        Style::default().fg(theme.primary),
                    ),
                    Span::styled(&opt.label, label_style),
                ]));
                if let Some(desc) = &opt.description {
                    if !desc.is_empty() {
                        content.push(Line::from(vec![
                            Span::styled("        ", Style::default()),
                            Span::styled(desc.clone(), Style::default().fg(theme.text_muted)),
                        ]));
                    }
                }
                let option_end = content.len().saturating_sub(1);
                option_line_ranges.push((option_start, option_end, i));
            }
            if self.custom_input_active() {
                content.push(Line::from(""));
                content.push(Line::from(Span::styled(
                    "Custom answer:",
                    Style::default().fg(theme.primary).bold(),
                )));
                let input_display = if self.text_input.is_empty() {
                    "Type your custom answer...".to_string()
                } else {
                    format!("> {}", self.text_input)
                };
                let input_style = if self.text_input.is_empty() {
                    Style::default().fg(theme.text_muted)
                } else {
                    Style::default().fg(theme.text)
                };
                content.push(Line::from(Span::styled(input_display, input_style)));
            }
            content.push(Line::from(""));
            content.push(Line::from(Span::styled(
                if self.custom_input_active() {
                    "Up/Down or Tab/Shift+Tab to navigate, Space to toggle, type custom text, Enter to confirm"
                } else {
                    "Up/Down or Tab/Shift+Tab to navigate, Space to toggle, Enter to confirm"
                },
                Style::default().fg(theme.text_muted),
            )));
        } else {
            let input_display = if self.text_input.is_empty() {
                "Type your answer...".to_string()
            } else {
                format!("> {}", self.text_input)
            };
            let input_style = if self.text_input.is_empty() {
                Style::default().fg(theme.text_muted)
            } else {
                Style::default().fg(theme.text)
            };
            content.push(Line::from(Span::styled(input_display, input_style)));
            content.push(Line::from(""));
            content.push(Line::from(Span::styled(
                "Type your answer and press Enter",
                Style::default().fg(theme.text_muted),
            )));
        }

        let height = (content.len() as u16 + 2).min(area.height.saturating_sub(2));
        let width = area.width.saturating_sub(2).min(80);
        let popup_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(height + 1),
            width,
            height,
        );

        self.last_rendered_area.set(Some(popup_area));
        let content_origin_y = popup_area.y.saturating_add(1);
        let click_targets = option_line_ranges
            .into_iter()
            .map(|(start, end, option_index)| QuestionClickTarget {
                start_row: content_origin_y.saturating_add(start as u16),
                end_row: content_origin_y.saturating_add(end as u16),
                option_index,
            })
            .collect::<Vec<_>>();
        self.click_targets.replace(click_targets);

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Question ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.primary)),
            )
            .style(Style::default().bg(theme.background_panel));

        surface.render_widget(Clear, popup_area);
        surface.render_widget(paragraph, popup_area);
    }

    fn custom_input_active(&self) -> bool {
        let Some(question) = &self.current_question else {
            return false;
        };
        question.options.iter().enumerate().any(|(idx, opt)| {
            opt.id.eq_ignore_ascii_case(OTHER_OPTION_ID)
                && self.selected_options.get(idx).copied().unwrap_or(false)
        })
    }
}

impl Component for QuestionPrompt {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let mut surface = BufferSurface::new(buffer);
        self.render_surface(&mut surface, area, &theme);
    }
}

impl Default for QuestionPrompt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};

    use crate::{theme::Theme, ui::BufferSurface};

    fn choice_prompt() -> QuestionPrompt {
        let mut prompt = QuestionPrompt::new();
        prompt.ask(QuestionRequest {
            id: "q1".to_string(),
            question: "Which path?".to_string(),
            question_type: QuestionType::SingleChoice,
            options: vec![
                QuestionOption {
                    id: "fast".to_string(),
                    label: "Fast".to_string(),
                    description: None,
                },
                QuestionOption {
                    id: OTHER_OPTION_ID.to_string(),
                    label: OTHER_OPTION_LABEL.to_string(),
                    description: None,
                },
            ],
        });
        prompt
    }

    #[test]
    fn confirm_uses_custom_text_instead_of_other_literal() {
        let mut prompt = choice_prompt();
        prompt.move_down();
        prompt.toggle_selected();
        for ch in "Need a hybrid mode".chars() {
            prompt.type_char(ch);
        }

        let (_request, answers) = prompt.confirm().expect("should confirm");
        assert_eq!(answers, vec!["Need a hybrid mode".to_string()]);
    }

    #[test]
    fn confirm_requires_custom_text_when_only_other_selected() {
        let mut prompt = choice_prompt();
        prompt.move_down();
        prompt.toggle_selected();

        assert!(prompt.confirm().is_none());
    }

    #[test]
    fn confirm_preserves_standard_selection_without_other_literal() {
        let mut prompt = choice_prompt();
        prompt.toggle_selected();

        let (_request, answers) = prompt.confirm().expect("should confirm");
        assert_eq!(answers, vec!["fast".to_string()]);
    }

    #[test]
    fn question_prompt_renders_to_buffer_surface() {
        let prompt = choice_prompt();
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        prompt.render_surface(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }

    #[test]
    fn clicking_option_row_selects_it() {
        let mut prompt = choice_prompt();
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        prompt.render_surface(&mut surface, area, &Theme::dark());
        let target = prompt.click_targets.borrow()[1];
        prompt.handle_click(5, target.start_row);

        assert_eq!(prompt.selected_index, 1);
        assert!(prompt.selected_options[1]);
    }

    #[test]
    fn handle_space_types_when_custom_input_is_active() {
        let mut prompt = choice_prompt();
        prompt.move_down();
        prompt.toggle_selected();
        prompt.type_char('n');
        prompt.handle_space();
        prompt.type_char('o');

        assert_eq!(prompt.text_input, "n o");
    }
}
