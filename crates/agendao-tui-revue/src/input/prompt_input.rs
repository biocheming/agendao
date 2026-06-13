//! 木 — PromptInput: single authority for all user text input.
//!
//! Manages text directly (String) rather than recreating Revue Input widget.
//! Key handling: append/backspace/Enter/history navigation.

use revue::prelude::*;
use revue::event::Key;

#[derive(Clone, Debug)]
pub enum PromptAction { None, Submit(String), SubmitShell(String) }

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode { Normal, Shell }

pub struct PromptInput {
    text: String,
    cursor: usize,
    mode: InputMode,
    focused: bool,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
}

impl PromptInput {
    pub fn new() -> Self {
        Self { text: String::new(), cursor: 0, mode: InputMode::Normal, focused: false, history: Vec::new(), history_idx: None, draft: None }
    }

    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        // Shell mode toggle
        if let Key::Char('!') = key {
            if self.text.is_empty() { self.mode = InputMode::Shell; self.focused = true; return PromptAction::None; }
        }
        if matches!(key, Key::Escape) && self.mode == InputMode::Shell {
            self.mode = InputMode::Normal; self.clear(); return PromptAction::None;
        }

        match key {
            Key::Enter => {
                if self.mode == InputMode::Shell {
                    let cmd = self.text.trim().to_string();
                    if !cmd.is_empty() { self.history.push(cmd.clone()); self.history_idx = None; self.draft = None; self.clear(); self.mode = InputMode::Normal; return PromptAction::SubmitShell(cmd); }
                }
                let text = self.text.trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone()); self.history_idx = None; self.draft = None; self.clear();
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }
            Key::Char(c) => {
                self.focused = true;
                self.insert_char(*c);
                PromptAction::None
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.text.remove(self.cursor);
                }
                PromptAction::None
            }
            Key::Delete => {
                if self.cursor < self.text.len() {
                    self.text.remove(self.cursor);
                }
                PromptAction::None
            }
            Key::Left => { if self.cursor > 0 { self.cursor -= 1; } PromptAction::None }
            Key::Right => { if self.cursor < self.text.len() { self.cursor += 1; } PromptAction::None }
            Key::Home => { self.cursor = 0; PromptAction::None }
            Key::End => { self.cursor = self.text.len(); PromptAction::None }
            Key::Up => self.history_up(),
            Key::Down => self.history_down(),
            _ => PromptAction::None,
        }
    }

    fn history_up(&mut self) -> PromptAction {
        if self.history.is_empty() { return PromptAction::None; }
        if self.history_idx.is_none() { self.draft = Some(self.text.clone()); self.history_idx = Some(self.history.len().saturating_sub(1)); }
        else if let Some(idx) = self.history_idx { if idx > 0 { self.history_idx = Some(idx - 1); } }
        if let Some(idx) = self.history_idx {
            if let Some(entry) = self.history.get(idx) { self.text = entry.clone(); self.cursor = self.text.len(); }
        }
        PromptAction::None
    }

    fn history_down(&mut self) -> PromptAction {
        if self.history_idx.is_none() { return PromptAction::None; }
        if let Some(idx) = self.history_idx {
            if idx + 1 < self.history.len() {
                self.history_idx = Some(idx + 1);
                if let Some(entry) = self.history.get(idx + 1) { self.text = entry.clone(); self.cursor = self.text.len(); }
            } else {
                self.history_idx = None;
                self.text = self.draft.take().unwrap_or_default();
                self.cursor = self.text.len();
            }
        }
        PromptAction::None
    }

    fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn text(&self) -> &str { &self.text }
    pub fn set_text(&mut self, t: &str) { self.text = t.to_string(); self.cursor = self.text.len(); self.focused = true; }
    pub fn clear(&mut self) { self.text.clear(); self.cursor = 0; self.focused = false; }
    pub fn is_focused(&self) -> bool { self.focused }
    pub fn mode(&self) -> &InputMode { &self.mode }

    /// Show status hint above the prompt bar.
    pub fn status_hint(&self, is_running: bool) -> String {
        if is_running { return "Running... Esc: stop".into(); }
        if self.focused {
            if self.text.len() > 40 { format!("{} chars | Enter: send", self.text.len()) }
            else if !self.text.is_empty() { format!("{} chars | Enter: send", self.text.len()) }
            else { "Type to start... | Enter: send".into() }
        } else { "Type to start...".into() }
    }

    /// Render the prompt text with cursor at the bottom.
    pub fn render_prompt(&self, ctx: &mut RenderContext, y: u16) {
        let before = if self.cursor <= self.text.len() { &self.text[..self.cursor] } else { &self.text };
        let after = if self.cursor < self.text.len() { &self.text[self.cursor..] } else { "" };
        let display = format!("> {}{}{}", before, if self.focused { "█" } else { "" }, after);
        ctx.draw_text(0, y, &display, Color::rgb(169, 177, 214));
        if self.focused {
            // Dim placeholder if text is empty
            if self.text.is_empty() {
                let ph = if self.mode == InputMode::Shell { "Run a command... \"ls -la\"" } else { "Ask anything..." };
                ctx.draw_text(2, y, ph, Color::rgb(86, 95, 137));
            }
        }
    }
}
