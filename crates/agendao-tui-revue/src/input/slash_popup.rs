//! 木 — Slash command popup: / triggered command palette.
//!
//! Uses agendao_command::CommandRegistry for real slash commands
//! with fuzzy matching, keyboard navigation, and declarative Revue layout.

use agendao_command::{CommandRegistry, UiActionId, UiCommandSpec};
use revue::prelude::*;
use revue::event::Key;

/// Simple fuzzy match: check if all chars of `query` appear in `target` in order.
pub(crate) fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return Some(0); }
    let t = target.to_lowercase();
    let mut qi = q.chars();
    let mut current = qi.next();
    let mut score = 0i32;
    for (i, tc) in t.chars().enumerate() {
        if let Some(qc) = current {
            if qc == tc { score += 100 - (i as i32).min(50); current = qi.next(); }
        } else { break; }
    }
    if current.is_none() { Some(score) } else { None }
}

pub struct SlashPopup {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    /// All slash commands from the registry
    all_commands: Vec<UiCommandSpec>,
    /// Filtered indices into all_commands
    filtered: Vec<usize>,
    selected_action: Option<UiActionId>,
}

impl SlashPopup {
    pub fn new() -> Self {
        Self::with_dir(None)
    }

    /// Optionally load custom commands from a project directory.
    pub fn with_dir(dir: Option<&std::path::Path>) -> Self {
        let mut registry = CommandRegistry::new();
        if let Some(d) = dir {
            let _ = registry.load_from_directory(d);
        }
        let all_commands: Vec<UiCommandSpec> = registry
            .ui_all_slash_commands()
            .into_iter()
            .cloned()
            .collect();
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            all_commands,
            filtered: Vec::new(),
            selected_action: None,
        }
    }

    pub fn open(&mut self) {
        self.visible = true; self.selected = 0; self.query.clear();
        self.refresh_filter();
    }

    pub fn open_with_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.refresh_filter();
        self.visible = true;
        self.selected_action = None;
    }

    pub fn close(&mut self) {
        self.visible = false; self.query.clear(); self.filtered.clear();
        self.selected = 0; self.selected_action = None;
    }

    pub fn is_open(&self) -> bool { self.visible }

    pub fn take_action(&mut self) -> Option<UiActionId> {
        self.selected_action.take()
    }

    /// Detect if the current prompt text contains a slash token.
    /// Returns the text after `/` (the query) if a slash command is detected.
    pub fn slash_token(text: &str) -> Option<String> {
        text.split_whitespace()
            .last()
            .filter(|token| token.starts_with('/'))
            .map(|token| token.trim_start_matches('/').to_string())
            .filter(|token| !token.is_empty())
    }

    /// Number of filtered results (for sizing the popup).
    pub fn filtered_count(&self) -> usize { self.filtered.len() }

    /// Push a character to the filter query.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refresh_filter();
    }

    /// Pop last character from the filter query.
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refresh_filter();
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<UiActionId> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Enter => {
                if let Some(idx) = self.filtered.get(self.selected) {
                    let action_id = self.all_commands[*idx].action_id;
                    // close() clears selected_action — remember the action
                    // BEFORE close(), then return it directly. Calling
                    // self.take_action() after close() always yields None.
                    self.close();
                    return Some(action_id);
                }
                self.take_action()
            }
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Key::Down => {
                let max = self.filtered.len().saturating_sub(1);
                if self.selected < max { self.selected += 1; }
                None
            }
            Key::Backspace => { self.pop_char(); None }
            Key::Char(c) if c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_' => {
                self.push_char(*c); None
            }
            _ => None,
        }
    }

    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            // Show suggested commands from the registry
            let registry = CommandRegistry::new();
            let suggested: Vec<UiActionId> = registry
                .ui_suggested_slash_commands()
                .into_iter()
                .map(|cmd| cmd.action_id)
                .collect();
            self.filtered = self.all_commands.iter().enumerate()
                .filter(|(_, cmd)| suggested.contains(&cmd.action_id))
                .map(|(i, _)| i)
                .collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self.all_commands.iter().enumerate()
                .filter_map(|(i, cmd)| {
                    let slash = cmd.slash.as_ref()?;
                    let name_score = fuzzy_match(&self.query, slash.name);
                    let alias_score = slash.aliases.iter()
                        .filter_map(|alias| fuzzy_match(&self.query, alias))
                        .max();
                    let title_score = fuzzy_match(&self.query, cmd.title);
                    let best = name_score.into_iter().chain(alias_score).chain(title_score).max()?;
                    Some((i, best))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = 0;
    }

    /// Render popup above the prompt via Revue Border + vstack.
    pub fn render_popup(&self) -> impl View {
        let mut stack = vstack();
        if !self.visible || self.filtered.is_empty() {
            return stack;
        }

        let max_visible = 10usize.min(self.filtered.len());
        let mut list = vstack().gap(0);
        let mut last_category: Option<&str> = None;

        for (row_idx, &cmd_idx) in self.filtered.iter().enumerate().take(max_visible) {
            let cmd = &self.all_commands[cmd_idx];
            let is_selected = row_idx == self.selected;

            // Show category separator when category changes
            let cat = cmd.category.label();
            if last_category.map(|c| c != cat).unwrap_or(true) {
                if last_category.is_some() {
                    list = list.child(Text::new(""));
                }
                list = list.child(Text::new(&format!(" {}:", cat)).fg(Color::rgb(137, 180, 250)));
                last_category = Some(cat);
            }

            let slash_name = cmd.slash.as_ref()
                .map(|s| s.name)
                .unwrap_or(cmd.title);

            let marker = if is_selected { "▶" } else { " " };
            let keybind_str = cmd.keybind.map(|k| format!(" ({})", k)).unwrap_or_default();
            let desc = format!("{} /{}{}  {}", marker, slash_name, keybind_str, cmd.description);

            let mut text = Text::new(&desc);
            if is_selected {
                text = text.fg(Color::rgb(125, 207, 255)).bg(Color::rgb(40, 42, 54));
            } else {
                text = text.fg(Color::rgb(169, 177, 214));
            }
            list = list.child(text);
        }

        if self.filtered.len() > max_visible {
            list = list.child(Text::new(
                format!("  ... and {} more", self.filtered.len() - max_visible)
            ).fg(Color::rgb(100, 100, 120)));
        }

        let border = Border::rounded()
            .title(format!(" /{} ({} results) ", self.query, self.filtered.len()))
            .fg(Color::rgb(125, 207, 255))
            .child(list);

        stack = stack.child(border);
        stack
    }
}
