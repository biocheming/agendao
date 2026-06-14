//! 金 — Simple confirm/cancel dialog.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct ConfirmDialog {
    pub visible: bool,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub confirmed: bool,
}

impl ConfirmDialog {
    pub fn new() -> Self {
        Self { visible: false, title: String::new(), message: String::new(), confirm_label: "Yes".into(), confirmed: false }
    }

    pub fn ask(&mut self, title: &str, message: &str, confirm_label: &str) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.confirm_label = confirm_label.to_string();
        self.confirmed = false;
        self.visible = true;
    }

    pub fn close(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> Option<bool> {
        if !self.visible { return None; }
        match key {
            Key::Enter | Key::Char('y') => {
                self.confirmed = true;
                self.visible = false;
                Some(true)
            }
            Key::Escape | Key::Char('n') | Key::Char('q') => {
                self.visible = false;
                Some(false)
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let content = vstack().child(
            Text::new(&self.message).fg(colors::FG_SECONDARY)
        );
        let hint = format!("y/Enter: {}  n/Esc: cancel", self.confirm_label);
        backdrop::render_dialog(&self.title, colors::ACCENT_YELLOW, content, &hint, ctx, 48, 5);
    }
}
