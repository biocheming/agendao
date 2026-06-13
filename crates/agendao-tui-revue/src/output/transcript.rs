//! 金 — Output authority: TranscriptPanel placeholder.
//!
//! Note: SessionScreen currently handles message rendering directly.
//! This module will be expanded when TranscriptFeed component is built in Phase D.

use revue::prelude::*;

pub struct TranscriptPanel;

impl TranscriptPanel {
    pub fn new() -> Self { Self }
}

impl View for TranscriptPanel {
    fn render(&self, ctx: &mut RenderContext) {
        ctx.draw_text(ctx.area.x + 2, ctx.area.y + 1, "(transcript)", Color::rgb(86, 95, 137));
    }
}
