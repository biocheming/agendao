//! 金 — Alert dialog.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct AlertDialog {
    pub title: String,
    pub message: String,
    pub visible: bool,
}

impl AlertDialog {
    pub fn new() -> Self { Self { title: String::new(), message: String::new(), visible: false } }

    pub fn show(&mut self, title: &str, message: &str) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.visible = true;
    }

    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        match key {
            Key::Enter | Key::Escape | Key::Char(' ') | Key::Char('q') => { self.dismiss(); true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        let mut content = vstack().gap(1);
        for line in self.message.lines() {
            content = content.child(Text::new(line).fg(colors::FG_SECONDARY));
        }

        backdrop::render_dialog(
            &self.title,
            colors::ACCENT_RED,
            content,
            "Enter/Esc/Space to dismiss",
            ctx,
            (ctx.area.width / 2).max(40),
            self.message.lines().count() as u16 + 5,
        );
    }
}
