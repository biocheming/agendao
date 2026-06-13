//! 金 — Output authority: the TranscriptPanel.
//!
//! Renders the session message transcript.

use revue::prelude::*;

use crate::store::session_store::{MessageRole, TranscriptMessage};

/// Panel that displays the session message history.
pub struct TranscriptPanel;

impl TranscriptPanel {
    pub fn new() -> Self {
        Self
    }

    /// Render a list of messages into the given context.
    pub fn render_messages(&self, ctx: &mut RenderContext, messages: &[TranscriptMessage]) {
        let mut y = ctx.area.y + 1;
        let max_y = ctx.area.bottom().saturating_sub(1);

        for msg in messages {
            if y >= max_y {
                break;
            }

            let (prefix, color) = match msg.role {
                MessageRole::User => ("> ", Color::rgb(125, 207, 255)),
                MessageRole::Assistant => ("", Color::rgb(169, 177, 214)),
                MessageRole::System => ("", Color::rgb(86, 95, 137)),
            };

            // Render each line of the message
            for line in msg.content.lines() {
                if y >= max_y {
                    break;
                }
                let display = format!("{}{}", prefix, line);
                ctx.draw_text(ctx.area.x + 1, y, &display, color);
                y += 1;
            }

            // Blank line between messages
            y += 1;
        }
    }
}

impl View for TranscriptPanel {
    fn render(&self, ctx: &mut RenderContext) {
        // Empty transcript — rendered externally with data
        ctx.draw_text(
            ctx.area.x + 2,
            ctx.area.y + 1,
            "(no messages yet)",
            Color::rgb(86, 95, 137),
        );
    }
}
