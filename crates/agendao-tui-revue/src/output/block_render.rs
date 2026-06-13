//! 金 — TranscriptBlock → Revue widget tree.
//!
//! Styled via CSS classes defined in styles/base.css.

use revue::prelude::*;
use crate::store::types::*;

pub fn render_block(block: &TranscriptBlock) -> revue::widget::Stack {
    match block {
        TranscriptBlock::UserPrompt { content, folded, .. } => {
            if *folded {
                let preview: String = content.lines().take(3).collect::<Vec<_>>().join("\n");
                let label = if content.lines().count() > 3 {
                    format!("{}... ({} more)", preview, content.lines().count() - 3)
                } else { preview };
                vstack().child(Text::new(label).class("UserPrompt"))
            } else {
                vstack()
                    .child(Text::new("> You").bold().class("UserPrompt"))
                    .child(Text::new(content.as_str()).class("UserPrompt"))
            }
        }

        TranscriptBlock::Thinking { content, folded, duration_ms, .. } => {
            if *folded {
                let wc = content.split_whitespace().count();
                vstack().child(Text::new(format!("▶ Thinking ({} words, {}ms)", wc, duration_ms)).class("ThinkingBlock"))
            } else {
                vstack()
                    .child(Text::new("▼ Thinking").class("ThinkingBlock"))
                    .child(Text::new(content.as_str()).class("ThinkingBlock"))
            }
        }

        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            let icon = match phase { ToolPhase::Starting => "○", ToolPhase::Running => "◉", ToolPhase::Done => "●" };
            let params_short = if params.len() > 80 { format!("{}...", &params[..77]) } else { params.clone() };
            vstack()
                .child(Text::new(format!("{} {}", icon, name)).bold().class("ToolCall"))
                .child(Text::new(params_short).class("ToolCall"))
        }

        TranscriptBlock::ToolResult { name, result, is_error, folded, .. } => {
            let cls = if *is_error { "ToolResultError" } else { "ToolResult" };
            if *folded {
                let preview: String = result.lines().take(3).collect::<Vec<_>>().join("\n");
                vstack().child(Text::new(format!("▶ {}: {}...", name, preview)).class(cls))
            } else {
                vstack()
                    .child(Text::new(format!("▼ {} result:", name)).class(cls))
                    .child(Text::new(result.as_str()).class(cls))
            }
        }

        TranscriptBlock::SkillActivated { name, .. } => {
            vstack().child(Text::new(format!("⚡ {}", name)).class("SkillBlock"))
        }

        TranscriptBlock::StageUpdate { name, status, .. } => {
            vstack().child(Text::new(format!("⚙ {} — {}", name, status)).class("StageBlock"))
        }

        TranscriptBlock::AssistantMsg { content, .. } => {
            vstack().child(Text::new(content.as_str()).class("AssistantMsg"))
        }

        TranscriptBlock::ImageRef { mime, .. } => {
            vstack().child(Text::new(format!("🖼 [{}]", mime)).class("SystemNotice"))
        }

        TranscriptBlock::CompactionHint { before_tokens, after_tokens, .. } => {
            vstack().child(Text::new(format!("📦 {} → {} tokens", before_tokens, after_tokens)).class("Compaction"))
        }

        TranscriptBlock::SystemNotice { text, .. } => {
            vstack().child(Text::new(format!("ℹ {}", text)).class("SystemNotice"))
        }
    }
}
