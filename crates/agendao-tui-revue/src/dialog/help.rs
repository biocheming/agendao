//! 金 — Help dialog with keybindings.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct HelpDialog {
    pub visible: bool,
}

impl HelpDialog {
    pub fn new() -> Self { Self { visible: false } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }
    pub fn dismiss(&mut self) { self.visible = false; }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible { return false; }
        match key {
            Key::Escape | Key::Char('q') | Key::Char('h') | Key::Char('?') => { self.dismiss(); true }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }

        let bindings: &[(&str, &str)] = &[
            ("Enter", "Send prompt"),
            ("Esc/q", "Quit"),
            ("h", "Home screen"),
            ("?", "Toggle help"),
            ("↑/↓", "Prompt history"),
            ("Tab", "Autocomplete"),
            ("Ctrl+B", "Toggle sidebar"),
            ("Ctrl+P", "Command palette"),
            ("/models", "Switch model"),
            ("/sessions", "Browse sessions"),
            ("Ctrl+C", "Force quit"),
        ];

        let mut content = vstack().gap(0);
        for (key, desc) in bindings {
            content = content.child(
                hstack().gap(2)
                    .child(Text::new(format!("{:>10}", key)).fg(colors::ACCENT_CYAN))
                    .child(Text::new(*desc).fg(colors::FG_SECONDARY))
            );
        }

        backdrop::render_dialog(
            "Help — Keybindings",
            colors::ACCENT_BLUE,
            content,
            "Esc/q/h/? to close",
            ctx, 54, bindings.len() as u16 + 4,
        );
    }
}
