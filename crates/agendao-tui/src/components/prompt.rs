use agendao_command_render::run_status_labels::canonical_run_status_title;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use reratui::element::Element;
use reratui::hooks::{stop_propagation, use_context, use_event, use_keyboard_press};
use reratui::Component;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthChar;

use crate::context::{AppContext, MessageRole, SessionStatus};
use crate::file_index::FileIndex;
use crate::theme::Theme;
use crate::ui::{BufferSurface, RenderSurface};

use super::spinner::{KnightRiderSpinner, SpinnerMode, TaskKind};

const MAX_HISTORY_ENTRIES: usize = 200;
const MAX_STASH_ENTRIES: usize = 50;
const MAX_FRECENCY_ENTRIES: usize = 1000;
const PROMPT_MIN_INPUT_LINES: u16 = 2;
const PROMPT_MAX_INPUT_LINES: u16 = 6;
const SHELL_PLACEHOLDER: &str = "Run a command... \"ls -la\"";
const INTERRUPT_CONFIRM_WINDOW_SECS: u64 = 5;
const PLACEHOLDER_ROTATE_SECS: u64 = 12;
const FILE_INDEX_MAX_DEPTH: usize = 8;
const FILE_SUGGESTION_LIMIT: usize = 20;
const PROMPT_BLOCK_PAD_LEFT: u16 = 1;
const PROMPT_BLOCK_PAD_RIGHT: u16 = 1;
const PROMPT_BLOCK_PAD_TOP: u16 = 0;
const PROMPT_BLOCK_PAD_BOTTOM: u16 = 1;
const PROMPT_LINE_H_INSET: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Normal,
    Shell,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptStashEntry {
    pub input: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct FrecencyEntry {
    frequency: u64,
    last_used: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct HistoryStore {
    entries: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct FrecencyStore {
    entries: HashMap<String, FrecencyEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct StashStore {
    entries: Vec<PromptStashEntry>,
}

#[derive(Clone)]
pub struct Prompt {
    context: Arc<AppContext>,
    input: String,
    cursor_position: usize,
    focused: bool,
    placeholder: String,
    placeholder_variants: Vec<String>,
    shell_placeholders: Vec<String>,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<String>,
    frecency: HashMap<String, FrecencyEntry>,
    stash: Vec<PromptStashEntry>,
    suggestions: Vec<String>,
    suggestion_index: Option<usize>,
    known_commands: Vec<String>,
    known_agents: Vec<String>,
    known_skills: Vec<String>,
    history_path: PathBuf,
    frecency_path: PathBuf,
    stash_path: PathBuf,
    file_index: FileIndex,
    spinner: KnightRiderSpinner,
    mode: PromptMode,
    interrupt_press_count: u8,
    last_interrupt_time: Option<Instant>,
    attachment_status_hint: Option<String>,
    return_flow_text: Option<String>,
}

#[derive(Clone)]
struct PromptComponent {
    prompt: Prompt,
    cursor_sink: Option<Arc<Mutex<Option<(u16, u16)>>>>,
}

impl Prompt {
    pub fn new(context: Arc<AppContext>) -> Self {
        let state_dir = prompt_state_dir();
        let history_path = state_dir.join("prompt-history.json");
        let frecency_path = state_dir.join("prompt-frecency.json");
        let stash_path = state_dir.join("prompt-stash.json");

        let history = load_history(&history_path);
        let frecency = load_frecency(&frecency_path);
        let stash = load_stash(&stash_path);

        let spinner_color = {
            let theme = context.theme.read();
            let mode_name = current_mode_name(&context);
            prompt_agent_color(&theme, mode_name.as_deref().unwrap_or(""))
        };

        let mut spinner = KnightRiderSpinner::with_color(spinner_color);
        spinner.set_mode(spinner_mode_from_env());

        let mut prompt = Self {
            context,
            input: String::new(),
            cursor_position: 0,
            focused: true,
            placeholder: "Ask anything...".to_string(),
            placeholder_variants: Vec::new(),
            shell_placeholders: Vec::new(),
            history,
            history_index: None,
            history_draft: None,
            frecency,
            stash,
            suggestions: Vec::new(),
            suggestion_index: None,
            known_commands: vec![
                "/help".to_string(),
                "/model".to_string(),
                "/agent".to_string(),
                "/status".to_string(),
                "/session".to_string(),
                "/sessions".to_string(),
                "/parent".to_string(),
                "/back".to_string(),
                "/mcp".to_string(),
                "/mcps".to_string(),
                "/skill".to_string(),
                "/export".to_string(),
                "/stash".to_string(),
                "/new".to_string(),
                "/clear".to_string(),
                "/share".to_string(),
                "/unshare".to_string(),
                "/rename".to_string(),
                "/fork".to_string(),
                "/compact".to_string(),
                "/timeline".to_string(),
                "/undo".to_string(),
                "/redo".to_string(),
                "/copy".to_string(),
                "/themes".to_string(),
                "/timestamps".to_string(),
                "/tips.toggle".to_string(),
                "/tips".to_string(),
                "/thinking".to_string(),
                "/density".to_string(),
                "/highlight".to_string(),
                "/sidebar".to_string(),
                "/command".to_string(),
                "/start-work".to_string(),
                "/connect".to_string(),
                "/image".to_string(),
                "/editor".to_string(),
                "/exit".to_string(),
                "/quit".to_string(),
            ],
            known_agents: vec![
                "build".to_string(),
                "plan".to_string(),
                "general".to_string(),
                "explore".to_string(),
                "compaction".to_string(),
                "title".to_string(),
            ],
            known_skills: Vec::new(),
            history_path,
            frecency_path,
            stash_path,
            file_index: FileIndex::default(),
            spinner,
            mode: PromptMode::Normal,
            interrupt_press_count: 0,
            last_interrupt_time: None,
            attachment_status_hint: None,
            return_flow_text: None,
        };
        prompt.recompute_suggestions();
        prompt
    }

    pub fn with_placeholder(mut self, placeholder: &str) -> Self {
        self.placeholder = placeholder.to_string();
        self.placeholder_variants.clear();
        self.shell_placeholders.clear();
        self
    }

    pub fn with_placeholders(
        mut self,
        placeholders: impl IntoIterator<Item = impl AsRef<str>>,
        shell_placeholders: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        self.placeholder_variants = placeholders
            .into_iter()
            .map(|value| value.as_ref().to_string())
            .filter(|value: &String| !value.is_empty())
            .collect();
        self.shell_placeholders = shell_placeholders
            .into_iter()
            .map(|value| value.as_ref().to_string())
            .filter(|value: &String| !value.is_empty())
            .collect();
        if let Some(first) = self.placeholder_variants.first() {
            self.placeholder = first.clone();
        }
        self
    }

    pub fn set_agent_suggestions(&mut self, agents: Vec<String>) {
        if agents.is_empty() {
            return;
        }
        self.known_agents = dedup_sort(agents);
        self.recompute_suggestions();
    }

    pub fn set_skill_suggestions(&mut self, skills: Vec<String>) {
        self.known_skills = dedup_sort(skills);
        self.recompute_suggestions();
    }

    pub fn render_surface<S: RenderSurface>(&self, surface: &mut S, area: Rect) {
        let theme = self.context.theme.read().clone();
        let animations_enabled = *self.context.animations_enabled.read();
        self.render_with_theme_and_animations(
            surface,
            area,
            &theme,
            animations_enabled,
            None,
        );
    }

    pub fn render_with_theme<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        let animations_enabled = *self.context.animations_enabled.read();
        self.render_with_theme_and_animations(
            surface,
            area,
            theme,
            animations_enabled,
            None,
        );
    }

    pub fn render_reactive(&self, buffer: &mut Buffer, area: Rect) {
        Element::component(PromptComponent {
            prompt: self.clone(),
            cursor_sink: None,
        })
        .with_key("prompt")
        .render(area, buffer);
    }

    pub fn render_reactive_with_cursor(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        cursor_sink: Arc<Mutex<Option<(u16, u16)>>>,
    ) -> Option<(u16, u16)> {
        {
            let mut cursor = cursor_sink.lock().expect("prompt cursor lock");
            *cursor = None;
        }
        Element::component(PromptComponent {
            prompt: self.clone(),
            cursor_sink: Some(cursor_sink.clone()),
        })
        .with_key("prompt")
        .render(area, buffer);
        *cursor_sink.lock().expect("prompt cursor lock")
    }

    fn render_with_theme_and_animations<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        animations_enabled: bool,
        cursor_sink: Option<&Arc<Mutex<Option<(u16, u16)>>>>,
    ) {
        let mode_name = current_mode_name(&self.context);
        let selection = self.context.selection_state();
        let variant = self.context.current_model_variant();
        let show_activity_row = self.shows_activity_row();
        let inline_token_line = (!show_activity_row)
            .then(|| self.render_token_line(theme))
            .filter(|line| !line.spans.is_empty());

        let highlight_color = prompt_agent_color(theme, mode_name.as_deref().unwrap_or(""));
        let active_color = if matches!(self.mode, PromptMode::Shell) {
            theme.primary
        } else {
            highlight_color
        };
        let card_border_color = if self.focused {
            if matches!(self.mode, PromptMode::Shell) {
                theme.primary
            } else {
                theme.border_active
            }
        } else {
            theme.border_subtle
        };
        let card_background = theme.background_panel;
        let content_background = theme.background_element;
        let meta_background = card_background;
        let placeholder = self.active_placeholder();
        let inner_area = Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner_area.width == 0 || inner_area.height == 0 {
            return;
        }

        surface.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(card_border_color))
                .style(Style::default().bg(card_background)),
            area,
        );

        let max_content_lines = inner_area
            .height
            .saturating_sub(if show_activity_row { 3 } else { 2 })
            .saturating_sub(if self.return_flow_text.is_some() {
                1
            } else {
                0
            })
            .saturating_sub(PROMPT_BLOCK_PAD_TOP)
            .saturating_sub(PROMPT_BLOCK_PAD_BOTTOM)
            .max(PROMPT_MIN_INPUT_LINES);
        let content_lines = self.input_display_lines(area.width).min(max_content_lines);
        let input_lines = content_lines
            .saturating_add(PROMPT_BLOCK_PAD_TOP)
            .saturating_add(PROMPT_BLOCK_PAD_BOTTOM);
        let has_return_flow = self.return_flow_text.is_some();
        let chunk_constraints = if show_activity_row {
            let mut constraints = vec![Constraint::Length(input_lines)];
            if has_return_flow {
                constraints.push(Constraint::Length(1));
            }
            constraints.extend([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]);
            constraints
        } else {
            let mut constraints = vec![Constraint::Length(input_lines)];
            if has_return_flow {
                constraints.push(Constraint::Length(1));
            }
            constraints.extend([Constraint::Length(1), Constraint::Length(1)]);
            constraints
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(chunk_constraints)
            .split(inner_area);

        let paragraph = if self.input.is_empty() {
            let display_text = if self.should_show_placeholder() {
                placeholder.to_string()
            } else {
                String::new()
            };
            Paragraph::new(Line::from(Span::styled(
                display_text,
                Style::default().fg(if self.focused {
                    theme.border_subtle
                } else {
                    theme.text_muted
                }),
            )))
            .block(
                Block::default()
                    .padding(Padding::new(
                        PROMPT_BLOCK_PAD_LEFT,
                        PROMPT_BLOCK_PAD_RIGHT,
                        PROMPT_BLOCK_PAD_TOP,
                        PROMPT_BLOCK_PAD_BOTTOM,
                    ))
                    .style(Style::default().bg(content_background)),
            )
            .style(Style::default().fg(if self.focused {
                theme.text
            } else {
                theme.text_muted
            }))
        } else {
            Paragraph::new(self.input.clone())
                .block(
                    Block::default()
                        .padding(Padding::new(
                            PROMPT_BLOCK_PAD_LEFT,
                            PROMPT_BLOCK_PAD_RIGHT,
                            PROMPT_BLOCK_PAD_TOP,
                            PROMPT_BLOCK_PAD_BOTTOM,
                        ))
                        .style(Style::default().bg(content_background)),
                )
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(if self.focused {
                    theme.text
                } else {
                    theme.text_muted
                }))
        };

        surface.render_widget(paragraph, chunks[0]);
        if self.focused {
            let content_origin_x = chunks[0].x.saturating_add(PROMPT_BLOCK_PAD_LEFT);
            let content_origin_y = chunks[0].y.saturating_add(PROMPT_BLOCK_PAD_TOP);
            let input_width = usize::from(
                chunks[0]
                    .width
                    .saturating_sub(PROMPT_BLOCK_PAD_LEFT)
                    .saturating_sub(PROMPT_BLOCK_PAD_RIGHT),
            )
            .max(1);
            let visible_rows = usize::from(content_lines.max(1));
            let (cursor_row, cursor_col) =
                self.cursor_visual_offset(input_width, visible_rows.saturating_sub(1));
            let cursor_x = content_origin_x
                .saturating_add(cursor_col as u16)
                .min(chunks[0].right().saturating_sub(1));
            let cursor_y = content_origin_y
                .saturating_add(cursor_row as u16)
                .min(chunks[0].bottom().saturating_sub(1));
            surface.set_cursor_position(cursor_x, cursor_y);
            if let Some(cursor_sink) = cursor_sink {
                let mut cursor = cursor_sink.lock().expect("prompt cursor lock");
                *cursor = Some((cursor_x, cursor_y));
            }
        }

        let mut chunk_index = 1usize;

        let mut info_parts =
            render_prompt_identity_spans(&self.context, &theme, self.mode, active_color);
        info_parts.push(Span::raw("  "));

        if let Some(model_label) = prompt_model_summary(
            selection.current_model.as_deref(),
            selection.current_provider.as_deref(),
        ) {
            info_parts.push(Span::styled(
                model_label,
                Style::default().fg(theme.text_muted),
            ));
            if let Some(ref v) = variant {
                info_parts.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
                info_parts.push(Span::styled(
                    v.clone(),
                    Style::default().fg(theme.text_muted),
                ));
            }
        }
        if let Some(token_line) = inline_token_line {
            info_parts.push(Span::styled("  ", Style::default().bg(meta_background)));
            info_parts.extend(token_line.spans);
        }

        render_prompt_continuation_row(surface, chunks[chunk_index], meta_background);
        let info_row = row_content_area(chunks[chunk_index], PROMPT_LINE_H_INSET);
        let info_line = Line::from(info_parts);
        let info_paragraph = Paragraph::new(info_line).style(Style::default().bg(meta_background));
        surface.render_widget(info_paragraph, info_row);
        chunk_index += 1;

        if let Some(ref return_flow) = self.return_flow_text {
            let flow_line = Paragraph::new(Line::from(Span::styled(
                return_flow.clone(),
                Style::default().fg(theme.text_muted),
            )))
            .style(Style::default().bg(meta_background));
            let flow_row = row_content_area(chunks[chunk_index], PROMPT_LINE_H_INSET);
            surface.render_widget(flow_line, flow_row);
            chunk_index += 1;
        }

        if show_activity_row {
            let spinner_row = chunks[chunk_index];
            surface.render_widget(
                Paragraph::new("").style(Style::default().bg(meta_background)),
                spinner_row,
            );
            let spinner_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(spinner_row);

            let spinner_icon = self.spinner.frame_symbol(animations_enabled);
            let spinner = Paragraph::new(Line::from(vec![Span::styled(
                spinner_icon,
                Style::default().fg(active_color),
            )]))
            .style(Style::default().bg(meta_background));
            surface.render_widget(spinner, spinner_chunks[0]);
            let token_line = Paragraph::new(self.render_token_line(theme))
                .style(Style::default().bg(meta_background));
            surface.render_widget(token_line, spinner_chunks[1]);

            let status_line = Paragraph::new(self.render_status_line(theme))
                .style(Style::default().bg(meta_background));
            surface.render_widget(status_line, chunks[chunk_index + 1]);
        } else {
            let status_line = Paragraph::new(self.render_status_line(theme))
                .style(Style::default().bg(meta_background));
            surface.render_widget(status_line, chunks[chunk_index]);
        }
    }

    pub fn tick_spinner(&mut self, delta_ms: u64) -> bool {
        let spinner_changed = self.spinner.advance(delta_ms);
        let interrupt_changed = self.maybe_reset_interrupt_confirmation();
        spinner_changed || interrupt_changed
    }

    pub fn next_tick_after(&self, now: Instant, last_tick_at: Instant) -> Option<Duration> {
        let animations_enabled = *self.context.animations_enabled.read();
        let spinner_deadline = if animations_enabled {
            self.spinner
                .next_tick_after()
                .map(|delta| (last_tick_at + delta).saturating_duration_since(now))
        } else {
            None
        };
        let interrupt_deadline = if self.interrupt_press_count == 0 {
            None
        } else {
            self.last_interrupt_time.map(|started_at| {
                let expiry = started_at + Duration::from_secs(INTERRUPT_CONFIRM_WINDOW_SECS);
                expiry.saturating_duration_since(now)
            })
        };

        match (spinner_deadline, interrupt_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        }
    }

    pub fn set_spinner_active(&mut self, active: bool) {
        self.spinner.set_active(active);
        if !active {
            self.reset_interrupt_confirmation();
        }
    }

    pub fn spinner_active(&self) -> bool {
        self.spinner.is_active()
    }

    pub fn set_spinner_task_kind(&mut self, task_kind: TaskKind) {
        self.spinner.set_task_kind(task_kind);
    }

    pub fn spinner_task_kind(&self) -> TaskKind {
        self.spinner.task_kind()
    }

    pub fn set_spinner_color(&mut self, color: Color) {
        self.spinner.set_color(color);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        match key.code {
            KeyCode::Char(c) => {
                if c == '!'
                    && key.modifiers.is_empty()
                    && matches!(self.mode, PromptMode::Normal)
                    && self.cursor_position == 0
                    && self.input.is_empty()
                {
                    self.mode = PromptMode::Shell;
                    return false;
                }
                self.input.insert(self.cursor_position, c);
                self.cursor_position += c.len_utf8();
                self.reset_history_cursor();
                self.recompute_suggestions();
            }
            KeyCode::Backspace => {
                if matches!(self.mode, PromptMode::Shell) && self.cursor_position == 0 {
                    self.mode = PromptMode::Normal;
                    return false;
                }
                if let Some(prev) = prev_char_boundary(&self.input, self.cursor_position) {
                    self.input.replace_range(prev..self.cursor_position, "");
                    self.cursor_position = prev;
                    self.reset_history_cursor();
                    self.recompute_suggestions();
                }
            }
            KeyCode::Delete => {
                if let Some(next) = next_char_boundary(&self.input, self.cursor_position) {
                    self.input.replace_range(self.cursor_position..next, "");
                    self.reset_history_cursor();
                    self.recompute_suggestions();
                }
            }
            KeyCode::Left => {
                if let Some(prev) = prev_char_boundary(&self.input, self.cursor_position) {
                    self.cursor_position = prev;
                }
            }
            KeyCode::Right => {
                if let Some(next) = next_char_boundary(&self.input, self.cursor_position) {
                    self.cursor_position = next;
                }
            }
            KeyCode::Home => {
                self.cursor_position = 0;
            }
            KeyCode::End => {
                self.cursor_position = self.input.len();
            }
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.apply_autocomplete_previous();
                } else {
                    self.apply_autocomplete_next();
                }
            }
            KeyCode::BackTab => {
                self.apply_autocomplete_previous();
            }
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    return true;
                }
            }
            KeyCode::Esc => {
                if matches!(self.mode, PromptMode::Shell) {
                    self.mode = PromptMode::Normal;
                }
            }
            KeyCode::Up => {
                self.history_previous();
            }
            KeyCode::Down => {
                self.history_next();
            }
            _ => {}
        }

        false
    }

    pub fn history_previous_entry(&mut self) {
        self.history_previous();
    }

    pub fn history_next_entry(&mut self) {
        self.history_next();
    }

    pub fn autocomplete_next(&mut self) {
        self.apply_autocomplete_next();
    }

    pub fn autocomplete_previous(&mut self) {
        self.apply_autocomplete_previous();
    }

    pub fn get_input(&self) -> &str {
        &self.input
    }

    pub fn cursor_position(&self) -> usize {
        self.cursor_position
    }

    pub fn take_input(&mut self) -> String {
        let input = std::mem::take(&mut self.input);
        let trimmed = input.trim();
        if !trimmed.is_empty() {
            self.push_history(input.clone());
            self.bump_frecency(trimmed);
            if let Some(first) = trimmed.split_whitespace().next() {
                if first.starts_with('/') || first.starts_with('@') {
                    self.bump_frecency(first);
                }
            }
            store_history(&self.history_path, &self.history);
            store_frecency(&self.frecency_path, &self.frecency);
        }
        self.cursor_position = 0;
        self.history_index = None;
        self.history_draft = None;
        self.suggestions.clear();
        self.suggestion_index = None;
        self.mode = PromptMode::Normal;
        self.reset_interrupt_confirmation();
        input
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor_position = 0;
        self.history_index = None;
        self.history_draft = None;
        self.suggestions.clear();
        self.suggestion_index = None;
        self.mode = PromptMode::Normal;
        self.reset_interrupt_confirmation();
    }

    pub fn set_attachment_status_hint(&mut self, hint: Option<String>) {
        self.attachment_status_hint = hint
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }

    pub fn set_return_flow_text(&mut self, text: Option<String>) {
        self.return_flow_text = text.filter(|value| !value.trim().is_empty());
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_input(&mut self, input: String) {
        self.input = input;
        self.cursor_position = self.input.len();
        self.history_index = None;
        self.history_draft = None;
        self.reset_interrupt_confirmation();
        self.recompute_suggestions();
    }

    pub fn mode(&self) -> PromptMode {
        self.mode
    }

    pub fn is_shell_mode(&self) -> bool {
        matches!(self.mode, PromptMode::Shell)
    }

    pub fn exit_shell_mode(&mut self) {
        self.mode = PromptMode::Normal;
    }

    pub fn register_interrupt_keypress(&mut self) -> bool {
        if self.interrupt_confirmation_active() {
            self.reset_interrupt_confirmation();
            return true;
        }
        self.interrupt_press_count = 1;
        self.last_interrupt_time = Some(Instant::now());
        false
    }

    pub fn clear_interrupt_confirmation(&mut self) {
        self.reset_interrupt_confirmation();
    }

    pub fn insert_text(&mut self, text: &str) {
        self.input.insert_str(self.cursor_position, text);
        self.cursor_position = self.cursor_position.saturating_add(text.len());
        self.reset_history_cursor();
        self.recompute_suggestions();
    }

    pub fn desired_height(&self, width: u16) -> u16 {
        self.input_display_lines(width)
            .saturating_add(PROMPT_BLOCK_PAD_TOP)
            .saturating_add(PROMPT_BLOCK_PAD_BOTTOM)
            .saturating_add(if self.shows_activity_row() { 3 } else { 2 })
            .saturating_add(if self.return_flow_text.is_some() {
                1
            } else {
                0
            })
            // `render()` draws an outer bordered card, so callers that size the
            // prompt from `desired_height()` need the full surface height, not
            // just the inner content rows.
            .saturating_add(2)
    }

    fn shows_activity_row(&self) -> bool {
        matches!(
            self.current_session_status(),
            Some(
                SessionStatus::Running
                    | SessionStatus::Compacting
                    | SessionStatus::Reconnecting
                    | SessionStatus::Retrying { .. }
            )
        )
    }

    pub fn stash_current(&mut self) -> bool {
        if self.input.trim().is_empty() {
            return false;
        }

        self.stash.push(PromptStashEntry {
            input: self.input.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
        });
        if self.stash.len() > MAX_STASH_ENTRIES {
            let overflow = self.stash.len().saturating_sub(MAX_STASH_ENTRIES);
            self.stash.drain(0..overflow);
        }
        store_stash(&self.stash_path, &self.stash);
        self.clear();
        true
    }

    pub fn stash_entries(&self) -> &[PromptStashEntry] {
        &self.stash
    }

    pub fn pop_stash(&mut self) -> Option<PromptStashEntry> {
        let entry = self.stash.pop();
        if entry.is_some() {
            store_stash(&self.stash_path, &self.stash);
        }
        entry
    }

    pub fn remove_stash(&mut self, index: usize) -> bool {
        if index >= self.stash.len() {
            return false;
        }
        self.stash.remove(index);
        store_stash(&self.stash_path, &self.stash);
        true
    }

    pub fn load_stash(&mut self, index: usize) -> bool {
        let Some(entry) = self.stash.get(index) else {
            return false;
        };
        self.set_input(entry.input.clone());
        true
    }

    fn history_previous(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            Some(idx) => {
                if idx > 0 {
                    self.history_index = Some(idx - 1);
                }
            }
            None => {
                self.history_draft = Some(self.input.clone());
                self.history_index = Some(self.history.len() - 1);
            }
        }

        if let Some(idx) = self.history_index {
            self.input = self.history[idx].clone();
            self.cursor_position = self.input.len();
            self.recompute_suggestions();
        }
    }

    fn history_next(&mut self) {
        let Some(idx) = self.history_index else {
            return;
        };

        if idx + 1 < self.history.len() {
            let next = idx + 1;
            self.history_index = Some(next);
            self.input = self.history[next].clone();
            self.cursor_position = self.input.len();
        } else {
            self.history_index = None;
            self.input = self.history_draft.take().unwrap_or_default();
            self.cursor_position = self.input.len();
        }
        self.recompute_suggestions();
    }

    fn reset_history_cursor(&mut self) {
        self.history_index = None;
        self.history_draft = None;
    }

    fn push_history(&mut self, entry: String) {
        if self
            .history
            .last()
            .is_some_and(|existing| existing == &entry)
        {
            return;
        }
        self.history.push(entry);
        if self.history.len() > MAX_HISTORY_ENTRIES {
            let overflow = self.history.len().saturating_sub(MAX_HISTORY_ENTRIES);
            self.history.drain(0..overflow);
        }
    }

    fn bump_frecency(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let entry = self.frecency.entry(key.to_string()).or_default();
        entry.frequency = entry.frequency.saturating_add(1);
        entry.last_used = now;

        if self.frecency.len() > MAX_FRECENCY_ENTRIES {
            let mut items = self.frecency.iter().collect::<Vec<_>>();
            items.sort_by(|(_, a), (_, b)| b.last_used.cmp(&a.last_used));
            let keep = items
                .into_iter()
                .take(MAX_FRECENCY_ENTRIES)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<HashMap<_, _>>();
            self.frecency = keep;
        }
    }

    fn frecency_score(&self, key: &str) -> f64 {
        let Some(entry) = self.frecency.get(key) else {
            return 0.0;
        };
        let now = chrono::Utc::now().timestamp_millis();
        let days_since = (now - entry.last_used).max(0) as f64 / 86_400_000.0;
        let weight = 1.0 / (1.0 + days_since);
        entry.frequency as f64 * weight
    }

    fn selected_suggestion(&self) -> Option<&str> {
        self.suggestion_index
            .and_then(|idx| self.suggestions.get(idx))
            .map(String::as_str)
    }

    fn refresh_file_index_if_needed(&mut self) {
        let directory = self.context.directory.read().clone();
        let root = if directory.trim().is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            PathBuf::from(directory)
        };
        self.file_index.refresh(&root, FILE_INDEX_MAX_DEPTH);
    }

    fn push_candidate(
        scored: &mut Vec<(String, i32)>,
        dedup: &mut HashMap<String, ()>,
        token: &str,
        token_lower: &str,
        item: String,
        pre_scored: Option<i32>,
    ) {
        if item.eq_ignore_ascii_case(token) {
            return;
        }
        if dedup.insert(item.to_lowercase(), ()).is_some() {
            return;
        }

        let score = pre_scored
            .or_else(|| crate::command::fuzzy_match(token, &item))
            .or_else(|| {
                if item.to_lowercase().starts_with(token_lower) {
                    Some(1)
                } else {
                    None
                }
            });
        if let Some(score) = score {
            scored.push((item, score));
        }
    }

    fn recompute_suggestions(&mut self) {
        let Some((_, _, token)) = self.current_token() else {
            self.suggestions.clear();
            self.suggestion_index = None;
            return;
        };

        if token.is_empty() {
            self.suggestions.clear();
            self.suggestion_index = None;
            return;
        }

        let token_lower = token.to_lowercase();
        let mut dedup = HashMap::<String, ()>::new();
        let mut scored: Vec<(String, i32)> = Vec::new();
        if token.starts_with('/') {
            for item in self.known_commands.iter().cloned() {
                Self::push_candidate(
                    &mut scored,
                    &mut dedup,
                    token.as_str(),
                    token_lower.as_str(),
                    item,
                    None,
                );
            }
            for item in self
                .known_skills
                .iter()
                .map(|skill| format!("/{}", skill.trim()))
            {
                Self::push_candidate(
                    &mut scored,
                    &mut dedup,
                    token.as_str(),
                    token_lower.as_str(),
                    item,
                    None,
                );
            }
        } else if let Some(after_at) = token.strip_prefix('@') {
            // Agent completions
            for item in self
                .known_agents
                .iter()
                .map(|agent| format!("@{}", agent.trim()))
            {
                Self::push_candidate(
                    &mut scored,
                    &mut dedup,
                    token.as_str(),
                    token_lower.as_str(),
                    item,
                    None,
                );
            }
            // File path completions: strip @ prefix and optional #line range
            let (file_part, _line_range) = extract_line_range(after_at);
            let range_suffix = after_at
                .strip_prefix(file_part)
                .filter(|suffix| suffix.starts_with('#'))
                .unwrap_or("");
            self.refresh_file_index_if_needed();
            let path_score_boost = if file_part.trim().is_empty() { 0 } else { 1024 };
            for (path, path_score) in self.file_index.search(file_part, FILE_SUGGESTION_LIMIT) {
                let candidate = format!("@{}{}", path, range_suffix);
                let path_score = i32::try_from(path_score).unwrap_or(i32::MAX);
                let score = path_score.saturating_add(path_score_boost);
                Self::push_candidate(
                    &mut scored,
                    &mut dedup,
                    token.as_str(),
                    token_lower.as_str(),
                    candidate,
                    Some(score),
                );
            }
        } else {
            for item in self.history.iter().rev().cloned() {
                let history_score = subsequence_score(token.as_str(), item.as_str());
                Self::push_candidate(
                    &mut scored,
                    &mut dedup,
                    token.as_str(),
                    token_lower.as_str(),
                    item,
                    history_score,
                );
            }
        }

        scored.sort_by(|a, b| {
            let frecency_cmp = self
                .frecency_score(&b.0)
                .partial_cmp(&self.frecency_score(&a.0))
                .unwrap_or(std::cmp::Ordering::Equal);
            b.1.cmp(&a.1)
                .then(frecency_cmp)
                .then_with(|| a.0.len().cmp(&b.0.len()))
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });

        self.suggestions = scored.into_iter().map(|(item, _)| item).collect();
        self.suggestion_index = if self.suggestions.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn apply_autocomplete_next(&mut self) {
        if self.suggestions.is_empty() {
            self.recompute_suggestions();
            if self.suggestions.is_empty() {
                return;
            }
        }

        self.suggestion_index = Some(self.suggestion_index.unwrap_or(0));
        self.apply_selected_suggestion();
    }

    fn apply_autocomplete_previous(&mut self) {
        if self.suggestions.is_empty() {
            self.recompute_suggestions();
            if self.suggestions.is_empty() {
                return;
            }
        }

        self.suggestion_index = Some(
            self.suggestion_index
                .map(|current| {
                    if current == 0 {
                        self.suggestions.len() - 1
                    } else {
                        current - 1
                    }
                })
                .unwrap_or(self.suggestions.len() - 1),
        );
        self.apply_selected_suggestion();
    }

    fn apply_selected_suggestion(&mut self) {
        let Some((start, end, _)) = self.current_token() else {
            return;
        };
        let Some(suggestion) = self.selected_suggestion().map(ToString::to_string) else {
            return;
        };

        self.input.replace_range(start..end, &suggestion);
        self.cursor_position = start + suggestion.len();
        if (suggestion.starts_with('/') || suggestion.starts_with('@'))
            && self.cursor_position < self.input.len()
            && !self
                .input
                .as_bytes()
                .get(self.cursor_position)
                .copied()
                .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.input.insert(self.cursor_position, ' ');
            self.cursor_position += 1;
        }
        self.recompute_suggestions();
    }

    fn current_token(&self) -> Option<(usize, usize, String)> {
        if self.input.is_empty() || self.cursor_position > self.input.len() {
            return None;
        }

        let bytes = self.input.as_bytes();
        let mut start = self.cursor_position;
        while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }

        let mut end = self.cursor_position;
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
            end += 1;
        }

        let token = self.input[start..self.cursor_position].to_string();
        Some((start, end, token))
    }

    fn input_display_lines(&self, width: u16) -> u16 {
        let reserved = 1u16
            .saturating_add(PROMPT_BLOCK_PAD_LEFT)
            .saturating_add(PROMPT_BLOCK_PAD_RIGHT);
        let input_width = usize::from(width.saturating_sub(reserved)).max(1);
        let raw_lines = visual_line_count(&self.input, input_width) as u16;
        raw_lines.clamp(PROMPT_MIN_INPUT_LINES, PROMPT_MAX_INPUT_LINES)
    }

    fn cursor_visual_offset(&self, width: usize, max_row: usize) -> (usize, usize) {
        let mut row = 0usize;
        let mut col = 0usize;
        let safe_cursor = self.cursor_position.min(self.input.len());
        let prefix = &self.input[..safe_cursor];

        for ch in prefix.chars() {
            if ch == '\n' {
                row = row.saturating_add(1);
                col = 0;
                continue;
            }

            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col > 0 && col + ch_width > width {
                row = row.saturating_add(1);
                col = 0;
            }
            col = col.saturating_add(ch_width);
        }

        (row.min(max_row), col.min(width.saturating_sub(1)))
    }

    fn render_status_line(&self, theme: &Theme) -> Line<'static> {
        if let Some(status) = self.current_session_status() {
            if !matches!(status, SessionStatus::Idle) {
                return self.status_line_for_session(status, theme);
            }
        }
        self.render_secondary_hint_line(theme)
    }

    /// Unified secondary hint line — single exit for idle-state information.
    ///
    /// Priority (D1):
    /// 1. attachment hint + slash hint (layered with " · ")
    /// 2. attachment hint alone
    /// 3. slash hint alone
    /// 4. idle / help hint
    fn render_secondary_hint_line(&self, theme: &Theme) -> Line<'static> {
        let slash = self.current_slash_token();
        let attachment = self.attachment_status_hint.as_deref();

        match (attachment, slash) {
            (Some(att), Some(token)) => {
                let mut spans = vec![
                    Span::styled(att.to_string(), Style::default().fg(theme.text)),
                    Span::styled(" · ", Style::default().fg(theme.text_muted)),
                ];
                spans.extend(self.slash_hint_line(theme, &token).spans);
                Line::from(spans)
            }
            (Some(att), None) => {
                let base = self.hint_line(theme);
                let mut spans = vec![
                    Span::styled(att.to_string(), Style::default().fg(theme.text)),
                    Span::styled(" · ", Style::default().fg(theme.text_muted)),
                ];
                spans.extend(base.spans);
                Line::from(spans)
            }
            (None, Some(token)) => self.slash_hint_line(theme, &token),
            (None, None) => self.hint_line(theme),
        }
    }

    /// Placeholder only appears in truly idle state — no hot state present.
    fn should_show_placeholder(&self) -> bool {
        self.input.trim().is_empty()
            && self.current_token().is_none()
            && self.attachment_status_hint.is_none()
            && self.return_flow_text.is_none()
            && !self.spinner_active()
            && self
                .current_session_status()
                .map_or(true, |status| matches!(status, SessionStatus::Idle))
    }

    fn current_slash_token(&self) -> Option<String> {
        self.current_token().and_then(|(_, _, token)| {
            if token.starts_with('/') {
                Some(token)
            } else {
                None
            }
        })
    }

    fn current_session_status(&self) -> Option<SessionStatus> {
        let session_id = match self.context.current_route() {
            crate::router::Route::Session { session_id } => session_id,
            _ => return None,
        };
        let session_ctx = self.context.session.read();
        Some(session_ctx.status(&session_id).clone())
    }

    fn current_session_queue_count(&self) -> usize {
        let session_id = match self.context.current_route() {
            crate::router::Route::Session { session_id } => session_id,
            _ => return 0,
        };
        self.context.queued_prompts_for_session(&session_id)
    }

    fn status_line_for_session(&self, status: SessionStatus, theme: &Theme) -> Line<'static> {
        if matches!(self.mode, PromptMode::Shell) {
            return Line::from(vec![
                Span::styled("esc", Style::default().fg(theme.text)),
                Span::styled(" exit shell mode", Style::default().fg(theme.text_muted)),
            ]);
        }
        if self.context.get_pending_permission().is_some()
            || *self.context.pending_permissions.read() > 0
        {
            return Line::from(vec![Span::styled(
                canonical_run_status_title("awaiting_permission"),
                Style::default().fg(theme.warning),
            )]);
        }
        if self.context.has_pending_question() {
            return Line::from(vec![Span::styled(
                canonical_run_status_title("awaiting_user"),
                Style::default().fg(theme.warning),
            )]);
        }
        if matches!(status, SessionStatus::Idle) {
            if let Some(tail_status) = self.context.current_session_terminal_tail_status() {
                let tone = if tail_status == "error" {
                    theme.error
                } else {
                    theme.text_muted
                };
                return Line::from(vec![Span::styled(
                    canonical_run_status_title(&tail_status),
                    Style::default().fg(tone),
                )]);
            }
            return Line::from(vec![Span::styled(
                canonical_run_status_title("idle"),
                Style::default().fg(theme.text_muted),
            )]);
        }
        let interrupt = self.context.keybind.read().print("session_interrupt");
        match status {
            SessionStatus::Retrying {
                message,
                attempt,
                next,
            } => {
                let now = chrono::Utc::now().timestamp_millis();
                let secs = ((next - now) / 1000).max(0);
                let truncated = truncate_for_status(&message, 72);
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{} in {}s (#{}) ",
                            canonical_run_status_title("retrying"),
                            secs,
                            attempt
                        ),
                        Style::default().fg(theme.warning),
                    ),
                    Span::styled(truncated, Style::default().fg(theme.text_muted)),
                    Span::raw("  "),
                    Span::styled(interrupt, Style::default().fg(theme.text)),
                    Span::styled(" interrupt", Style::default().fg(theme.text_muted)),
                ])
            }
            SessionStatus::Running => {
                let mut spans = vec![
                    Span::styled(
                        canonical_run_status_title("running"),
                        Style::default().fg(theme.text_muted),
                    ),
                    Span::raw("  "),
                ];
                let queued = self.current_session_queue_count();
                if queued > 0 {
                    spans.push(Span::styled(
                        format!("queued {}", queued),
                        Style::default().fg(theme.text_muted),
                    ));
                    spans.push(Span::raw("  "));
                }
                if self.interrupt_confirmation_active() {
                    spans.push(Span::styled(interrupt, Style::default().fg(theme.warning)));
                    spans.push(Span::styled(
                        " again to interrupt",
                        Style::default().fg(theme.warning),
                    ));
                } else {
                    spans.push(Span::styled(interrupt, Style::default().fg(theme.text)));
                    spans.push(Span::styled(
                        " interrupt",
                        Style::default().fg(theme.text_muted),
                    ));
                }
                Line::from(spans)
            }
            SessionStatus::Compacting => {
                let mut spans = vec![
                    Span::styled(
                        canonical_run_status_title("compacting"),
                        Style::default().fg(theme.text_muted),
                    ),
                    Span::raw("  "),
                ];
                if self.interrupt_confirmation_active() {
                    spans.push(Span::styled(interrupt, Style::default().fg(theme.warning)));
                    spans.push(Span::styled(
                        " again to interrupt",
                        Style::default().fg(theme.warning),
                    ));
                } else {
                    spans.push(Span::styled(interrupt, Style::default().fg(theme.text)));
                    spans.push(Span::styled(
                        " interrupt",
                        Style::default().fg(theme.text_muted),
                    ));
                }
                Line::from(spans)
            }
            SessionStatus::Reconnecting => Line::from(vec![Span::styled(
                canonical_run_status_title("reconnecting"),
                Style::default().fg(theme.warning),
            )]),
            SessionStatus::WaitingOnUser => Line::from(vec![Span::styled(
                canonical_run_status_title("waiting_on_user"),
                Style::default().fg(theme.warning),
            )]),
            SessionStatus::Idle => self.hint_line(theme),
        }
    }

    fn render_token_line(&self, theme: &Theme) -> Line<'static> {
        let Some((input, output, reasoning, cache_read, cache_miss, cache_write)) =
            self.current_session_token_io()
        else {
            return Line::from("");
        };
        if input == 0
            && output == 0
            && reasoning == 0
            && cache_read == 0
            && cache_miss == 0
            && cache_write == 0
        {
            return Line::from("");
        }

        let mut spans = vec![
            Span::styled("Turn ", Style::default().fg(theme.text_muted)),
            Span::styled("↑", Style::default().fg(theme.text_muted)),
            Span::styled(
                format_compact_number(input),
                Style::default().fg(theme.text),
            ),
            Span::raw("  "),
            Span::styled("↓", Style::default().fg(theme.text_muted)),
            Span::styled(
                format_compact_number(output),
                Style::default().fg(theme.text),
            ),
        ];
        if reasoning > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled("R", Style::default().fg(theme.text_muted)));
            spans.push(Span::styled(
                format_compact_number(reasoning),
                Style::default().fg(theme.text),
            ));
        }
        if cache_read > 0 || cache_miss > 0 || cache_write > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "C H/M/W",
                Style::default().fg(theme.text_muted),
            ));
            spans.push(Span::styled(
                format!(
                    "{}/{}/{}",
                    format_compact_number(cache_read),
                    format_compact_number(cache_miss),
                    format_compact_number(cache_write)
                ),
                Style::default().fg(theme.text),
            ));
        }
        Line::from(spans)
    }

    fn current_session_token_io(&self) -> Option<(u64, u64, u64, u64, u64, u64)> {
        let session_id = match self.context.current_route() {
            crate::router::Route::Session { session_id } => session_id,
            _ => return None,
        };
        let session_ctx = self.context.session.read();
        let messages = session_ctx.messages.get(&session_id)?;
        let last_assistant = messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))?;
        Some((
            last_assistant.tokens.input,
            last_assistant.tokens.output,
            last_assistant.tokens.reasoning,
            last_assistant.tokens.cache_read,
            last_assistant.tokens.cache_miss,
            last_assistant.tokens.cache_write,
        ))
    }

    fn interrupt_confirmation_active(&self) -> bool {
        if self.interrupt_press_count == 0 {
            return false;
        }
        self.last_interrupt_time
            .is_some_and(|t| t.elapsed() < Duration::from_secs(INTERRUPT_CONFIRM_WINDOW_SECS))
    }

    fn maybe_reset_interrupt_confirmation(&mut self) -> bool {
        if !self.interrupt_confirmation_active() {
            return self.reset_interrupt_confirmation();
        }
        false
    }

    fn reset_interrupt_confirmation(&mut self) -> bool {
        let changed = self.interrupt_press_count != 0 || self.last_interrupt_time.is_some();
        self.interrupt_press_count = 0;
        self.last_interrupt_time = None;
        changed
    }

    fn active_placeholder(&self) -> &str {
        self.placeholder_for_mode_at(self.mode, self.current_placeholder_slot())
    }

    fn current_placeholder_slot(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / PLACEHOLDER_ROTATE_SECS
    }

    fn placeholder_for_mode_at(&self, mode: PromptMode, slot: u64) -> &str {
        let variants = match mode {
            PromptMode::Normal => &self.placeholder_variants,
            PromptMode::Shell => &self.shell_placeholders,
        };

        if variants.is_empty() {
            return match mode {
                PromptMode::Normal => self.placeholder.as_str(),
                PromptMode::Shell => SHELL_PLACEHOLDER,
            };
        }

        let index = (slot as usize) % variants.len();
        variants
            .get(index)
            .map(String::as_str)
            .unwrap_or(match mode {
                PromptMode::Normal => self.placeholder.as_str(),
                PromptMode::Shell => SHELL_PLACEHOLDER,
            })
    }

    fn hint_line(&self, theme: &Theme) -> Line<'static> {
        if let Some(token) = self.current_slash_token() {
            return self.slash_hint_line(theme, &token);
        }
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(theme.text)),
            Span::styled(" send", Style::default().fg(theme.text_muted)),
            Span::styled(" · ", Style::default().fg(theme.text_muted)),
            Span::styled("Ctrl+L", Style::default().fg(theme.text)),
            Span::styled(" clear", Style::default().fg(theme.text_muted)),
        ])
    }

    fn slash_hint_line(&self, theme: &Theme, token: &str) -> Line<'static> {
        let mut spans = vec![Span::styled(
            "Commands",
            Style::default().fg(theme.text_muted),
        )];

        let preview = self
            .suggestions
            .iter()
            .filter(|item| item.starts_with('/'))
            .take(3)
            .cloned()
            .collect::<Vec<_>>();

        if preview.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(theme.primary),
            ));
            spans.push(Span::styled(
                " will be sent directly",
                Style::default().fg(theme.text_muted),
            ));
            return Line::from(spans);
        }

        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "matches:",
            Style::default().fg(theme.text_muted),
        ));
        for item in preview {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(item, Style::default().fg(theme.primary)));
        }
        Line::from(spans)
    }
}

impl Component for PromptComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let theme = use_context::<Theme>();
        let animations_enabled = use_context::<crate::bridge::ReactiveAnimationsEnabled>().0;
        let prompt_input_blocked = use_context::<crate::bridge::ReactivePromptInputBlocked>().0;
        let event_emitter = use_context::<crate::bridge::ReactiveUiEventEmitter>().0;

        if !prompt_input_blocked {
            let prompt = self.prompt.clone();
            let emitter = event_emitter.clone();
            use_keyboard_press(move |key_event| {
                let key = crate::context::normalize_key_event(key_event);
                if !reactive_prompt_handles_key(key) {
                    return;
                }

                let mut next_prompt = prompt.clone();
                let submit = next_prompt.handle_key(key);
                let emitted = if submit {
                    emitter.emit_custom_event(crate::event::CustomEvent::PromptSubmitRequested {
                        prompt: Box::new(next_prompt),
                    })
                } else {
                    emitter.emit_custom_event(crate::event::CustomEvent::PromptEdited {
                        prompt: Box::new(next_prompt),
                    })
                };

                if emitted {
                    stop_propagation();
                }
            });

            if let Some(CrosstermEvent::Paste(text)) = use_event() {
                if !text.is_empty() {
                    let emitted =
                        event_emitter.emit_custom_event(crate::event::CustomEvent::PromptPasteText {
                            text: text.to_string(),
                        });
                    if emitted {
                        stop_propagation();
                    }
                }
            }
        }

        let mut surface = BufferSurface::new(buffer);
        self.prompt.render_with_theme_and_animations(
            &mut surface,
            area,
            &theme,
            animations_enabled,
            self.cursor_sink.as_ref(),
        );
    }
}

fn truncate_for_status(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::with_capacity(max_chars + 1);
    for ch in input.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
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

fn visual_line_count(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }

    let mut rows = 1usize;
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            rows += 1;
            col = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col > 0 && col + ch_width > width {
            rows += 1;
            col = 0;
        }
        col += ch_width;
    }

    rows.max(1)
}

fn dedup_sort(mut items: Vec<String>) -> Vec<String> {
    items.retain(|item| !item.trim().is_empty());
    items.sort_by_key(|a| a.to_ascii_lowercase());
    items.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    items
}

fn inset_horizontal(area: Rect, padding: u16) -> Rect {
    if area.width <= padding.saturating_mul(2) {
        return area;
    }
    Rect {
        x: area.x.saturating_add(padding),
        y: area.y,
        width: area.width.saturating_sub(padding.saturating_mul(2)),
        height: area.height,
    }
}

fn row_content_area(area: Rect, horizontal_padding: u16) -> Rect {
    if area.width == 0 {
        return area;
    }
    inset_horizontal(area, horizontal_padding)
}

fn render_prompt_continuation_row<S: RenderSurface>(
    surface: &mut S,
    area: Rect,
    background: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    surface.render_widget(
        Paragraph::new(" ".repeat(area.width as usize)).style(Style::default().bg(background)),
        area,
    );
}

fn subsequence_score(query: &str, target: &str) -> Option<i32> {
    let query_lower = query.trim().to_lowercase();
    if query_lower.is_empty() {
        return Some(0);
    }
    let target_lower = target.to_lowercase();
    let mut score = 0i32;
    let mut query_idx = 0usize;
    let query_chars: Vec<char> = query_lower.chars().collect();
    for (idx, ch) in target_lower.chars().enumerate() {
        if query_idx < query_chars.len() && ch == query_chars[query_idx] {
            score += if idx == 0 || query_idx == 0 { 10 } else { 5 };
            query_idx += 1;
        }
    }
    if query_idx == query_chars.len() {
        Some(score)
    } else {
        None
    }
}

fn current_mode_name(context: &Arc<AppContext>) -> Option<String> {
    context
        .current_scheduler_profile()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let agent = context.current_agent();
            (!agent.trim().is_empty()).then_some(agent)
        })
}

fn prompt_agent_color(theme: &Theme, agent_name: &str) -> Color {
    if theme.agent_colors.is_empty() {
        return theme.primary;
    }

    let mut hasher = DefaultHasher::new();
    agent_name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % theme.agent_colors.len();
    theme.agent_colors[idx]
}

fn spinner_mode_from_env() -> SpinnerMode {
    match std::env::var("AGENDAO_TUI_SPINNER")
        .ok()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "knight" | "knight-rider" | "scan" | "scanner" => SpinnerMode::KnightRider,
        _ => SpinnerMode::Braille,
    }
}

fn render_prompt_identity_spans(
    context: &Arc<AppContext>,
    theme: &Theme,
    mode: PromptMode,
    accent: Color,
) -> Vec<Span<'static>> {
    let (icon, label) = if matches!(mode, PromptMode::Shell) {
        ("⌘", "shell".to_string())
    } else if let Some(profile) = context
        .current_scheduler_profile()
        .filter(|value| !value.trim().is_empty())
    {
        ("☯", profile)
    } else if let Some(agent) = current_mode_name(context) {
        ("☯", agent)
    } else {
        ("☯", "auto".to_string())
    };

    vec![
        Span::styled(icon, Style::default().fg(accent)),
        Span::raw(" "),
        Span::styled(label, Style::default().fg(theme.text)),
    ]
}

fn reactive_prompt_handles_key(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(_) => {
            let forbidden_modifiers =
                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER;
            !key.modifiers.intersects(forbidden_modifiers)
        }
        KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::Tab
        | KeyCode::BackTab
        | KeyCode::Enter
        | KeyCode::Esc
        | KeyCode::Up
        | KeyCode::Down => true,
        _ => false,
    }
}

fn prompt_model_summary(model: Option<&str>, provider: Option<&str>) -> Option<String> {
    let model = model?.trim();
    if model.is_empty() {
        return None;
    }

    let normalized = provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .and_then(|provider| model.strip_prefix(&format!("{provider}/")))
        .unwrap_or(model)
        .trim();

    (!normalized.is_empty()).then(|| normalized.to_string())
}

fn prev_char_boundary(input: &str, cursor_position: usize) -> Option<usize> {
    if cursor_position == 0 || cursor_position > input.len() {
        return None;
    }
    input[..cursor_position]
        .char_indices()
        .last()
        .map(|(idx, _)| idx)
}

fn next_char_boundary(input: &str, cursor_position: usize) -> Option<usize> {
    if cursor_position >= input.len() {
        return None;
    }
    let suffix = &input[cursor_position..];
    suffix
        .chars()
        .next()
        .map(|ch| cursor_position + ch.len_utf8())
}

fn prompt_state_dir() -> PathBuf {
    let base = std::env::var("AGENDAO_STATE_DIR")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::state_dir().map(|d| d.join("agendao")))
        .unwrap_or_else(|| std::env::temp_dir().join("agendao"));
    let path = base.join("tui");
    let _ = std::fs::create_dir_all(&path);
    path
}

fn load_history(path: &PathBuf) -> Vec<String> {
    let store = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<HistoryStore>(&s).ok())
        .unwrap_or_default();
    let mut entries = store.entries;
    if entries.len() > MAX_HISTORY_ENTRIES {
        let overflow = entries.len() - MAX_HISTORY_ENTRIES;
        entries.drain(0..overflow);
    }
    entries
}

fn load_frecency(path: &PathBuf) -> HashMap<String, FrecencyEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<FrecencyStore>(&s).ok())
        .map(|store| store.entries)
        .unwrap_or_default()
}

fn load_stash(path: &PathBuf) -> Vec<PromptStashEntry> {
    let store = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<StashStore>(&s).ok())
        .unwrap_or_default();
    let mut entries = store.entries;
    if entries.len() > MAX_STASH_ENTRIES {
        let overflow = entries.len() - MAX_STASH_ENTRIES;
        entries.drain(0..overflow);
    }
    entries
}

fn store_history(path: &PathBuf, entries: &[String]) {
    let payload = HistoryStore {
        entries: entries.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = std::fs::write(path, json);
    }
}

fn store_frecency(path: &PathBuf, entries: &HashMap<String, FrecencyEntry>) {
    let payload = FrecencyStore {
        entries: entries.clone(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = std::fs::write(path, json);
    }
}

fn store_stash(path: &PathBuf, entries: &[PromptStashEntry]) {
    let payload = StashStore {
        entries: entries.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let _ = std::fs::write(path, json);
    }
}

/// Extract a `#line` or `#line-line` range suffix from a file path reference.
/// Returns the base path and an optional (start, optional_end) line range.
fn extract_line_range(input: &str) -> (&str, Option<(usize, Option<usize>)>) {
    if let Some(hash_idx) = input.rfind('#') {
        let base = &input[..hash_idx];
        let range_str = &input[hash_idx + 1..];
        if let Some(dash_idx) = range_str.find('-') {
            let start = range_str[..dash_idx].parse().ok();
            let end = range_str[dash_idx + 1..].parse().ok();
            if let Some(s) = start {
                return (base, Some((s, end)));
            }
        } else if let Ok(line) = range_str.parse::<usize>() {
            return (base, Some((line, None)));
        }
    }
    (input, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Message, MessageRole, TokenUsage};
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn with_isolated_prompt<T>(f: impl FnOnce(Prompt) -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let state_dir =
            std::env::temp_dir().join(format!("agendao-tui-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        let previous = std::env::var("AGENDAO_STATE_DIR").ok();
        std::env::set_var("AGENDAO_STATE_DIR", &state_dir);

        let context = Arc::new(AppContext::new());
        let prompt = Prompt::new(context);
        let result = f(prompt);

        if let Some(prev) = previous {
            std::env::set_var("AGENDAO_STATE_DIR", prev);
        } else {
            std::env::remove_var("AGENDAO_STATE_DIR");
        }
        let _ = std::fs::remove_dir_all(state_dir);
        result
    }

    fn line_text(line: Line<'static>) -> String {
        line.spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>()
    }

    #[test]
    fn tab_autocomplete_uses_first_candidate() {
        with_isolated_prompt(|mut prompt| {
            prompt.set_input("team".to_string());
            let _ = prompt.take_input();
            prompt.set_input("test".to_string());
            let _ = prompt.take_input();
            prompt.set_input("te".to_string());

            prompt.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
            let completed = prompt.get_input().to_string();
            assert!(
                completed == "team" || completed == "test",
                "unexpected completion: {}",
                completed
            );
        });
    }

    #[test]
    fn tab_autocomplete_keeps_line_range_for_file_candidates() {
        with_isolated_prompt(|mut prompt| {
            let root =
                std::env::temp_dir().join(format!("agendao-tui-files-{}", uuid::Uuid::new_v4()));
            let src_dir = root.join("src");
            std::fs::create_dir_all(&src_dir).expect("create src dir");
            std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("write file");
            *prompt.context.directory.write() = root.to_string_lossy().to_string();

            prompt.set_input("@src/main#12-20".to_string());
            prompt.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
            assert_eq!(prompt.get_input(), "@src/main.rs#12-20");

            let _ = std::fs::remove_dir_all(root);
        });
    }

    #[test]
    fn history_navigation_preserves_draft() {
        with_isolated_prompt(|mut prompt| {
            prompt.set_input("alpha".to_string());
            let _ = prompt.take_input();
            prompt.set_input("beta".to_string());
            let _ = prompt.take_input();
            prompt.set_input("draft".to_string());

            prompt.history_previous_entry();
            assert_eq!(prompt.get_input(), "beta");

            prompt.history_previous_entry();
            assert_eq!(prompt.get_input(), "alpha");

            prompt.history_next_entry();
            assert_eq!(prompt.get_input(), "beta");

            prompt.history_next_entry();
            assert_eq!(prompt.get_input(), "draft");
        });
    }

    #[test]
    fn running_status_line_uses_canonical_tail_title() {
        with_isolated_prompt(|prompt| {
            let theme = prompt.context.theme.read().clone();
            let rendered =
                line_text(prompt.status_line_for_session(SessionStatus::Running, &theme));
            assert!(rendered.contains("Running"), "{rendered}");
            assert!(!rendered.contains("thinking"), "{rendered}");
        });
    }

    #[test]
    fn pending_permission_overrides_running_status_line() {
        with_isolated_prompt(|prompt| {
            prompt.context.set_pending_permissions(1);
            let theme = prompt.context.theme.read().clone();
            let rendered =
                line_text(prompt.status_line_for_session(SessionStatus::Running, &theme));
            assert_eq!(rendered, "Waiting for permission");
        });
    }

    #[test]
    fn reconnecting_status_line_uses_canonical_tail_title() {
        with_isolated_prompt(|prompt| {
            let theme = prompt.context.theme.read().clone();
            let rendered =
                line_text(prompt.status_line_for_session(SessionStatus::Reconnecting, &theme));
            assert_eq!(rendered, "Reconnecting stream");
        });
    }

    #[test]
    fn idle_status_line_surfaces_run_complete() {
        with_isolated_prompt(|prompt| {
            let session_id = {
                let mut session = prompt.context.session.write();
                let session_id = session.data.create_session(Some("Test".to_string()));
                session.data.add_message(
                    &session_id,
                    Message {
                        id: "assistant-complete".to_string(),
                        role: MessageRole::Assistant,
                        content: "done".to_string(),
                        created_at: Utc::now(),
                        agent: None,
                        model: None,
                        mode: None,
                        finish: Some("stop".to_string()),
                        error: None,
                        completed_at: Some(Utc::now()),
                        cost: 0.0,
                        tokens: TokenUsage {
                            output: 1,
                            ..TokenUsage::default()
                        },
                        metadata: None,
                        multimodal: None,
                        parts: Vec::new(),
                    },
                );
                session_id
            };
            prompt.context.navigate_session(session_id);
            let theme = prompt.context.theme.read().clone();
            let rendered = line_text(prompt.status_line_for_session(SessionStatus::Idle, &theme));
            assert_eq!(rendered, "Run complete");
        });
    }

    #[test]
    fn idle_status_line_surfaces_run_failed() {
        with_isolated_prompt(|prompt| {
            let session_id = {
                let mut session = prompt.context.session.write();
                let session_id = session.data.create_session(Some("Test".to_string()));
                session.data.add_message(
                    &session_id,
                    Message {
                        id: "assistant-error".to_string(),
                        role: MessageRole::Assistant,
                        content: "boom".to_string(),
                        created_at: Utc::now(),
                        agent: None,
                        model: None,
                        mode: None,
                        finish: Some("error".to_string()),
                        error: Some("boom".to_string()),
                        completed_at: Some(Utc::now()),
                        cost: 0.0,
                        tokens: TokenUsage::default(),
                        metadata: None,
                        multimodal: None,
                        parts: Vec::new(),
                    },
                );
                session_id
            };
            prompt.context.navigate_session(session_id);
            let theme = prompt.context.theme.read().clone();
            let rendered = line_text(prompt.status_line_for_session(SessionStatus::Idle, &theme));
            assert_eq!(rendered, "Run failed");
        });
    }

    #[test]
    fn slash_hint_line_surfaces_inside_idle_session() {
        with_isolated_prompt(|mut prompt| {
            let session_id = {
                let mut session = prompt.context.session.write();
                session.data.create_session(Some("Test".to_string()))
            };
            prompt.context.navigate_session(session_id);
            prompt.set_input("/sess".to_string());

            let theme = prompt.context.theme.read().clone();
            let rendered = line_text(prompt.render_status_line(&theme));

            assert!(rendered.contains("matches:"), "{rendered}");
            assert!(rendered.contains("/session"), "{rendered}");
        });
    }

    #[test]
    fn attachment_hint_layers_with_idle_hint_line() {
        with_isolated_prompt(|mut prompt| {
            prompt.set_attachment_status_hint(Some("1 image attached".to_string()));
            let theme = prompt.context.theme.read().clone();
            let rendered = line_text(prompt.render_status_line(&theme));
            assert!(rendered.contains("1 image attached"), "{rendered}");
            assert!(rendered.contains("Enter send"), "{rendered}");
            assert!(rendered.contains("Ctrl+L clear"), "{rendered}");
        });
    }

    #[test]
    fn attachment_hint_layers_with_slash_hint_line() {
        with_isolated_prompt(|mut prompt| {
            let session_id = {
                let mut session = prompt.context.session.write();
                session.data.create_session(Some("Test".to_string()))
            };
            prompt.context.navigate_session(session_id);
            prompt.set_input("/sess".to_string());
            prompt.set_attachment_status_hint(Some("1 image attached".to_string()));

            let theme = prompt.context.theme.read().clone();
            let rendered = line_text(prompt.render_status_line(&theme));

            assert!(rendered.contains("1 image attached"), "{rendered}");
            assert!(rendered.contains("matches:"), "{rendered}");
            assert!(rendered.contains("/session"), "{rendered}");
        });
    }

    #[test]
    fn placeholder_sets_rotate_by_mode() {
        with_isolated_prompt(|prompt| {
            let prompt = prompt.with_placeholders(
                ["Fix broken tests", "Explain this repo"],
                ["git status", "pwd"],
            );

            assert_eq!(
                prompt.placeholder_for_mode_at(PromptMode::Normal, 0),
                "Fix broken tests"
            );
            assert_eq!(
                prompt.placeholder_for_mode_at(PromptMode::Normal, 1),
                "Explain this repo"
            );
            assert_eq!(
                prompt.placeholder_for_mode_at(PromptMode::Shell, 0),
                "git status"
            );
            assert_eq!(prompt.placeholder_for_mode_at(PromptMode::Shell, 1), "pwd");
        });
    }

    #[test]
    fn desired_height_preserves_two_line_input_when_idle_and_empty() {
        with_isolated_prompt(|prompt| {
            assert_eq!(prompt.desired_height(80), 7);
        });
    }

    #[test]
    fn desired_height_ignores_context_meter_availability() {
        with_isolated_prompt(|prompt| {
            {
                let mut providers = prompt.context.providers.write();
                providers.push(crate::context::ProviderInfo {
                    id: "openai".to_string(),
                    name: "OpenAI".to_string(),
                    models: vec![crate::context::ModelInfo {
                        id: "openai/gpt-5".to_string(),
                        name: "gpt-5".to_string(),
                        context_window: 200_000,
                        max_output_tokens: 0,
                        supports_vision: false,
                        supports_tools: true,
                        cost_per_million_input: None,
                        cost_per_million_output: None,
                    }],
                });
            }
            prompt
                .context
                .set_model("openai/gpt-5".to_string(), "openai".to_string());
            prompt.context.navigate_session("session-1");
            prompt.context.apply_session_projection_snapshot(
                "session-1",
                None,
                Vec::new(),
                None,
                Some(agendao_types::SessionUsageBooks {
                    live_context_tokens: Some(52_830),
                    request_context_tokens: Some(52_830),
                    ..Default::default()
                }),
                None,
                None,
                None,
                None,
            );

            assert_eq!(prompt.desired_height(80), 7);
        });
    }

    #[test]
    fn prompt_model_summary_omits_redundant_provider_prefix() {
        assert_eq!(
            prompt_model_summary(
                Some("zhipuai-coding-plan/glm-5.1"),
                Some("zhipuai-coding-plan")
            )
            .as_deref(),
            Some("glm-5.1")
        );
        assert_eq!(
            prompt_model_summary(Some("claude-sonnet-4"), Some("anthropic")).as_deref(),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn utf8_backspace_delete_and_cursor_are_char_safe() {
        with_isolated_prompt(|mut prompt| {
            prompt.set_input("你好".to_string());

            prompt.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::empty()));
            prompt.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()));
            assert_eq!(prompt.get_input(), "好");

            prompt.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::empty()));
            prompt.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()));
            assert_eq!(prompt.get_input(), "");
        });
    }

    // ── D1: placeholder gate & secondary hint ─────────────────────

    fn idle_prompt() -> Prompt {
        let context = Arc::new(AppContext::new());
        let mut prompt = Prompt::new(context);
        // Ensure no placeholder variants influence the gate test —
        // empty input + no hot state = truly idle.
        prompt.placeholder_variants.clear();
        prompt.shell_placeholders.clear();
        prompt
    }

    #[test]
    fn placeholder_hidden_when_return_flow_present() {
        let mut prompt = idle_prompt();
        prompt.set_return_flow_text(Some("Last turn: ↑12k ↓3.4k".to_string()));
        assert!(!prompt.should_show_placeholder());
    }

    #[test]
    fn placeholder_hidden_when_attachment_hint_present() {
        let mut prompt = idle_prompt();
        prompt.set_attachment_status_hint(Some("1 image attached".to_string()));
        assert!(!prompt.should_show_placeholder());
    }

    #[test]
    fn placeholder_hidden_when_spinner_active() {
        let mut prompt = idle_prompt();
        prompt.set_spinner_active(true);
        assert!(!prompt.should_show_placeholder());
    }

    #[test]
    fn placeholder_hidden_when_session_running() {
        let prompt = idle_prompt();
        // Plumb a running session through the shared context so
        // current_session_status() sees it via the route.
        let sid = {
            let mut session = prompt.context.session.write();
            let sid = session.create_session(Some("test-session".to_string()));
            session.set_current_session_id(sid.clone());
            session.set_status(&sid, SessionStatus::Running);
            sid
        };
        // Navigate to the session route so prompt sees the status.
        prompt.context.navigate_session(sid);
        assert!(!prompt.should_show_placeholder());
    }

    #[test]
    fn secondary_hint_prefers_attachment_plus_slash_over_idle_hint() {
        let mut prompt = idle_prompt();
        prompt.set_attachment_status_hint(Some("2 images".to_string()));
        prompt.input = "/mcp".to_string();
        prompt.cursor_position = 4;
        prompt.recompute_suggestions();
        let theme = Theme::default();
        let hint = line_text(prompt.render_secondary_hint_line(&theme));
        assert!(
            hint.contains("2 images"),
            "attachment hint should be present"
        );
        assert!(hint.contains("/mcp"), "slash hint should be present");
    }
}
