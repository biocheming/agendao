//! 金 — Session fork dialog: fork from a specific message.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct SessionForkDialog {
    pub visible: bool,
    pub session_id: String,
    pub message_id: Option<String>,
}

impl SessionForkDialog {
    pub fn new() -> Self {
        Self { visible: false, session_id: String::new(), message_id: None }
    }

    pub fn open(&mut self, session_id: &str, message_id: Option<&str>) {
        self.session_id = session_id.to_string();
        self.message_id = message_id.map(|s| s.to_string());
        self.visible = true;
    }

    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<(String, Option<String>)> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Enter => {
                let sid = self.session_id.clone();
                let mid = self.message_id.clone();
                self.close();
                Some((sid, mid))
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let content = vstack().child(
            Text::new(&format!("Fork from: {}", self.message_id.as_deref().unwrap_or("(latest)")))
                .fg(colors::FG_SECONDARY)
        );
        backdrop::render_dialog(
            "Fork Session",
            colors::ACCENT_PURPLE,
            content,
            "Enter: fork  Esc: cancel",
            ctx, 50, 5,
        );
    }
}
