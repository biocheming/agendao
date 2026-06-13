//! 木 — ContextStrip: shows current input context above the prompt.
//!
//! Renders via Revue's hstack + Text widgets (no hand-drawing).
//! Displays: attachment count, image count, prompt mode indicator.

use revue::prelude::*;
use crate::store::types::{Attachment, AttachmentKind, PromptMode};

/// Context strip rendered above the prompt input.
pub struct ContextStrip;

impl ContextStrip {
    /// Render the strip using Revue layout widgets.
    pub fn render(
        attachments: &[Attachment],
        mode: &PromptMode,
        ctx: &mut RenderContext,
    ) {
        let mut strip = hstack().gap(1);

        // Mode indicator
        let mode_text = match mode {
            PromptMode::Normal => Text::new("").fg(Color::rgb(86, 95, 137)),
            PromptMode::Shell => Text::new("[shell]").fg(Color::rgb(224, 175, 104)),
            PromptMode::Slash(q) => Text::new(format!("/{}", q)).fg(Color::rgb(125, 207, 255)),
        };
        strip = strip.child(mode_text);

        // Attachment count
        if !attachments.is_empty() {
            let files = attachments.iter().filter(|a| matches!(a.kind, AttachmentKind::File { .. })).count();
            let images = attachments.iter().filter(|a| matches!(a.kind, AttachmentKind::Image { .. })).count();
            let mut info = Vec::new();
            if files > 0 { info.push(format!("📎{}", files)); }
            if images > 0 { info.push(format!("🖼{}", images)); }
            if !info.is_empty() {
                strip = strip.child(Text::new(info.join(" ")).fg(Color::rgb(169, 177, 214)));
            }
        }

        strip.render(ctx);
    }
}
