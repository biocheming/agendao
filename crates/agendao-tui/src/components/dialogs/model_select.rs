use std::collections::HashMap;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;
use crate::ui::RenderSurface;

/// Popular providers shown first (matches TS opencode ordering).
const POPULAR_PROVIDERS: &[&str] = &[
    "ethnopic",
    "github-copilot",
    "openai",
    "google",
    "openrouter",
    "vercel",
    "deepseek",
];

const RECENT_LIMIT: usize = 5;

#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_window: u64,
}

/// A display row in the model list — either a group header or a selectable model.
#[derive(Clone)]
enum Row {
    Header(String),
    Model { index: usize, is_recent: bool },
}

pub struct ModelSelectDialog {
    models: Vec<Model>,
    /// (provider_id, model_id) pairs, most recent first.
    recent: Vec<(String, String)>,
    /// Currently active model key: "provider/model_id".
    current_model: Option<String>,
    /// Flat display rows (headers + models) after filtering/grouping.
    rows: Vec<Row>,
    /// Indices into `rows` that are selectable (Model rows only).
    selectable: Vec<usize>,
    cursor: usize,
    scroll_offset: usize,
    query: String,
    open: bool,
}

impl ModelSelectDialog {
    pub fn new() -> Self {
        Self {
            models: Vec::new(),
            recent: Vec::new(),
            current_model: None,
            rows: Vec::new(),
            selectable: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            query: String::new(),
            open: false,
        }
    }

    pub fn set_models(&mut self, models: Vec<Model>) {
        self.models = models;
        self.rebuild();
    }

    pub fn set_current_model(&mut self, key: Option<String>) {
        self.current_model = key;
    }

    /// Record a model as recently used (pushed to front, capped at RECENT_LIMIT).
    pub fn push_recent(&mut self, provider: &str, model_id: &str) {
        let entry = (provider.to_string(), model_id.to_string());
        self.recent.retain(|r| r != &entry);
        self.recent.insert(0, entry);
        if self.recent.len() > RECENT_LIMIT {
            self.recent.truncate(RECENT_LIMIT);
        }
    }

    /// Return a slice of the recent models list for persistence.
    pub fn recent(&self) -> &[(String, String)] {
        &self.recent
    }

    /// Replace the recent models list (used on startup to restore persisted state).
    pub fn set_recent(&mut self, recent: Vec<(String, String)>) {
        self.recent = recent;
        if self.recent.len() > RECENT_LIMIT {
            self.recent.truncate(RECENT_LIMIT);
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.rebuild();
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.rebuild();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.rebuild();
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor < self.selectable.len().saturating_sub(1) {
            self.cursor += 1;
        }
    }

    pub fn selected_model(&self) -> Option<&Model> {
        self.selectable
            .get(self.cursor)
            .and_then(|&row_idx| match &self.rows[row_idx] {
                Row::Model { index, .. } => self.models.get(*index),
                _ => None,
            })
    }

    /// Rebuild the flat row list from models, applying search filter and grouping.
    fn rebuild(&mut self) {
        let query_lower = self.query.to_lowercase();
        let filtered: Vec<usize> = self
            .models
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                query_lower.is_empty()
                    || m.name.to_lowercase().contains(&query_lower)
                    || m.provider.to_lowercase().contains(&query_lower)
                    || m.id.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect();

        let mut rows = Vec::new();
        let mut used = vec![false; self.models.len()];

        // --- Recent section ---
        let recent_indices: Vec<usize> = self
            .recent
            .iter()
            .filter_map(|(prov, mid)| {
                filtered.iter().copied().find(|&i| {
                    let m = &self.models[i];
                    m.provider == *prov && m.id == *mid
                })
            })
            .collect();

        if !recent_indices.is_empty() {
            rows.push(Row::Header("Recent".into()));
            for &idx in &recent_indices {
                rows.push(Row::Model {
                    index: idx,
                    is_recent: true,
                });
                used[idx] = true;
            }
        }

        // --- Group remaining by provider (popular first) ---
        let remaining: Vec<usize> = filtered.iter().copied().filter(|&i| !used[i]).collect();
        let mut by_provider: HashMap<&str, Vec<usize>> = HashMap::new();
        for &idx in &remaining {
            by_provider
                .entry(self.models[idx].provider.as_str())
                .or_default()
                .push(idx);
        }

        // Sort provider keys: popular first, then alphabetical
        let mut provider_keys: Vec<&str> = by_provider.keys().copied().collect();
        provider_keys.sort_by(|a, b| {
            let a_pop = POPULAR_PROVIDERS.iter().position(|&p| p == *a);
            let b_pop = POPULAR_PROVIDERS.iter().position(|&p| p == *b);
            match (a_pop, b_pop) {
                (Some(ai), Some(bi)) => ai.cmp(&bi),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
            }
        });

        for provider in provider_keys {
            let mut indices = by_provider.remove(provider).unwrap_or_default();
            indices.sort_by(|&a, &b| {
                self.models[a]
                    .name
                    .to_lowercase()
                    .cmp(&self.models[b].name.to_lowercase())
            });
            rows.push(Row::Header(provider.to_string()));
            for idx in indices {
                rows.push(Row::Model {
                    index: idx,
                    is_recent: false,
                });
            }
        }

        // Build selectable index
        let selectable: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::Model { .. }).then_some(i))
            .collect();

        self.rows = rows;
        self.selectable = selectable;
        self.cursor = 0;
        self.scroll_offset = 0;
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        let dialog_width = 72u16;
        let dialog_height = (self.rows.len() + 4).clamp(6, 20) as u16;
        let dialog_area = centered_rect(dialog_width, dialog_height, area);

        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Select Model ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));

        let inner_area = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        // Search bar
        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.primary)),
            Span::styled(&self.query, Style::default().fg(theme.text)),
            Span::styled("▏", Style::default().fg(theme.primary)),
        ]);
        surface.render_widget(
            Paragraph::new(search_line),
            Rect {
                x: inner_area.x,
                y: inner_area.y,
                width: inner_area.width,
                height: 1,
            },
        );

        // List area
        let list_area = Rect {
            x: inner_area.x,
            y: inner_area.y + 2,
            width: inner_area.width.saturating_sub(1), // reserve 1 for scrollbar
            height: inner_area.height.saturating_sub(2),
        };
        if list_area.height == 0 {
            return;
        }

        // Determine which row the cursor points to
        let selected_row = self.selectable.get(self.cursor).copied();

        // Auto-scroll so the selected row is visible
        let scroll = {
            let vis_height = list_area.height as usize;
            let mut s = self.scroll_offset;
            if let Some(sel) = selected_row {
                if sel < s {
                    s = sel;
                } else if sel >= s + vis_height {
                    s = sel.saturating_sub(vis_height.saturating_sub(1));
                }
            }
            s
        };

        let visible_rows = self
            .rows
            .iter()
            .enumerate()
            .skip(scroll)
            .take(list_area.height as usize);
        let content_width = list_area.width as usize;

        for (row_idx, (abs_idx, row)) in visible_rows.enumerate() {
            let y = list_area.y + row_idx as u16;
            let row_area = Rect {
                x: list_area.x,
                y,
                width: list_area.width,
                height: 1,
            };

            match row {
                Row::Header(label) => {
                    let line = Line::from(Span::styled(
                        format!(" {}", label),
                        Style::default()
                            .fg(theme.text_muted)
                            .add_modifier(Modifier::BOLD),
                    ));
                    surface.render_widget(Paragraph::new(line), row_area);
                }
                Row::Model { index, is_recent } => {
                    let m = &self.models[*index];
                    let is_selected = selected_row == Some(abs_idx);
                    let model_key = format!("{}/{}", m.provider, m.id);
                    let is_current = self.current_model.as_deref() == Some(model_key.as_str());

                    let bg = if is_selected {
                        theme.background_element
                    } else {
                        theme.background_panel
                    };
                    let base = Style::default().bg(bg);

                    let check = if is_current { "✓ " } else { "  " };
                    let ctx_str = format_context_window(m.context_window);
                    let label = if *is_recent {
                        format!("{}/{}", m.provider, m.id)
                    } else {
                        m.name.clone()
                    };
                    let reserved_width = check.len() + ctx_str.len() + 1;
                    let max_label_width = content_width.saturating_sub(reserved_width);
                    let visible_label = truncate_text_to_width(&label, max_label_width);

                    let name_width = UnicodeWidthStr::width(visible_label.as_str()) + check.len();
                    let ctx_width = UnicodeWidthStr::width(ctx_str.as_str());
                    let padding = content_width.saturating_sub(name_width + ctx_width + 1);

                    let line = Line::from(vec![
                        Span::styled(check, base.fg(theme.success)),
                        Span::styled(
                            visible_label,
                            base.fg(if is_current {
                                theme.primary
                            } else {
                                theme.text
                            }),
                        ),
                        Span::styled(" ".repeat(padding), base),
                        Span::styled(ctx_str, base.fg(theme.text_muted)),
                    ]);
                    surface.render_widget(Paragraph::new(line), row_area);
                }
            }
        }

        // Scrollbar
        if self.rows.len() > list_area.height as usize {
            let scroll_area = Rect {
                x: list_area.x + list_area.width,
                y: list_area.y,
                width: 1,
                height: list_area.height,
            };
            let mut sb_state = ScrollbarState::new(self.rows.len())
                .position(scroll)
                .viewport_content_length(list_area.height as usize);
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::default().fg(theme.border_subtle))
                .thumb_symbol("█")
                .thumb_style(Style::default().fg(theme.primary));
            surface.render_stateful_widget(sb, scroll_area, &mut sb_state);
        }
    }
}

impl Default for ModelSelectDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

fn format_context_window(ctx: u64) -> String {
    if ctx >= 1_000_000 {
        format!("{}M", ctx / 1_000_000)
    } else if ctx >= 1_000 {
        format!("{}K", ctx / 1_000)
    } else if ctx > 0 {
        format!("{}", ctx)
    } else {
        String::new()
    }
}

fn truncate_text_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width.saturating_add(ch_width).saturating_add(1) > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn model_select_dialog_renders_to_buffer_surface() {
        let mut dialog = ModelSelectDialog::new();
        dialog.set_models(vec![
            Model {
                id: "gpt-5".to_string(),
                name: "GPT-5".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
            },
            Model {
                id: "gpt-5-mini".to_string(),
                name: "GPT-5 Mini".to_string(),
                provider: "openai".to_string(),
                context_window: 64_000,
            },
        ]);
        dialog.set_current_model(Some("openai/gpt-5".to_string()));
        dialog.open();

        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }

    #[test]
    fn recent_rows_render_provider_and_model_id() {
        let mut dialog = ModelSelectDialog::new();
        dialog.set_models(vec![
            Model {
                id: "gpt-4.1".to_string(),
                name: "GPT-4.1".to_string(),
                provider: "openai".to_string(),
                context_window: 128_000,
            },
            Model {
                id: "deepseek-v4-flash".to_string(),
                name: "DeepSeek V4 Flash".to_string(),
                provider: "deepseek".to_string(),
                context_window: 64_000,
            },
        ]);
        dialog.set_recent(vec![("openai".to_string(), "gpt-4.1".to_string())]);
        dialog.open();

        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("");

        assert!(rendered.contains("Recent"));
        assert!(rendered.contains("openai/gpt-4.1"));
    }
}
