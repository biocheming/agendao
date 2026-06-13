//! 木+金 — Home Screen
//!
//! Displays welcome banner and basic interaction guide.

use revue::prelude::*;

use crate::store::app_store::AppStore;

/// Home screen — landing page before entering a session.
pub struct HomeScreen<'a> {
    pub store: &'a AppStore,
}

impl<'a> View for HomeScreen<'a> {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        let width = area.width as usize;

        // Center vertically
        let content_height: u16 = 12;
        let top_padding = area.height.saturating_sub(content_height) / 2;

        let mut y = top_padding;

        // ── Logo / Banner ──
        let logo_lines = [
            "    ╭──────────────────────────────────────────────╮",
            "    │                                              │",
            "    │     █████╗  ██████╗ ███████╗███╗   ██╗       │",
            "    │    ██╔══██╗██╔════╝ ██╔════╝████╗  ██║       │",
            "    │    ███████║██║  ███╗█████╗  ██╔██╗ ██║       │",
            "    │    ██╔══██║██║   ██║██╔══╝  ██║╚██╗██║       │",
            "    │    ██║  ██║╚██████╔╝███████╗██║ ╚████║       │",
            "    │    ╚═╝  ╚═╝ ╚═════╝ ╚══════╝╚═╝  ╚═══╝       │",
            "    │                                              │",
            "    │      道纪 — Canon of Flow & Governance        │",
            "    ╰──────────────────────────────────────────────╯",
        ];

        for line in &logo_lines {
            let line_width = line.chars().count();
            let centered_x = width.saturating_sub(line_width).saturating_div(2) as u16;
            ctx.draw_text(
                centered_x,
                y,
                line,
                Color::rgb(189, 147, 249),
            );
            y += 1;
        }

        y += 2;

        // ── Quick start hint ──
        let hint = "Type your prompt and press Enter to begin";
        let hint_x = width.saturating_sub(hint.len()).saturating_div(2) as u16;
        ctx.draw_text(hint_x, y, hint, Color::rgb(150, 150, 170));
        y += 2;

        // ── Keybindings ──
        let bindings: [(&str, &str); 3] = [
            ("Enter", "Start a new session with your prompt"),
            ("h",     "Return to home screen"),
            ("q/Esc", "Quit agendao"),
        ];

        for (key, desc) in bindings {
            let key_text = format!("  {:<6}", key);
            ctx.draw_text(2, y, &key_text, Color::rgb(125, 207, 255));
            ctx.draw_text(10, y, desc, Color::rgb(169, 177, 214));
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::app_store::AppStore;

    #[test]
    fn home_screen_constructs() {
        let store = AppStore::new();
        let _home = HomeScreen { store: &store };
    }
}
