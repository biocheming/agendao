//! 金 — Alert dialog: simple modal with title + message.

use revue::prelude::*;
use revue::event::Key;

/// Alert dialog state.
pub struct AlertDialog {
    pub title: String,
    pub message: String,
    pub visible: bool,
}

impl AlertDialog {
    pub fn new() -> Self {
        Self { title: String::new(), message: String::new(), visible: false }
    }

    pub fn show(&mut self, title: &str, message: &str) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.visible = true;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    /// Handle key events. Returns true if key was consumed.
    pub fn handle_key(&mut self, key: &Key) -> bool {
        if !self.visible {
            return false;
        }
        match key {
            Key::Enter | Key::Escape | Key::Char(' ') | Key::Char('q') => {
                self.dismiss();
                true
            }
            _ => true, // consume all keys while alert is visible
        }
    }

    /// Render the alert dialog centered on screen.
    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible {
            return;
        }

        let area = ctx.area;
        let w = (area.width / 2).max(40).min(area.width - 4);
        let h = (self.message.lines().count() as u16 + 6).min(area.height - 4);
        let x = (area.width - w) / 2;
        let y = (area.height - h) / 2;

        // Backdrop
        for cy in 0..area.height {
            for cx in 0..area.width {
                ctx.draw_text(cx, cy, " ", Color::rgb(10, 10, 20));
            }
        }

        // Dialog box
        let _bg = Color::rgb(36, 40, 59);
        let border = Color::rgb(86, 95, 137);
        for cy in y..y + h {
            for cx in x..x + w {
                let ch = if cy == y || cy == y + h - 1 { "─" }
                    else if cx == x || cx == x + w - 1 { "│" }
                    else { " " };
                ctx.draw_text(cx, cy, ch, border);
            }
        }

        // Title
        let title = format!(" {} ", self.title);
        let tx = x + (w - title.len() as u16) / 2;
        ctx.draw_text(tx, y, &title, Color::rgb(247, 118, 142));

        // Message
        for (i, line) in self.message.lines().enumerate() {
            let msg_y = y + 3 + i as u16;
            let lx = x + 2;
            if msg_y < y + h - 2 {
                ctx.draw_text(lx, msg_y, line, Color::rgb(169, 177, 214));
            }
        }

        // Dismiss hint
        let hint = " Press Enter/Esc/Space to dismiss ";
        let hx = x + (w - hint.len() as u16) / 2;
        let hy = y + h - 2;
        ctx.draw_text(hx, hy, hint, Color::rgb(86, 95, 137));
    }
}
