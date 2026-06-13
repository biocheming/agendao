//! 木 — Input authority: the PromptBar.
//!
//! Wraps Revue's `Input` widget with submit-on-Enter behavior.
//! All user text input flows through this single component.

use revue::prelude::*;
use revue::event::Key;

use crate::store::session_store::RunStatus;

/// The prompt input bar at the bottom of the session screen.
pub struct PromptBar {
    /// Revue's built-in text input widget.
    input: revue::widget::Input,
    /// Whether the bar is in editing mode (focused).
    pub editing: bool,
}

impl PromptBar {
    pub fn new() -> Self {
        Self {
            input: revue::widget::Input::new()
                .placeholder("Ask anything..."),
            editing: false,
        }
    }

    /// Handle a key event. Returns `true` if the prompt was submitted.
    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        if !self.editing {
            // Any printable key starts editing
            if let Key::Char(_) = key {
                self.editing = true;
                self.input.clear();
            }
        }

        if self.editing {
            match key {
                Key::Enter => {
                    let text = self.input.text().trim().to_string();
                    if !text.is_empty() {
                        self.input.clear();
                        self.editing = false;
                        return PromptAction::Submit(text);
                    }
                    self.editing = false;
                    return PromptAction::None;
                }
                Key::Escape => {
                    self.editing = false;
                    self.input.clear();
                    return PromptAction::None;
                }
                _ => {
                    self.input.handle_key(key);
                }
            }
        }

        PromptAction::None
    }

    /// Get helper text for the status bar.
    pub fn status_hint(&self, status: &RunStatus) -> String {
        if self.editing {
            "Enter: send | Esc: cancel".into()
        } else {
            match status {
                RunStatus::Idle => "Type to start... | q: quit | h: home".into(),
                RunStatus::Sending => "Sending...".into(),
                RunStatus::Running => "Running... | Esc: stop".into(),
                RunStatus::WaitingUser => "Waiting for your input...".into(),
                RunStatus::Error(_) => "Error — type to retry".into(),
            }
        }
    }

    /// Check if text can be submitted.
    pub fn can_submit(&self) -> bool {
        self.editing && !self.input.text().trim().is_empty()
    }
}

impl View for PromptBar {
    fn render(&self, ctx: &mut RenderContext) {
        // Render the input widget
        self.input.render(ctx);
    }
}

/// Action produced by the PromptBar.
#[derive(Clone, Debug)]
pub enum PromptAction {
    None,
    Submit(String),
}
