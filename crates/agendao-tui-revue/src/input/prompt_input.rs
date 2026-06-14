//! 木 — PromptInput: single authority for all user text input.

use revue::event::Key;

#[derive(Clone, Debug)]
pub enum PromptAction { None, Consumed, Submit(String), SubmitShell(String) }

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode { Normal, Shell }

pub struct PromptInput {
    input: revue::widget::Input,
    mode: InputMode,
    focused: bool,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
    normal_placeholders: Vec<String>,
    shell_placeholders: Vec<String>,
    /// Optional path for persisting history to disk.
    history_path: Option<std::path::PathBuf>,
}

fn default_history_path() -> std::path::PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("agendao").join("prompt-history.json")
}

fn load_history(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(path: &std::path::Path, history: &[String]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(history) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true)
                .mode(0o600)
                .open(path)
            {
                let _ = f.write_all(json.as_bytes());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::write(path, &json);
        }
    }
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
            normal_placeholders: vec!["Ask anything...".into()],
            shell_placeholders: vec!["Run a command...".into()],
            history_path: None,
        }
    }

    /// Load history from default path.
    pub fn with_persistence(mut self) -> Self {
        let path = default_history_path();
        self.history = load_history(&path);
        self.history_path = Some(path);
        self
    }

    pub fn with_placeholders(mut self, normal: &[&str], shell: &[&str]) -> Self {
        self.normal_placeholders = normal.iter().map(|s| s.to_string()).collect();
        self.shell_placeholders = shell.iter().map(|s| s.to_string()).collect();
        // Pick a random one
        let idx = (std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as usize)
            % self.normal_placeholders.len();
        let placeholder = &self.normal_placeholders[idx];
        self.input = revue::widget::Input::new().placeholder(placeholder);
        self
    }

    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        // Shell mode toggle
        if let Key::Char('!') = key {
            if self.input.text().trim().is_empty() {
                self.mode = InputMode::Shell;
                self.focused = true;
                let placeholder = self.shell_placeholders.first().map(|s| s.as_str()).unwrap_or("Run a command...");
                self.input = revue::widget::Input::new().placeholder(placeholder);
                return PromptAction::None;
            }
        }
        if matches!(key, Key::Escape) && self.mode == InputMode::Shell {
            self.mode = InputMode::Normal;
            self.input.clear();
            let placeholder = self.normal_placeholders.first().map(|s| s.as_str()).unwrap_or("Ask anything...");
            self.input = revue::widget::Input::new().placeholder(placeholder);
            self.focused = false;
            return PromptAction::None;
        }

        match key {
            Key::Enter => {
                let text = self.input.text().trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone());
                    if let Some(ref path) = self.history_path {
                        save_history(path, &self.history);
                    }
                    self.history_idx = None;
                    self.draft = None;
                    self.input.clear();
                    self.focused = false;
                    if self.mode == InputMode::Shell {
                        self.mode = InputMode::Normal;
                        let placeholder = self.normal_placeholders.first().map(|s| s.as_str()).unwrap_or("Ask anything...");
                        self.input = revue::widget::Input::new().placeholder(placeholder);
                        return PromptAction::SubmitShell(text);
                    }
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }
            Key::Up => { self.history_up(); PromptAction::Consumed }
            Key::Down => { self.history_down(); PromptAction::Consumed }
            Key::Left | Key::Right | Key::Home | Key::End => {
                self.focused = true;
                self.input.handle_key(key);
                PromptAction::Consumed
            }
            _ => {
                self.focused = true;
                let changed = self.input.handle_key(key);
                if changed { PromptAction::Consumed } else { PromptAction::None }
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

    /// Handle a mouse click at (x, y). The prompt bar occupies the bottom ~5 lines.
    /// Returns true if the click was on the prompt area.
    pub fn handle_click(&mut self, _x: u16, y: u16) -> bool {
        // Prompt bar is near the bottom: hint(1) + border(2~3) + status(1) ≈ 5 lines
        // Use a heuristic: if y is at least 5 lines from the bottom of a 40-line terminal
        if y >= 35 {
            self.focused = true;
            true
        } else {
            self.focused = false;
            false
        }
    }
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
