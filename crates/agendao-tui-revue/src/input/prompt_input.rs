//! 木 — PromptInput: single authority for all user text input.
//!
//! Wraps Revue's `Input` widget. Handles:
//!   - Text input (short/medium/long via wrapping)
//!   - Paste (Revue delegates to crossterm bracketed-paste)
//!   - History navigation (Up/Down)
//!   - Slash-command autocomplete (/)
//!   - Submit on Enter

use revue::prelude::*;
use revue::event::Key;

/// Action produced by the prompt input.
#[derive(Clone, Debug)]
pub enum PromptAction {
    None,
    Submit(String),
}

/// Complete prompt input component wrapping Revue's Input.
pub struct PromptInput {
    input: revue::widget::Input,
    focused: bool,

    // History
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,

    // Slash autocomplete
    slash_cmds: Vec<String>,
    slash_visible: bool,
    slash_sel: usize,
}

const SLASH_COMMANDS: &[&str] = &[
    "help", "clear", "model", "agent", "theme",
    "session", "list", "export", "rename", "delete",
    "mcp", "lsp", "sidebar", "quit",
];

impl PromptInput {
    pub fn new() -> Self {
        Self {
            input: revue::widget::Input::new()
                .placeholder("Ask anything..."),
            focused: false,
            history: Vec::new(),
            history_idx: None,
            draft: None,
            slash_cmds: SLASH_COMMANDS.iter().map(|s| s.to_string()).collect(),
            slash_visible: false,
            slash_sel: 0,
        }
    }

    /// Handle a key event. Returns a PromptAction.
    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        match key {
            // ── Submit ──
            Key::Enter => {
                if self.slash_visible && !self.filtered_cmds().is_empty() {
                    let sel = self.slash_sel.min(self.filtered_cmds().len().saturating_sub(1));
                    if let Some(cmd) = self.filtered_cmds().get(sel) {
                        self.input = revue::widget::Input::new()
                            .placeholder(&format!("/{} ", cmd));
                        self.slash_visible = false;
                        return PromptAction::None;
                    }
                }
                let text = self.text().trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone());
                    self.history_idx = None;
                    self.draft = None;
                    self.clear();
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }

            // ── History: Up ──
            Key::Up => {
                if self.history.is_empty() { return PromptAction::None; }
                if self.history_idx.is_none() {
                    self.draft = Some(self.text());
                    self.history_idx = Some(self.history.len().saturating_sub(1));
                } else if let Some(idx) = self.history_idx {
                    if idx > 0 { self.history_idx = Some(idx - 1); }
                }
                if let Some(idx) = self.history_idx {
                    if let Some(entry) = self.history.get(idx) {
                        self.input = revue::widget::Input::new()
                            .placeholder(entry);
                    }
                }
                PromptAction::None
            }

            // ── History: Down ──
            Key::Down => {
                if self.history_idx.is_none() { return PromptAction::None; }
                match self.history_idx {
                    Some(idx) if idx + 1 < self.history.len() => {
                        self.history_idx = Some(idx + 1);
                        if let Some(entry) = self.history.get(idx + 1) {
                            self.input = revue::widget::Input::new().placeholder(entry);
                        }
                    }
                    _ => {
                        self.history_idx = None;
                        let draft = self.draft.take().unwrap_or_default();
                        self.input = revue::widget::Input::new().placeholder(&draft);
                    }
                }
                PromptAction::None
            }

            // ── Slash trigger ──
            Key::Char('/') => {
                self.input.handle_key(key);
                self.slash_sel = 0;
                PromptAction::None
            }

            // ── Slash navigation ──
            Key::Tab if self.text().starts_with('/') => {
                self.slash_visible = true;
                let cmds = self.filtered_cmds();
                self.slash_sel = (self.slash_sel + 1) % cmds.len().max(1);
                PromptAction::None
            }

            // ── Escape: cancel slash or blur ──
            Key::Escape => {
                if self.slash_visible {
                    self.slash_visible = false;
                    self.clear();
                    return PromptAction::None;
                }
                self.focused = false;
                PromptAction::None
            }

            // ── Any other key activates input ──
            _ => {
                self.focused = true;
                if self.text().starts_with('/') {
                    self.slash_visible = true;
                }
                self.input.handle_key(key);
                PromptAction::None
            }
        }
    }

    /// Get current text content.
    pub fn text(&self) -> String {
        self.input.text().trim().to_string()
    }

    /// Clear the input.
    pub fn clear(&mut self) {
        self.input = revue::widget::Input::new()
            .placeholder("Ask anything...");
        self.slash_visible = false;
        self.focused = false;
    }

    /// Whether the input is focused.
    pub fn is_focused(&self) -> bool { self.focused }

    /// Filtered slash commands matching current input.
    pub fn filtered_cmds(&self) -> Vec<&String> {
        let query = self.text().trim_start_matches('/').to_lowercase();
        if query.is_empty() {
            self.slash_cmds.iter().collect()
        } else {
            self.slash_cmds.iter()
                .filter(|c| c.to_lowercase().contains(&query))
                .collect()
        }
    }

    pub fn slash_visible(&self) -> bool { self.slash_visible }
    pub fn slash_selected(&self) -> usize { self.slash_sel }

    /// Check if the prompt can be submitted.
    pub fn can_submit(&self) -> bool {
        !self.text().trim().is_empty()
    }

    /// Status hint text for the bar above the input.
    pub fn status_hint(&self, is_running: bool) -> String {
        if is_running { return "Running... Esc: stop".into(); }
        if self.focused {
            let lines = self.text().lines().count();
            if lines > 1 { format!("{} lines | Enter: send | Esc: cancel", lines) }
            else if !self.text().is_empty() { format!("{} chars | Enter: send", self.text().len()) }
            else { "Type... | Enter: send".into() }
        } else {
            "Type to start...".into()
        }
    }

    /// Render the input widget.
    pub fn render(&self, ctx: &mut RenderContext) {
        self.input.render(ctx);
    }
}
