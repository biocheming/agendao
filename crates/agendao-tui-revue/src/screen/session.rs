//! Session Screen — Phase 1 placeholder.

use revue::prelude::*;

pub struct SessionScreen {
    pub session_id: String,
}

impl View for SessionScreen {
    fn render(&self, ctx: &mut RenderContext) {
        let width = ctx.area.width as usize;
        let msg = format!("Session: {} (placeholder)", self.session_id);
        let x = width.saturating_sub(msg.len()).saturating_div(2) as u16;
        let y = ctx.area.height / 2;
        ctx.draw_text(x, y, &msg, Color::rgb(169, 177, 214));
    }
}
