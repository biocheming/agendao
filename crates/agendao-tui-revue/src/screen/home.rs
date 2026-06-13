//! 木+金 — Home Screen.
//!
//! Logo from agendao_command_render::branding (same as old TUI).

use revue::prelude::*;
use crate::store::app_store::AppStore;

pub struct HomeScreen<'a> { pub store: &'a AppStore }

impl<'a> View for HomeScreen<'a> {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        let lines = agendao_command_render::branding::logo_lines("  ");
        let logo_h = lines.len() as u16;
        let content_h = logo_h + 5;
        let top_padding = area.y + area.height.saturating_sub(content_h) / 2;
        let mut y = top_padding;

        for line in &lines {
            ctx.draw_text(2, y, line, Color::rgb(189, 147, 249));
            y += 1;
        }
        y += 2;

        let hint = "Type below and press Enter to start";
        let width = area.width as usize;
        ctx.draw_text(width.saturating_sub(hint.len()).saturating_div(2) as u16, y, hint, Color::rgb(150, 150, 170));
        y += 2;

        for (key, desc) in [("Enter", "Start a new session"), ("h", "Home"), ("q/Esc", "Quit")] {
            ctx.draw_text(2, y, &format!("  {:<6}", key), Color::rgb(125, 207, 255));
            ctx.draw_text(10, y, desc, Color::rgb(169, 177, 214));
            y += 1;
        }
    }
}
