//! 金 — TranscriptBlock → Revue widget tree.
//!
//! Styled via CSS classes defined in styles/base.css.

use revue::prelude::*;
use crate::store::types::*;

const FOLD_PREVIEW_LINES: usize = 3;

pub fn render_block(block: &TranscriptBlock) -> revue::widget::Stack {
    match block {
        TranscriptBlock::UserPrompt { content, fold, .. } => match fold {
            FoldState::Folded => {
                let wc = content.split_whitespace().count();
                vstack().child(Text::new(format!("▸ YOU · {} words", wc)).class("UserPrompt"))
            }
            FoldState::Truncated => {
                let preview: String = content.lines().take(FOLD_PREVIEW_LINES).collect::<Vec<_>>().join("\n");
                let extra = if content.lines().count() > FOLD_PREVIEW_LINES {
                    format!("\n… +{} more", content.lines().count() - FOLD_PREVIEW_LINES)
                } else { String::new() };
                vstack()
                    .child(Text::new("▾ YOU").bold().class("UserPrompt"))
                    .child(Text::new(format!("{}{}", preview, extra)).class("UserPrompt"))
            }
            FoldState::Expanded => {
                vstack()
                    .child(Text::new("▾ YOU").bold().class("UserPrompt"))
                    .child(Text::new(content.as_str()).class("UserPrompt"))
            }
        },

        TranscriptBlock::Thinking { content, fold, duration_ms, .. } => match fold {
            FoldState::Folded => {
                let wc = content.split_whitespace().count();
                vstack().child(Text::new(format!("▸ 💭 thinking · {} words · {}ms", wc, duration_ms)).class("ThinkingBlock"))
            }
            FoldState::Truncated => {
                let preview: String = content.lines().take(FOLD_PREVIEW_LINES).collect::<Vec<_>>().join("\n");
                let extra = if content.lines().count() > FOLD_PREVIEW_LINES {
                    format!("\n… +{} more", content.lines().count() - FOLD_PREVIEW_LINES)
                } else { String::new() };
                vstack()
                    .child(Text::new("▼ 💭 THINKING").class("ThinkingBlock"))
                    .child(Text::new(format!("{}{}", preview, extra)).class("ThinkingBlock"))
            }
            FoldState::Expanded => {
                vstack()
                    .child(Text::new("▼ 💭 THINKING").class("ThinkingBlock"))
                    .child(Text::new(content.as_str()).class("ThinkingBlock"))
            }
        },

        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            let icon = match phase { ToolPhase::Starting => "○", ToolPhase::Running => "◉", ToolPhase::Done => "●" };
            let params_short = if params.len() > 80 { format!("{}...", &params[..77]) } else { params.clone() };
            vstack()
                .child(Text::new(format!("{} {}", icon, name)).bold().class("ToolCall"))
                .child(Text::new(params_short).class("ToolCall"))
        }

        TranscriptBlock::ToolResult { name, result, is_error, fold, .. } => {
            let cls = if *is_error { "ToolResultError" } else { "ToolResult" };
            match fold {
                FoldState::Folded => {
                    let lines = result.lines().count();
                    vstack().child(Text::new(format!("▸ result · {} · {} lines", name, lines)).class(cls))
                }
                FoldState::Truncated => {
                    let preview: String = result.lines().take(FOLD_PREVIEW_LINES).collect::<Vec<_>>().join("\n");
                    let extra = if result.lines().count() > FOLD_PREVIEW_LINES {
                        format!("\n… +{} more", result.lines().count() - FOLD_PREVIEW_LINES)
                    } else { String::new() };
                    vstack()
                        .child(Text::new(format!("▾ {} result:", name)).class(cls))
                        .child(Text::new(format!("{}{}", preview, extra)).class(cls))
                }
                FoldState::Expanded => {
                    vstack()
                        .child(Text::new(format!("▾ {} result:", name)).class(cls))
                        .child(Text::new(result.as_str()).class(cls))
                }
            }
        }

        TranscriptBlock::SkillActivated { name, .. } => {
            vstack().child(Text::new(format!("⚡ {}", name)).class("SkillBlock"))
        }

        TranscriptBlock::StageUpdate { name, status, .. } => {
            vstack().child(Text::new(format!("⚙ {} — {}", name, status)).class("StageBlock"))
        }

        TranscriptBlock::AssistantMsg { content, .. } => {
            vstack().child(revue::widget::markdown(content.as_str()))
        }

        TranscriptBlock::TodoList { items, fold, .. } => {
            let done = items.iter().filter(|i| i.status == TodoStatus::Completed).count();
            let pending = items.len().saturating_sub(done);
            match fold {
                FoldState::Folded => {
                    vstack().child(Text::new(format!("▸ ◈ Tasks · {} pending, {} done", pending, done)).class("TodoBlock"))
                }
                FoldState::Truncated => {
                    let mut s = vstack().child(Text::new("▾ ◈ Tasks").bold().class("TodoBlock"));
                    for item in items.iter().take(FOLD_PREVIEW_LINES) {
                        let icon = match item.status {
                            TodoStatus::Completed => "✔", TodoStatus::InProgress => "◼",
                            TodoStatus::Cancelled => "✕", TodoStatus::Pending => "◻",
                        };
                        s = s.child(Text::new(format!("  {} {}", icon, item.content)).class("TodoBlock"));
                    }
                    if items.len() > FOLD_PREVIEW_LINES {
                        s = s.child(Text::new(format!("  … +{} more", items.len() - FOLD_PREVIEW_LINES)).class("TodoBlock"));
                    }
                    s
                }
                FoldState::Expanded => {
                    let mut s = vstack().child(Text::new("▾ ◈ Tasks").bold().class("TodoBlock"));
                    for item in items.iter() {
                        let icon = match item.status {
                            TodoStatus::Completed => "✔", TodoStatus::InProgress => "◼",
                            TodoStatus::Cancelled => "✕", TodoStatus::Pending => "◻",
                        };
                        s = s.child(Text::new(format!("  {} {}", icon, item.content)).class("TodoBlock"));
                    }
                    s
                }
            }
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
