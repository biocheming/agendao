//! 金 — TranscriptFeed: scrollable message list.
//!
//! Uses Revue's ScrollView + vstack to render the full transcript.

use revue::prelude::*;
use crate::store::types::TranscriptBlock;
use crate::output::block_render::render_block;

/// Transcript feed component.
pub struct TranscriptFeed;

impl TranscriptFeed {
    pub fn new() -> Self { Self }

    /// Render messages into a vstack (for use inside ScrollView).
    pub fn render_blocks(blocks: &[TranscriptBlock]) -> revue::widget::Stack {
        let mut stack = vstack().gap(1);
        for block in blocks {
            stack = stack.child(render_block(block));
        }
        stack
    }
}

impl View for TranscriptFeed {
    fn render(&self, ctx: &mut RenderContext) {
        Text::new("(transcript)").fg(Color::rgb(86, 95, 137)).render(ctx);
    }
}
