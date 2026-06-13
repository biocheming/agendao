//! 木+金 — Home Screen.
//!
//! Displays welcome banner and keybinding guide.
//! Prompt input is global in RootView (like old TUI).

use revue::prelude::*;
use crate::store::app_store::AppStore;

pub struct HomeScreen<'a> { pub store: &'a AppStore }

impl<'a> View for HomeScreen<'a> {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        let width = area.width as usize;
        let content_height: u16 = 12;
        let top_padding = area.y + area.height.saturating_sub(content_height) / 2;
        let mut y = top_padding;

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
            let lw = line.chars().count();
            ctx.draw_text(width.saturating_sub(lw).saturating_div(2) as u16, y, line, Color::rgb(189, 147, 249));
            y += 1;
        }
        y += 2;
        let hint = "Type below and press Enter to start";
        ctx.draw_text(width.saturating_sub(hint.len()).saturating_div(2) as u16, y, hint, Color::rgb(150, 150, 170));
        y += 2;
        for (key, desc) in [("Enter", "Start a new session"), ("h", "Home"), ("q/Esc", "Quit")] {
            ctx.draw_text(2, y, &format!("  {:<6}", key), Color::rgb(125, 207, 255));
            ctx.draw_text(10, y, desc, Color::rgb(169, 177, 214));
            y += 1;
        }
    }
}
