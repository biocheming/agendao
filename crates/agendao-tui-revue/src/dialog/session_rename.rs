//! 金 — Session rename dialog: inline text input for new title.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct SessionRenameDialog {
    pub visible: bool,
    pub session_id: String,
    pub current_title: String,
    input: revue::widget::Input,
}

impl SessionRenameDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            session_id: String::new(),
            current_title: String::new(),
            input: revue::widget::Input::new().placeholder("New session name..."),
        }
    }

    pub fn open(&mut self, session_id: &str, current_title: &str) {
        self.session_id = session_id.to_string();
        self.current_title = current_title.to_string();
        self.input = revue::widget::Input::new()
            .placeholder("New session name...")
            .value(current_title);
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<(String, String)> {
        if !self.visible { return None; }
        match key {
            Key::Enter => {
                let new_title = self.input.text().trim().to_string();
                if !new_title.is_empty() && new_title != self.current_title {
                    let sid = self.session_id.clone();
                    let title = new_title;
                    self.close();
                    return Some((sid, title));
                }
                self.close();
                None
            }
            Key::Escape => { self.close(); None }
            _ => { self.input.handle_key(key); None }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let content = vstack().gap(1)
            .child(Text::new("Rename session:").bold().fg(colors::ACCENT_CYAN))
            .child(Border::rounded().fg(colors::BORDER).child(self.input.clone()));

        backdrop::render_dialog(
            "Rename Session",
            colors::ACCENT_CYAN,
            content,
            "Enter: confirm  Esc: cancel",
            ctx, 50, 6,
        );
    }
}
