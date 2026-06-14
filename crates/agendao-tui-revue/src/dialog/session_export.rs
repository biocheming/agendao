//! 金 — Session export dialog: copy/share transcript as text.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct SessionExportDialog {
    pub visible: bool,
    pub session_id: String,
    pub messages_text: String,
    pub copied: bool,
}

impl SessionExportDialog {
    pub fn new() -> Self {
        Self { visible: false, session_id: String::new(), messages_text: String::new(), copied: false }
    }

    pub fn open(&mut self, session_id: &str, messages: &str) {
        self.session_id = session_id.to_string();
        self.messages_text = messages.to_string();
        self.copied = false;
        self.visible = true;
    }

    pub fn close(&mut self) { self.visible = false; self.copied = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<ExportAction> {
        if !self.visible { return None; }
        match key {
            Key::Escape => { self.close(); None }
            Key::Char('c') => {
                self.copied = true;
                Some(ExportAction::Copy(self.messages_text.clone()))
            }
            Key::Char('s') => {
                Some(ExportAction::Share(self.session_id.clone()))
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let msg_count = self.messages_text.lines().count();
        let mut content = vstack().gap(1)
            .child(Text::new(&format!("{} messages in this session", msg_count))
                .fg(colors::FG_MUTED));

        if self.copied {
            content = content.child(Text::new("✓ Copied to clipboard!").fg(colors::ACCENT_GREEN));
        } else {
            content = content
                .child(Text::new("c: copy to clipboard").fg(colors::ACCENT_CYAN))
                .child(Text::new("s: share as link").fg(colors::ACCENT_CYAN));
        }

        backdrop::render_dialog(
            "Export Session",
            colors::ACCENT_CYAN,
            content,
            "Esc: close",
            ctx, 44, 8,
        );
    }
}

#[derive(Clone, Debug)]
pub enum ExportAction {
    Copy(String),
    Share(String),
}
