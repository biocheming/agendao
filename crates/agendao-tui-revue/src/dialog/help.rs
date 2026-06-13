//! 金 — Help dialog: displays keybindings reference.

use revue::prelude::*;
use revue::event::Key;

/// Help dialog state.
pub struct HelpDialog {
    pub visible: bool,
}

impl HelpDialog {
    pub fn new() -> Self {
        Self { visible: false }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible {
            return false;
        }
        match key {
            Key::Escape | Key::Char('q') | Key::Char('h') => {
                self.dismiss();
                true
            }
            _ => true,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible {
            return;
        }

        let area = ctx.area;

        // Semi-transparent backdrop
        for cy in 0..area.height {
            for cx in 0..area.width {
                ctx.draw_text(cx, cy, " ", Color::rgb(10, 10, 20));
            }
        }

        let bindings: &[(&str, &str)] = &[
            ("Enter", "Send prompt / dismiss dialog"),
            ("Esc/q", "Dismiss dialog / quit"),
            ("h", "Go home / toggle help"),
            ("Ctrl+C", "Quit application"),
            ("Tab", "Start editing prompt"),
            ("Type...", "Enter text in the prompt"),
        ];

        let w = 50u16;
        let h = (bindings.len() as u16 + 4).min(area.height - 4);
        let x = (area.width - w) / 2;
        let y = (area.height - h) / 2;

        let _bg = Color::rgb(36, 40, 59);
        let border = Color::rgb(137, 180, 250);
        for cy in y..y + h {
            for cx in x..x + w {
                let ch = if cy == y || cy == y + h - 1 { "─" }
                    else if cx == x || cx == x + w - 1 { "│" }
                    else { " " };
                ctx.draw_text(cx, cy, ch, border);
            }
        }

        // Title
        let title = " Help — Keybindings ";
        let tx = x + (w - title.len() as u16) / 2;
        ctx.draw_text(tx, y, title, Color::rgb(137, 180, 250));

        // Bindings
        let mut row = y + 2;
        for (key, desc) in bindings {
            ctx.draw_text(x + 2, row, &format!("{:<10}", key), Color::rgb(125, 207, 255));
            ctx.draw_text(x + 13, row, desc, Color::rgb(169, 177, 214));
            row += 1;
        }

        let hint = " Press Esc/q/h to close ";
        let hx = x + (w - hint.len() as u16) / 2;
        let hy = y + h - 2;
        ctx.draw_text(hx, hy, hint, Color::rgb(86, 95, 137));
    }
}
