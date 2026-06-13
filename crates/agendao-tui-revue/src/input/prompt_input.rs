//! 木 — PromptInput: single authority for all user text input.

use revue::event::Key;

#[derive(Clone, Debug)]
pub enum PromptAction { None, Submit(String), SubmitShell(String) }

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode { Normal, Shell }

pub struct PromptInput {
    input: revue::widget::Input,
    mode: InputMode,
    focused: bool,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
}

impl PromptInput {
    pub fn new() -> Self {
        Self {
            input: revue::widget::Input::new().placeholder("Ask anything..."),
            mode: InputMode::Normal,
            focused: false,
            history: Vec::new(),
            history_idx: None,
            draft: None,
        }
    }

    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        // Shell mode toggle
        if let Key::Char('!') = key {
            if self.input.text().trim().is_empty() {
                self.mode = InputMode::Shell;
                self.focused = true;
                self.input = revue::widget::Input::new().placeholder("Run a command...");
                return PromptAction::None;
            }
        }
        if matches!(key, Key::Escape) && self.mode == InputMode::Shell {
            self.mode = InputMode::Normal;
            self.input.clear();
            self.input = revue::widget::Input::new().placeholder("Ask anything...");
            self.focused = false;
            return PromptAction::None;
        }

        match key {
            Key::Enter => {
                let text = self.input.text().trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone());
                    self.history_idx = None;
                    self.draft = None;
                    self.input.clear();
                    self.focused = false;
                    if self.mode == InputMode::Shell {
                        self.mode = InputMode::Normal;
                        self.input = revue::widget::Input::new().placeholder("Ask anything...");
                        return PromptAction::SubmitShell(text);
                    }
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }
            Key::Up => self.history_up(),
            Key::Down => self.history_down(),
            _ => {
                self.focused = true;
                self.input.handle_key(key);
                PromptAction::None
            }
        }
    }

    fn history_up(&mut self) -> PromptAction {
        if self.history.is_empty() { return PromptAction::None; }
        if self.history_idx.is_none() {
            self.draft = Some(self.input.text().to_string());
            self.history_idx = Some(self.history.len().saturating_sub(1));
        } else if let Some(idx) = self.history_idx {
            if idx > 0 { self.history_idx = Some(idx - 1); }
        }
        if let Some(idx) = self.history_idx {
            if let Some(entry) = self.history.get(idx) {
                self.input = revue::widget::Input::new().placeholder("").value(entry);
            }
        }
        PromptAction::None
    }

    fn history_down(&mut self) -> PromptAction {
        if self.history_idx.is_none() { return PromptAction::None; }
        if let Some(idx) = self.history_idx {
            if idx + 1 < self.history.len() {
                self.history_idx = Some(idx + 1);
                if let Some(entry) = self.history.get(idx + 1) {
                    self.input = revue::widget::Input::new().placeholder("").value(entry);
                }
            } else {
                self.history_idx = None;
                let draft = self.draft.take().unwrap_or_default();
                self.input = revue::widget::Input::new()
                    .placeholder(if self.mode == InputMode::Shell { "Run a command..." } else { "Ask anything..." })
                    .value(&draft);
            }
        }
        PromptAction::None
    }

    pub fn text(&self) -> String { self.input.text().to_string() }
    pub fn clear(&mut self) {
        self.input.clear();
        self.focused = false;
    }
    pub fn is_focused(&self) -> bool { self.focused }
    pub fn mode(&self) -> &InputMode { &self.mode }

    /// Show status hint above the prompt bar.
    pub fn status_hint(&self, is_running: bool) -> String {
        if is_running { return "Running... Esc: stop".into(); }
        let len = self.input.text().trim().len();
        if self.focused && len > 0 {
            format!("{} chars | Enter: send", len)
        } else if self.focused {
            "Type to start... | Enter: send".into()
        } else {
            "Click below to type, or just start typing...".into()
        }
    }

    /// Return the Input widget for rendering.
    pub fn widget(&self) -> revue::widget::Input {
        self.input.clone()
    }
}
