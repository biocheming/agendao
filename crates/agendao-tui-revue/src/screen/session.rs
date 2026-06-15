//! Session Screen — renders transcript using Revue widgets.
//!
//! All blocks rendered via revue widgets (Text, Markdown, JsonViewer, Callout).
//! Fold state controls reveal of long content.
//! Colors use theme::colors for consistent Tokyo Night identity.

use revue::prelude::*;
use revue::layout::Rect;

use crate::bridge::api::ApiBridge;
use crate::store::session_store::SessionStore;
use crate::store::types::*;
use crate::theme::colors;

const FOLD_PREVIEW_LINES: usize = 3;

/// Estimate the natural rendered height (in rows) of a single transcript
/// block. Callers that build a transcript inside a vstack must pair each
/// `render_block(block)` with `child_sized(_, transcript_block_height(block))`
/// — without explicit sizing, vstack distributes the available area equally
/// across all blocks and a single message gobbles the whole pane.
/// Color of the `▌` left-bar for one block, by role.
///
/// The TranscriptFeed renders every block with a 1-column-wide `▌` on
/// the left edge so the eye can scan a long scroll and pick out role
/// boundaries without reading the chip text. The mapping here is the
/// single source of truth — both the cursor bar and the static role
/// bar in [`render_block`] consume this helper so old (historical) and
/// new (cursor-fold) blocks look identical.
///
/// Conventions (per mockup E "glass tactile" direction):
///   - USER   = E_TEAL         (deep cyan, distinct from FG_SECONDARY)
///   - ASSIST = E_AMBER        (warm amber, complements the cyan user)
///   - THINK  = ACCENT_YELLOW  (muted, "internal" voice)
///   - TOOL   = E_AMBER        (matches the tool chip color)
///   - SYSTEM = ACCENT_YELLOW  (warnings/info)
///   - STAGE  = ACCENT_BLUE    (orchestrator phase)
///   - SKILL  = ACCENT_PURPLE  (capability load)
///   - COMPACT= FG_MUTED       (background housekeeping, no attention)
///   - IMAGE  = ACCENT_PURPLE  (asset reference)
pub fn block_accent(block: &TranscriptBlock) -> revue::prelude::Color {
    use crate::theme::colors;
    match block {
        TranscriptBlock::UserPrompt { .. } => colors::E_TEAL,
        TranscriptBlock::AssistantMsg { .. } => colors::E_AMBER,
        TranscriptBlock::Thinking { .. } => colors::ACCENT_YELLOW,
        TranscriptBlock::ToolCall { .. } => colors::E_AMBER,
        TranscriptBlock::ToolResult { .. } => colors::E_AMBER,
        TranscriptBlock::StageUpdate { .. } => colors::ACCENT_BLUE,
        TranscriptBlock::SkillActivated { .. } => colors::ACCENT_PURPLE,
        TranscriptBlock::TodoList { .. } => colors::ACCENT_PURPLE,
        TranscriptBlock::CompactionHint { .. } => colors::FG_MUTED,
        TranscriptBlock::SystemNotice { .. } => colors::ACCENT_YELLOW,
        TranscriptBlock::ImageRef { .. } => colors::ACCENT_PURPLE,
    }
}

/// 一个 block 的成形布局——高度（阴）与形态（阳）由同一次 match 产出。
///
/// 唯一真相：height 与 view 在同一分支字面相邻，改任一变体的渲染必然
/// 同改（触点数 1）。多数分支 height 随 view 的 `child_sized` 累加，物理上
/// 无法与 view 漂移；少数用 border/flex 的分支（AssistantMsg、StageUpdate）
/// 在同分支用显式公式，由单测锁定一致性。
pub struct BlockLayout {
    pub height: u16,
    pub view: revue::widget::Stack,
}

pub fn layout_block(block: &TranscriptBlock) -> BlockLayout {
    match block {
        // ── User Prompt ──
        // height 随行累加。修正：原 transcript_block_height 在 Truncated
        // total≤3 时返回 total+1（多 1 行空白），现以 view 实际行数为准。
        TranscriptBlock::UserPrompt { content, fold, .. } => {
            use crate::store::types::FoldState;
            let total = content.lines().count();
            let (arrow, body_text, more_hint) = match fold {
                FoldState::Folded => ("▸", String::new(), None),
                FoldState::Truncated if total > FOLD_PREVIEW_LINES => (
                    "▾",
                    truncate_lines(content, FOLD_PREVIEW_LINES),
                    Some(format!("… +{} more lines", total - FOLD_PREVIEW_LINES)),
                ),
                FoldState::Truncated | FoldState::Expanded => ("▾", content.clone(), None),
            };
            let first_line = body_text.lines().next().unwrap_or("");
            let rest: Vec<&str> = body_text.lines().skip(1).collect();

            let mut stack = vstack().gap(0)
                .child_sized(
                    hstack().gap(0)
                        .child_sized(Text::new(format!(" {} ", arrow)).fg(colors::FG_MUTED), 3)
                        .child_sized(
                            Text::new(" YOU ").bold().fg(colors::E_TEAL).bg(colors::SURFACE_USER),
                            5,
                        )
                        .child_flex(
                            Text::new(format!(" {}", first_line)).fg(colors::FG_PRIMARY),
                            1.0,
                        ),
                    1,
                );
            let mut height = 1u16;
            for line in &rest {
                stack = stack.child_sized(
                    Text::new(format!("         {}", line)).fg(colors::FG_PRIMARY),
                    1,
                );
                height += 1;
            }
            if let Some(hint) = more_hint {
                stack = stack.child_sized(
                    Text::new(format!("         {}  (Space to expand)", hint))
                        .fg(colors::FG_MUTED).italic(),
                    1,
                );
                height += 1;
            }
            BlockLayout { height, view: stack }
        }

        // ── Assistant Message ──
        // RevueMarkdown 构造一次，height 与 view 共享（原 render 与 height 各造一遍）。
        TranscriptBlock::AssistantMsg { content, .. } => {
            let mut stack = vstack().gap(0)
                .child_sized(
                    Text::new(" ASSISTANT ").bold().fg(colors::FG_PRIMARY).bg(colors::SURFACE_RAISED),
                    1,
                );
            let height = if content.is_empty() {
                stack = stack.child_sized(Text::new("  …").fg(colors::FG_MUTED), 1);
                2
            } else {
                let mut md = crate::markdown::RevueMarkdown::new();
                md.set_content(content);
                let lines = md.line_count().max(1) as u16;
                stack = stack.child(md.as_stack());
                1 + lines
            };
            BlockLayout { height, view: stack }
        }

        // ── Thinking / Reasoning ──
        TranscriptBlock::Thinking { content, fold, duration_ms, .. } => {
            use crate::store::types::FoldState;
            let wc = content.split_whitespace().count();
            match fold {
                FoldState::Folded => {
                    let summary = if *duration_ms > 0 {
                        format!(" 💭 thinking · {} words · {}ms", wc, duration_ms)
                    } else {
                        format!(" 💭 thinking · {} words", wc)
                    };
                    BlockLayout {
                        height: 1,
                        view: vstack().child(
                            Text::new(summary).fg(colors::FG_MUTED).italic().bg(colors::SURFACE_THINK),
                        ),
                    }
                }
                FoldState::Truncated => {
                    let head = Text::new(" 💭 THINKING ").bold().fg(colors::E_AMBER);
                    let mut body = vstack().gap(0).child_sized(head, 1);
                    let mut height = 1u16;
                    let total = content.lines().count();
                    for line in content.lines().take(FOLD_PREVIEW_LINES) {
                        body = body.child_sized(Text::new(line).fg(colors::FG_MUTED).italic(), 1);
                        height += 1;
                    }
                    if total > FOLD_PREVIEW_LINES {
                        body = body.child_sized(
                            Text::new(format!("… +{} more lines", total - FOLD_PREVIEW_LINES))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                        height += 1;
                    }
                    BlockLayout { height, view: vstack().child(body.class("ThinkingBlock")) }
                }
                FoldState::Expanded => {
                    let head = Text::new(" 💭 THINKING ").bold().fg(colors::E_AMBER);
                    let mut body = vstack().gap(0).child_sized(head, 1);
                    let mut height = 1u16;
                    for line in content.lines() {
                        body = body.child_sized(Text::new(line).fg(colors::FG_MUTED).italic(), 1);
                        height += 1;
                    }
                    BlockLayout { height, view: vstack().child(body.class("ThinkingBlock")) }
                }
            }
        }

        // ── Tool Call ──
        // 修正：原 height 对带参返回 2，但 view 只有 1 行 → 统一 1 行。
        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            let (icon, status_color) = match phase {
                ToolPhase::Starting => ("◌", colors::ACCENT_BLUE),
                ToolPhase::Running  => ("◐", colors::E_AMBER),
                ToolPhase::Done     => ("●", colors::E_TEAL),
            };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
            let preview = if params.is_empty() {
                String::new()
            } else if params.len() > 40 {
                format!(" · {}…", &params.chars().take(37).collect::<String>())
            } else {
                format!(" · {}", params)
            };
            BlockLayout {
                height: 1,
                view: vstack().child(
                    hstack().gap(0)
                        .child_sized(Text::new(format!(" {} ", icon)).fg(status_color), 3)
                        .child_sized(Text::new("⚒ tool").bold().fg(colors::E_AMBER), 7)
                        .child_sized(
                            Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY),
                            name_display.chars().count() as u16 + 4,
                        )
                        .child_flex(Text::new(preview).fg(colors::FG_MUTED), 1.0),
                ),
            }
        }

        // ── Tool Result ──
        TranscriptBlock::ToolResult { name, result, is_error, fold, .. } => {
            use crate::store::types::FoldState;
            let total_lines = result.lines().count();
            let total_bytes = result.len();
            let (icon, accent) = if *is_error { ("✕", colors::ACCENT_RED) } else { ("✓", colors::E_TEAL) };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
            let name_w = name_display.chars().count() as u16 + 4;
            match fold {
                FoldState::Folded => BlockLayout {
                    height: 1,
                    view: vstack().child(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▸ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(
                                Text::new(format!(" · {} lines · {} chars", total_lines, total_bytes))
                                    .fg(colors::FG_MUTED),
                                1.0,
                            )
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2),
                    ),
                },
                FoldState::Truncated => {
                    let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                    let limit = FOLD_PREVIEW_LINES.min(total_lines);
                    let mut stack = vstack().gap(0).child_sized(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▾ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(Text::new(""), 1.0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2),
                        1,
                    );
                    let mut height = 1u16;
                    for line in result.lines().take(limit) {
                        stack = stack.child_sized(Text::new(format!("    {}", line)).fg(body_color), 1);
                        height += 1;
                    }
                    if total_lines > limit {
                        stack = stack.child_sized(
                            Text::new(format!("    … +{} more lines", total_lines - limit))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                        height += 1;
                    }
                    BlockLayout { height, view: stack }
                }
                FoldState::Expanded => {
                    let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                    let view_lines = total_lines.min(20);
                    let mut stack = vstack().gap(0).child_sized(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▾ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(Text::new(""), 1.0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2),
                        1,
                    );
                    let mut height = 1u16;
                    for line in result.lines().take(view_lines) {
                        stack = stack.child_sized(Text::new(format!("    {}", line)).fg(body_color), 1);
                        height += 1;
                    }
                    if total_lines > view_lines {
                        stack = stack.child_sized(
                            Text::new(format!("    … +{} more lines", total_lines - view_lines))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                        height += 1;
                    }
                    BlockLayout { height, view: stack }
                }
            }
        }

        // ── Todo List ──
        // 修正：原 height Folded 返回 1，但 view 是 header + summary = 2 行 → 统一 2。
        TranscriptBlock::TodoList { items, fold, summary, .. } => {
            use crate::store::types::{FoldState, TodoStatus};
            let done = items.iter().filter(|i| i.status == TodoStatus::Completed).count();
            let in_progress = items.iter().filter(|i| i.status == TodoStatus::InProgress).count();
            let pending = items.len().saturating_sub(done + in_progress);
            let mut header = String::from("◈ Tasks");
            if let Some(ref s) = summary {
                if !s.phase.is_empty() { header.push_str(&format!(": {}", s.phase)); }
                if !s.duration.is_empty() { header.push_str(&format!(" · {}", s.duration)); }
                if !s.tokens.is_empty() { header.push_str(&format!(" · {}", s.tokens)); }
            }
            let mut s = vstack().gap(0)
                .child_sized(Text::new(header).fg(colors::ACCENT_PURPLE).bold(), 1);
            let mut height = 1u16;
            match fold {
                FoldState::Folded => {
                    s = s.child_sized(
                        Text::new(format!("  … {} pending, {} completed", pending, done))
                            .fg(colors::FG_MUTED).italic(),
                        1,
                    );
                    height += 1;
                }
                FoldState::Truncated => {
                    let limit = FOLD_PREVIEW_LINES.min(items.len());
                    for item in items.iter().take(limit) {
                        let (icon, color) = match item.status {
                            TodoStatus::Completed => ("✔", colors::ACCENT_GREEN),
                            TodoStatus::InProgress => ("◼", colors::E_AMBER),
                            TodoStatus::Cancelled => ("✕", colors::FG_MUTED),
                            TodoStatus::Pending => ("◻", colors::FG_MUTED),
                        };
                        s = s.child_sized(Text::new(format!("  {} {}", icon, item.content)).fg(color), 1);
                        height += 1;
                    }
                    if items.len() > limit {
                        s = s.child_sized(
                            Text::new(format!("  … +{} pending, +{} completed", pending, done))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                        height += 1;
                    }
                }
                FoldState::Expanded => {
                    for item in items.iter() {
                        let (icon, color) = match item.status {
                            TodoStatus::Completed => ("✔", colors::ACCENT_GREEN),
                            TodoStatus::InProgress => ("◼", colors::E_AMBER),
                            TodoStatus::Cancelled => ("✕", colors::FG_MUTED),
                            TodoStatus::Pending => ("◻", colors::FG_MUTED),
                        };
                        s = s.child_sized(Text::new(format!("  {} {}", icon, item.content)).fg(color), 1);
                        height += 1;
                    }
                }
            }
            BlockLayout { height, view: s.class("TodoBlock") }
        }

        // ── Skill Activated ──
        TranscriptBlock::SkillActivated { name, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!(" ⚡ skill · {}", name)).fg(colors::ACCENT_PURPLE)),
        },

        // ── Stage Update ──
        // height = border(2) + status(1) + metadata 行数。
        TranscriptBlock::StageUpdate { name, status, metadata, .. } => {
            let (status_icon, status_color) = match status.as_str() {
                "Running" | "running"     => ("▶", colors::ACCENT_CYAN),
                "Done" | "done"           => ("✓", colors::ACCENT_GREEN),
                "Waiting" | "waiting"     => ("⏳", colors::ACCENT_YELLOW),
                "Cancelled" | "cancelled" | "Cancelling" => ("✕", colors::FG_MUTED),
                "Blocked" | "blocked"     => ("⊘", colors::ACCENT_RED),
                "Retrying" | "retrying"   => ("↻", colors::ACCENT_YELLOW),
                _                          => ("●", colors::FG_MUTED),
            };
            let mut body = vstack().gap(0);
            body = body.child(
                hstack().gap(1)
                    .child_sized(Text::new(status_icon).fg(status_color), 2)
                    .child(Text::new(status).fg(status_color)),
            );
            let mut meta_lines = 0u16;
            if let Some(ref detail) = metadata {
                for line in detail.lines() {
                    if line.is_empty() { continue; }
                    body = body.child(Text::new(format!("  {}", line)).fg(colors::FG_MUTED));
                    meta_lines += 1;
                }
            }
            BlockLayout {
                height: 3 + meta_lines,
                view: vstack().child(
                    Border::rounded()
                        .title(format!(" stage · {} ", name))
                        .fg(colors::ACCENT_BLUE)
                        .child(body),
                ),
            }
        }

        // ── Compaction Hint ──
        TranscriptBlock::CompactionHint { before_tokens, after_tokens, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(
                format!(" 📦 compact · {} → {} tokens", before_tokens, after_tokens),
            ).fg(colors::FG_MUTED).italic()),
        },

        // ── System Notice ──
        TranscriptBlock::SystemNotice { text, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!(" ℹ  {}", text)).fg(colors::FG_MUTED)),
        },

        // ── Image Reference ──
        TranscriptBlock::ImageRef { mime, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!(" 🖼  [{}]", mime)).fg(colors::FG_MUTED)),
        },
    }
}

pub fn transcript_block_height(block: &TranscriptBlock) -> u16 {
    use crate::store::types::FoldState;
    match block {
        TranscriptBlock::UserPrompt { content, fold, .. } => {
            let total = content.lines().count();
            match fold {
                FoldState::Folded => 1, // role label only
                FoldState::Truncated => {
                    let body = FOLD_PREVIEW_LINES.min(total) as u16;
                    let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                    1 + body + extra
                }
                FoldState::Expanded => total.max(1) as u16 + 1,
            }
        }
        TranscriptBlock::AssistantMsg { content, .. } => {
            if content.is_empty() {
                2
            } else {
                let mut md = crate::markdown::RevueMarkdown::new();
                md.set_content(content);
                1 /* role label */ + md.line_count().max(1)
            }
        }
        TranscriptBlock::Thinking { content, fold, .. } => {
            match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => {
                    let total = content.lines().count();
                    let body = FOLD_PREVIEW_LINES.min(total) as u16;
                    let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                    1 + body + extra // label + body (no border, padded bg)
                }
                FoldState::Expanded => 1 + content.lines().count().max(1) as u16,
            }
        }
        TranscriptBlock::ToolCall { params, .. } => {
            if params.is_empty() { 1 } else { 2 }
        }
        TranscriptBlock::ToolResult { result, fold, .. } => {
            match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => {
                    let total = result.lines().count();
                    let body = FOLD_PREVIEW_LINES.min(total) as u16;
                    let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                    1 + body + extra
                }
                FoldState::Expanded => {
                    let lines = result.lines().count().min(20).max(1) as u16;
                    let extra = if result.lines().count() > 20 { 1 } else { 0 };
                    1 + lines + extra
                }
            }
        }
        TranscriptBlock::StageUpdate { metadata, .. } => {
            let extra = metadata.as_ref().map(|m| m.lines().count() as u16).unwrap_or(0);
            // Border (2) + status line (1) + metadata
            3 + extra
        }
        TranscriptBlock::TodoList { items, fold, .. } => match fold {
            FoldState::Folded => 1,
            FoldState::Truncated => {
                let body = FOLD_PREVIEW_LINES.min(items.len()) as u16;
                let extra = if items.len() > FOLD_PREVIEW_LINES { 1 } else { 0 };
                1 + body + extra
            }
            FoldState::Expanded => 1 + items.len().max(1) as u16,
        },
        TranscriptBlock::SkillActivated { .. }
        | TranscriptBlock::CompactionHint { .. }
        | TranscriptBlock::SystemNotice { .. }
        | TranscriptBlock::ImageRef { .. } => 1,
    }
}

/// Render one TranscriptBlock to a Stack.
///
/// Visual language is the "E · Glass Tactile" direction from the
/// mockups (dev_docs/agendao-tui-redesign-mockups.html):
///
///   • USER and ASSISTANT messages are conversational bubbles distinct
///     from operational events. The header line uses a small UPPERCASE
///     label tag with subtle bg so messages glance-read at a different
///     rhythm than tool/reasoning rows.
///   • TOOL CALL / RESULT are single-row chips (`⚒ name · preview · 12 lines ✓`).
///     They communicate "something happened" without competing with
///     the actual answer for vertical real estate.
///   • REASONING is a muted italic accordion — clearly subordinate to
///     the answer above it.
///   • STAGE remains a Border card because it's a top-level orchestrator
///     phase, distinct from the inline conversation flow.
///
/// We deliberately avoid `revue::widget::markdown` for the assistant
/// body because markdown's wrapped line count is opaque to outside
/// callers — `child_sized` against a wrong estimate clips text.
/// `revue` Text/RichText also renders to y=0 only, so multi-line
/// content must become a column of separate Text children, one per
/// source line, each given child_sized(_, 1).
pub fn render_block(block: &TranscriptBlock) -> revue::widget::Stack {
    match block {
        // ── User Prompt ──────────────────────────────────────────────
        //
        // Compact inline form: " YOU " chip on a teal-tint background
        // followed by the body text. No border because user messages
        // are usually short — wrapping them in a bubble doubles their
        // height for no information gain. The chip itself is enough
        // to distinguish "this is what you said" from the assistant's
        // reply that follows.
        //
        // Folded `▸` / expanded `▾` glyph in front of the chip lets
        // the user toggle long prompts the same way tool results work.
        TranscriptBlock::UserPrompt { content, fold, .. } => {
            use crate::store::types::FoldState;
            let total = content.lines().count();
            let (arrow, body_text, more_hint) = match fold {
                FoldState::Folded => ("▸", String::new(), None),
                FoldState::Truncated => {
                    if total > FOLD_PREVIEW_LINES {
                        ("▾", truncate_lines(content, FOLD_PREVIEW_LINES),
                         Some(format!("… +{} more lines", total - FOLD_PREVIEW_LINES)))
                    } else {
                        ("▾", content.clone(), None)
                    }
                }
                FoldState::Expanded => ("▾", content.clone(), None),
            };
            let first_line = body_text.lines().next().unwrap_or("");
            let rest: Vec<&str> = body_text.lines().skip(1).collect();

            // ── Inline layout: arrow + role chip + first line on same row ──
            let mut stack = vstack().gap(0)
                .child_sized(
                    hstack().gap(0)
                        .child_sized(Text::new(format!(" {} ", arrow)).fg(colors::FG_MUTED), 3)
                        .child_sized(
                            Text::new(" YOU ")
                                .bold()
                                .fg(colors::E_TEAL)
                                .bg(colors::SURFACE_USER),
                            5,
                        )
                        .child_flex(
                            Text::new(format!(" {}", first_line)).fg(colors::FG_PRIMARY),
                            1.0,
                        ),
                    1,
                );
            // Additional lines indented
            for line in &rest {
                stack = stack.child_sized(
                    Text::new(format!("         {}", line)).fg(colors::FG_PRIMARY),
                    1,
                );
            }
            if let Some(hint) = more_hint {
                stack = stack.child_sized(
                    Text::new(format!("         {}  (Space to expand)", hint))
                        .fg(colors::FG_MUTED).italic(),
                    1,
                );
            }
            stack
        }

        // ── Assistant Message ───────────────────────────────────────
        //
        // Rendered with ratatui-markdown which handles tables (Unicode
        // box-drawing), code blocks (adaptive borders), CJK wrapping,
        // and all CommonMark inline formatting.  No more table/markdown
        // splitting — the entire content goes through one renderer.
        TranscriptBlock::AssistantMsg { content, .. } => {
            let mut stack = vstack().gap(0)
                .child_sized(
                    Text::new(" ASSISTANT ")
                        .bold()
                        .fg(colors::FG_PRIMARY)
                        .bg(colors::SURFACE_RAISED),
                    1,
                );
            if content.is_empty() {
                stack = stack.child_sized(Text::new("  …").fg(colors::FG_MUTED), 1);
            } else {
                let mut md = crate::markdown::RevueMarkdown::new();
                md.set_content(content);
                stack = stack.child(md.as_stack());
            }
            stack
        }

        // ── Thinking / Reasoning ─────────────────────────────────────
        //
        // Flat slab — no border.  A tinted background (SURFACE_THINK,
        // a warm amber-wash) distinguishes reasoning from assistant
        // prose.  The amber "💭 THINKING" head acts as a visual anchor.
        TranscriptBlock::Thinking { content, fold, duration_ms, .. } => {
            use crate::store::types::FoldState;
            let wc = content.split_whitespace().count();
            match fold {
                FoldState::Folded => {
                    let summary = if *duration_ms > 0 {
                        format!(" 💭 thinking · {} words · {}ms", wc, duration_ms)
                    } else {
                        format!(" 💭 thinking · {} words", wc)
                    };
                    vstack().child(
                        Text::new(summary)
                            .fg(colors::FG_MUTED).italic()
                            .bg(colors::SURFACE_THINK)
                    )
                }
                FoldState::Truncated => {
                    let head = Text::new(" 💭 THINKING ").bold().fg(colors::E_AMBER);
                    let mut body = vstack().gap(0).child_sized(head, 1);
                    let lines: Vec<&str> = content.lines().take(FOLD_PREVIEW_LINES).collect();
                    let total = content.lines().count();
                    for line in &lines {
                        body = body.child_sized(
                            Text::new(*line).fg(colors::FG_MUTED).italic(), 1);
                    }
                    if total > FOLD_PREVIEW_LINES {
                        body = body.child_sized(
                            Text::new(format!("… +{} more lines", total - FOLD_PREVIEW_LINES))
                                .fg(colors::FG_MUTED).italic(), 1);
                    }
                    vstack().child(body.class("ThinkingBlock"))
                }
                FoldState::Expanded => {
                    let head = Text::new(" 💭 THINKING ").bold().fg(colors::E_AMBER);
                    let mut body = vstack().gap(0).child_sized(head, 1);
                    for line in content.lines() {
                        body = body.child_sized(
                            Text::new(line).fg(colors::FG_MUTED).italic(), 1);
                    }
                    vstack().child(body.class("ThinkingBlock"))
                }
            }
        }

        // ── Tool Call ────────────────────────────────────────────────
        // Mockup E: amber pill chip — `⚒ tool · name · preview · meta`.
        // We keep it as a single Text line with amber bg + border via
        // a 1-row Border so the pill effect comes through. revue can't
        // render a true CSS border-radius pill in 1 row; the rounded
        // border still gives the visual rhythm of "this is a chip".
        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            let (icon, status_color) = match phase {
                ToolPhase::Starting => ("◌", colors::ACCENT_BLUE),
                ToolPhase::Running  => ("◐", colors::E_AMBER),
                ToolPhase::Done     => ("●", colors::E_TEAL),
            };
            // Truncate name to max 20 chars to prevent overflow
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
            let preview = if params.is_empty() {
                String::new()
            } else if params.len() > 40 {
                format!(" · {}…", &params.chars().take(37).collect::<String>())
            } else {
                format!(" · {}", params)
            };
            // Chip body: status icon + label + name + preview, all on
            // one line with amber-tinted bg.
            vstack().child(
                hstack().gap(0)
                    .child_sized(Text::new(format!(" {} ", icon)).fg(status_color), 3)
                    .child_sized(Text::new("⚒ tool").bold().fg(colors::E_AMBER), 7)
                    .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_display.chars().count() as u16 + 4)
                    .child_flex(Text::new(preview).fg(colors::FG_MUTED), 1.0)
            )
        }

        // ── Tool Result ──────────────────────────────────────────────
        TranscriptBlock::ToolResult { name, result, is_error, fold, .. } => {
            use crate::store::types::FoldState;
            let total_lines = result.lines().count();
            let total_bytes = result.len();
            let (icon, accent) = if *is_error {
                ("✕", colors::ACCENT_RED)
            } else {
                ("✓", colors::E_TEAL)
            };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };

            match fold {
                FoldState::Folded => {
                    vstack().child(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▸ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_display.chars().count() as u16 + 4)
                            .child_flex(
                                Text::new(format!(" · {} lines · {} chars", total_lines, total_bytes))
                                    .fg(colors::FG_MUTED),
                                1.0,
                            )
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2)
                    )
                }
                FoldState::Truncated => {
                    let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                    let limit = FOLD_PREVIEW_LINES.min(total_lines);
                    let mut stack = vstack().gap(0).child_sized(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▾ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_display.chars().count() as u16 + 4)
                            .child_flex(Text::new(""), 1.0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2),
                        1,
                    );
                    for line in result.lines().take(limit) {
                        stack = stack.child_sized(
                            Text::new(format!("    {}", line)).fg(body_color), 1,
                        );
                    }
                    if total_lines > limit {
                        stack = stack.child_sized(
                            Text::new(format!("    … +{} more lines", total_lines - limit))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                    }
                    stack
                }
                FoldState::Expanded => {
                    let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                    let view_lines = total_lines.min(20);
                    let mut stack = vstack().gap(0).child_sized(
                        hstack().gap(0)
                            .child_sized(Text::new(" ▾ ").fg(colors::FG_MUTED), 3)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_display.chars().count() as u16 + 4)
                            .child_flex(Text::new(""), 1.0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2),
                        1,
                    );
                    for line in result.lines().take(view_lines) {
                        stack = stack.child_sized(
                            Text::new(format!("    {}", line)).fg(body_color), 1,
                        );
                    }
                    if total_lines > view_lines {
                        stack = stack.child_sized(
                            Text::new(format!("    … +{} more lines", total_lines - view_lines))
                                .fg(colors::FG_MUTED).italic(),
                            1,
                        );
                    }
                    stack
                }
            }
        }

        // ── Todo List ──────────────────────────────────────────────
        TranscriptBlock::TodoList { items, fold, summary, .. } => {
            use crate::store::types::FoldState;
            let done = items.iter().filter(|i| i.status == crate::store::types::TodoStatus::Completed).count();
            let in_progress = items.iter().filter(|i| i.status == crate::store::types::TodoStatus::InProgress).count();
            let pending = items.len().saturating_sub(done + in_progress);
            // Header line
            let mut header = String::from("◈ Tasks");
            if let Some(ref s) = summary {
                if !s.phase.is_empty() { header.push_str(&format!(": {}", s.phase)); }
                if !s.duration.is_empty() { header.push_str(&format!(" · {}", s.duration)); }
                if !s.tokens.is_empty() { header.push_str(&format!(" · {}", s.tokens)); }
            }
            let mut s = vstack().gap(0)
                .child_sized(Text::new(header).fg(colors::ACCENT_PURPLE).bold(), 1);

            match fold {
                FoldState::Folded => {
                    s = s.child_sized(
                        Text::new(format!("  … {} pending, {} completed", pending, done))
                            .fg(colors::FG_MUTED).italic(), 1);
                }
                FoldState::Truncated => {
                    let limit = FOLD_PREVIEW_LINES.min(items.len());
                    for item in items.iter().take(limit) {
                        let (icon, color) = match item.status {
                            crate::store::types::TodoStatus::Completed => ("✔", colors::ACCENT_GREEN),
                            crate::store::types::TodoStatus::InProgress => ("◼", colors::E_AMBER),
                            crate::store::types::TodoStatus::Cancelled => ("✕", colors::FG_MUTED),
                            crate::store::types::TodoStatus::Pending => ("◻", colors::FG_MUTED),
                        };
                        s = s.child_sized(Text::new(format!("  {} {}", icon, item.content)).fg(color), 1);
                    }
                    if items.len() > limit {
                        s = s.child_sized(
                            Text::new(format!("  … +{} pending, +{} completed", pending, done))
                                .fg(colors::FG_MUTED).italic(), 1);
                    }
                }
                FoldState::Expanded => {
                    for item in items.iter() {
                        let (icon, color) = match item.status {
                            crate::store::types::TodoStatus::Completed => ("✔", colors::ACCENT_GREEN),
                            crate::store::types::TodoStatus::InProgress => ("◼", colors::E_AMBER),
                            crate::store::types::TodoStatus::Cancelled => ("✕", colors::FG_MUTED),
                            crate::store::types::TodoStatus::Pending => ("◻", colors::FG_MUTED),
                        };
                        s = s.child_sized(Text::new(format!("  {} {}", icon, item.content)).fg(color), 1);
                    }
                }
            }
            s.class("TodoBlock")
        }

        // ── Skill Activated ─────────────────────────────────────────
        TranscriptBlock::SkillActivated { name, .. } => {
            vstack().child(Text::new(format!(" ⚡ skill · {}", name))
                .fg(colors::ACCENT_PURPLE))
        }

        // ── Stage Update (boxed card — top-level orchestration phase) ─
        TranscriptBlock::StageUpdate { name, status, metadata, .. } => {
            let (status_icon, status_color) = match status.as_str() {
                "Running" | "running"     => ("▶", colors::ACCENT_CYAN),
                "Done" | "done"           => ("✓", colors::ACCENT_GREEN),
                "Waiting" | "waiting"     => ("⏳", colors::ACCENT_YELLOW),
                "Cancelled" | "cancelled" | "Cancelling" => ("✕", colors::FG_MUTED),
                "Blocked" | "blocked"     => ("⊘", colors::ACCENT_RED),
                "Retrying" | "retrying"   => ("↻", colors::ACCENT_YELLOW),
                _                          => ("●", colors::FG_MUTED),
            };

            let mut body = vstack().gap(0);
            body = body.child(
                hstack().gap(1)
                    .child_sized(Text::new(status_icon).fg(status_color), 2)
                    .child(Text::new(status).fg(status_color))
            );
            if let Some(ref detail) = metadata {
                for line in detail.lines() {
                    if line.is_empty() { continue; }
                    body = body.child(Text::new(format!("  {}", line)).fg(colors::FG_MUTED));
                }
            }

            vstack().child(
                Border::rounded()
                    .title(format!(" stage · {} ", name))
                    .fg(colors::ACCENT_BLUE)
                    .child(body)
            )
        }

        // ── Compaction Hint ─────────────────────────────────────────
        TranscriptBlock::CompactionHint { before_tokens, after_tokens, .. } => {
            vstack().child(Text::new(
                format!(" 📦 compact · {} → {} tokens", before_tokens, after_tokens)
            ).fg(colors::FG_MUTED).italic())
        }

        // ── System Notice ───────────────────────────────────────────
        TranscriptBlock::SystemNotice { text, .. } => {
            vstack().child(Text::new(format!(" ℹ  {}", text)).fg(colors::FG_MUTED))
        }

        // ── Image Reference ─────────────────────────────────────────
        TranscriptBlock::ImageRef { mime, .. } => {
            vstack().child(Text::new(format!(" 🖼  [{}]", mime)).fg(colors::FG_MUTED))
        }
    }
}

/// Truncate text to first N lines.
/// One slice of an assistant message body.
///
/// We split the raw text on GFM-style markdown tables (a header row
/// followed by a `| --- | --- |` separator) so the host can route
/// each segment to the right widget: `Markdown` for prose/headings/
/// lists/code, and `Table` for tabular data. revue's markdown widget
/// recognizes table tags via pulldown_cmark but currently doesn't
/// emit the accumulated cells back to the rendered line list, so
/// tables left inline collapse into stray pipe characters.
pub enum AssistantSegment {
    Markdown(String),
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
}

/// Walk `content` line by line and collect a flat list of segments.
///
/// A table starts at a line that, together with the following line,
/// forms `| col1 | col2 |` followed by `| --- | --- |`. Subsequent
/// `| ... |` rows belong to the same table; the first non-`|` line
/// terminates it. Anything outside table fences becomes a Markdown
/// segment containing the original line text (so heading prefixes
/// like `##`, lists, code fences etc. still parse correctly).
pub fn split_assistant_segments(content: &str) -> Vec<AssistantSegment> {
    let lines: Vec<&str> = content.lines().collect();
    let mut segments: Vec<AssistantSegment> = Vec::new();
    let mut md_buf = String::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        // Detect table start: header `| ... |` then sep `|---|---|`.
        if is_table_header_row(line) && i + 1 < lines.len() && is_table_separator_row(lines[i + 1]) {
            // Flush any pending markdown
            if !md_buf.is_empty() {
                segments.push(AssistantSegment::Markdown(std::mem::take(&mut md_buf)));
            }
            let headers = parse_table_row(line);
            let mut rows: Vec<Vec<String>> = Vec::new();
            i += 2; // skip header + separator
            while i < lines.len() && is_table_row(lines[i]) {
                rows.push(parse_table_row(lines[i]));
                i += 1;
            }
            segments.push(AssistantSegment::Table { headers, rows });
            continue;
        }
        // Otherwise accumulate into markdown buffer
        if !md_buf.is_empty() { md_buf.push('\n'); }
        md_buf.push_str(line);
        i += 1;
    }

    if !md_buf.is_empty() {
        segments.push(AssistantSegment::Markdown(md_buf));
    }
    segments
}

/// `| col | col |` — a row that starts and ends with a pipe.
fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.len() >= 3
}

/// Same as `is_table_row` but additionally requires at least 2 cells.
fn is_table_header_row(line: &str) -> bool {
    if !is_table_row(line) { return false; }
    parse_table_row(line).len() >= 2
}

/// `|---|:---:|---:|` — separator row. Each cell is `:?-+:?` after trim.
fn is_table_separator_row(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with('|') || !t.ends_with('|') { return false; }
    let cells = parse_table_row(line);
    if cells.is_empty() { return false; }
    cells.iter().all(|c| {
        let s = c.trim().trim_start_matches(':').trim_end_matches(':');
        !s.is_empty() && s.chars().all(|ch| ch == '-')
    })
}

fn parse_table_row(line: &str) -> Vec<String> {
    let t = line.trim();
    // Strip leading/trailing pipes, split on `|`.
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

/// Truncate text to first N lines.
fn truncate_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().take(n).collect();
    let total = text.lines().count();
    if total > n {
        format!("{}\n   ... ({} more lines)", lines.join("\n"), total - n)
    } else {
        lines.join("\n")
    }
}

// ── Transcript builder ─────────────────────────────────

/// Build the full transcript vstack from message blocks.
pub fn build_transcript(msgs: &[TranscriptBlock]) -> revue::widget::Stack {
    let mut stack = vstack().gap(1);
    for block in msgs {
        stack = stack.child(render_block(block));
    }
    stack.child(Text::new(""))
}

fn estimate_height(msgs: &[TranscriptBlock]) -> u16 {
    use crate::store::types::FoldState;
    let mut h = 0u16;
    for block in msgs {
        h += match block {
            TranscriptBlock::UserPrompt { content, fold, .. } => match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => 3,
                FoldState::Expanded => content.lines().count() as u16 + 2,
            },
            TranscriptBlock::AssistantMsg { content, .. } => {
                if content.is_empty() {
                    2
                } else {
                    let mut md = crate::markdown::RevueMarkdown::new();
                    md.set_content(content);
                    md.line_count().max(1) + 1
                }
            }
            TranscriptBlock::Thinking { fold, .. } => match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => 3,
                FoldState::Expanded => 3,
            },
            TranscriptBlock::ToolCall { .. } => 2,
            TranscriptBlock::ToolResult { result, fold, .. } => match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => 3,
                FoldState::Expanded => result.lines().count().min(10).max(1) as u16 + 1,
            },
            _ => 1,
        };
    }
    h + msgs.len() as u16
}

// ── ScrollView transcript rendering ────────────────────

/// Render transcript into a scrollable viewport.
pub fn render_transcript(
    msgs: &[TranscriptBlock],
    scroll_offset: u16,
    area: Rect,
    ctx: &mut RenderContext,
) -> u16 {
    if msgs.is_empty() {
        Text::new(" No messages yet — type below to start.")
            .fg(colors::FG_MUTED).render(ctx);
        return 0;
    }

    let content_width = area.width.saturating_sub(1);
    if content_width == 0 || area.height == 0 { return scroll_offset; }

    let transcript = build_transcript(msgs);
    let content_h = estimate_height(msgs);

    let sv = revue::widget::scroll_view()
        .content_height(content_h)
        .scroll_offset(scroll_offset)
        .show_scrollbar(true);

    let mut content_buf = sv.create_content_buffer(content_width);
    let content_area = Rect::new(0, 0, content_width, content_h);
    let mut content_ctx = RenderContext::new(&mut content_buf, content_area);
    transcript.render(&mut content_ctx);

    sv.render_content(ctx, &content_buf);
    scroll_offset
}

// ── Status line ────────────────────────────────────────

fn session_status_line(run_status: &RunStatus, prompt_text: &str) -> String {
    match run_status {
        RunStatus::Idle if prompt_text.is_empty() => " Type below. Ctrl+B: sidebar".into(),
        RunStatus::Idle => format!(" {} chars | Enter: send", prompt_text.len()),
        RunStatus::Sending => " Sending...".into(),
        RunStatus::Running => " Running... Esc: stop".into(),
        RunStatus::WaitingUser => " Waiting for your response...".into(),
        RunStatus::Error(e) => format!(" Error: {}", e),
    }
}

/// Render the full session view (header + transcript + status).
pub fn render_session(
    msgs: &[TranscriptBlock],
    run_status: &RunStatus,
    prompt_text: &str,
    _prompt_focused: bool,
    session_id: &str,
    title: &str,
    scroll_offset: u16,
    ctx: &mut RenderContext,
) {
    let area = ctx.area;
    let header_h: u16 = 2;
    let status_h: u16 = 1;
    let prompt_h: u16 = 3;
    let reserved = header_h + status_h + prompt_h;
    let transcript_h = area.height.saturating_sub(reserved);
    if transcript_h < 1 { return; }

    // 1. Header
    let ha = Rect { x: area.x, y: area.y, width: area.width, height: header_h };
    let mut hc = RenderContext::new(ctx.buffer, ha);
    Text::new(&format!(" {}", title)).bold().fg(colors::FG_PRIMARY).render(&mut hc);
    Text::new(&format!(" [{}]", session_id)).fg(colors::FG_MUTED).render(&mut hc);

    // 2. Transcript
    let ta = Rect { x: area.x, y: area.y + header_h, width: area.width, height: transcript_h };
    let mut tc = RenderContext::new(ctx.buffer, ta);
    render_transcript(msgs, scroll_offset, ta, &mut tc);

    // 3. Status line
    let sa = Rect { x: area.x, y: area.y + header_h + transcript_h, width: area.width, height: status_h };
    let mut sc = RenderContext::new(ctx.buffer, sa);
    let scolor = match run_status {
        RunStatus::Idle => colors::FG_MUTED,
        RunStatus::Sending | RunStatus::WaitingUser => colors::ACCENT_YELLOW,
        RunStatus::Running => colors::ACCENT_CYAN,
        RunStatus::Error(_) => colors::ACCENT_RED,
    };
    Text::new(&session_status_line(run_status, prompt_text)).fg(scolor).render(&mut sc);
}

// ── SessionScreen (kept for API integration) ───────────

pub struct SessionScreen {
    pub session_id: String,
    pub session: SessionStore,
    pub scroll_offset: u16,
    pub api: Option<ApiBridge>,
}

impl SessionScreen {
    pub fn new(session_id: String, api: Option<ApiBridge>) -> Self {
        Self { session_id, session: SessionStore::new(), scroll_offset: 0, api }
    }

    pub fn handle_key(&mut self, key: &revue::event::Key) -> bool {
        match key {
            revue::event::Key::Up | revue::event::Key::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                true
            }
            revue::event::Key::Down | revue::event::Key::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                true
            }
            revue::event::Key::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                true
            }
            revue::event::Key::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                true
            }
            _ => false,
        }
    }

    pub fn sync_messages(&self, other: &SessionStore) {
        self.session.messages.set(other.messages.get());
        self.session.run_status.set(other.run_status.get());
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::store::types::{FoldState, TodoItem, TodoStatus, ToolPhase};

    fn blk(b: TranscriptBlock) -> BlockLayout { layout_block(&b) }

    #[test]
    fn user_prompt_folded_is_one_row() {
        let b = TranscriptBlock::UserPrompt {
            id: "u".into(), content: "a\nb\nc".into(), fold: FoldState::Folded,
        };
        assert_eq!(blk(b).height, 1);
    }

    #[test]
    fn user_prompt_truncated_short_matches_view() {
        // 修正点：total=2 (≤3)，view = chip(1) + rest(1) = 2；原 height 错为 3
        let b = TranscriptBlock::UserPrompt {
            id: "u".into(), content: "a\nb".into(), fold: FoldState::Truncated,
        };
        assert_eq!(blk(b).height, 2);
    }

    #[test]
    fn user_prompt_truncated_long_is_five_rows() {
        // total=5 (>3)：chip(1) + 3 body + 1 hint = 5
        let b = TranscriptBlock::UserPrompt {
            id: "u".into(), content: "a\nb\nc\nd\ne".into(), fold: FoldState::Truncated,
        };
        assert_eq!(blk(b).height, 5);
    }

    #[test]
    fn user_prompt_expanded_matches_view() {
        // Expanded: chip 行含 first line，rest 含其余 → view = total 行
        // （不是 total+1；旧 transcript_block_height 在 Expanded 也多算 1，
        // 与 Truncated-short 同病，layout_block 以 view 为真相修正。
        // 这是 spec 漏列的第 4 处不一致，合并时一并修正。）
        let b = TranscriptBlock::UserPrompt {
            id: "u".into(), content: "a\nb\nc".into(), fold: FoldState::Expanded,
        };
        assert_eq!(blk(b).height, 3);
    }

    #[test]
    fn tool_call_always_one_row() {
        // 修正点：带参也只 1 行（原 height 对带参返回 2）
        let with_params = TranscriptBlock::ToolCall {
            id: "t".into(), name: "read".into(),
            params: "{\"path\":\"x\"}".into(), phase: ToolPhase::Done,
        };
        let empty = TranscriptBlock::ToolCall {
            id: "t".into(), name: "read".into(), params: String::new(), phase: ToolPhase::Done,
        };
        assert_eq!(blk(with_params).height, 1);
        assert_eq!(blk(empty).height, 1);
    }

    #[test]
    fn todo_list_folded_is_two_rows() {
        // 修正点：header + summary = 2（原 height 错为 1）
        let b = TranscriptBlock::TodoList {
            id: "td".into(),
            items: vec![TodoItem { content: "x".into(), status: TodoStatus::Pending }],
            fold: FoldState::Folded,
            summary: None,
        };
        assert_eq!(blk(b).height, 2);
    }

    #[test]
    fn assistant_msg_empty_is_two_rows() {
        let b = TranscriptBlock::AssistantMsg { id: "a".into(), content: String::new() };
        assert_eq!(blk(b).height, 2);
    }

    #[test]
    fn assistant_msg_with_content_at_least_two_rows() {
        let b = TranscriptBlock::AssistantMsg { id: "a".into(), content: "# hi\nbody".into() };
        assert!(blk(b).height >= 2);
    }

    #[test]
    fn stage_update_includes_metadata_rows() {
        let b = TranscriptBlock::StageUpdate {
            id: "s".into(), name: "p".into(), status: "Running".into(),
            metadata: Some("l1\nl2".into()),
        };
        assert_eq!(blk(b).height, 5); // border(2) + status(1) + 2 metadata
    }

    #[test]
    fn thinking_folded_is_one_row() {
        let b = TranscriptBlock::Thinking {
            id: "t".into(), content: "a b c".into(), fold: FoldState::Folded, duration_ms: 0,
        };
        assert_eq!(blk(b).height, 1);
    }

    #[test]
    fn tool_result_folded_is_one_row() {
        let b = TranscriptBlock::ToolResult {
            id: "r".into(), name: "read".into(), result: "out".into(),
            is_error: false, fold: FoldState::Folded,
        };
        assert_eq!(blk(b).height, 1);
    }

    #[test]
    fn single_row_variants() {
        let skill = TranscriptBlock::SkillActivated { id: "s".into(), name: "n".into() };
        let compact = TranscriptBlock::CompactionHint { id: "c".into(), before_tokens: 10, after_tokens: 5 };
        let notice = TranscriptBlock::SystemNotice { id: "n".into(), text: "hi".into() };
        let img = TranscriptBlock::ImageRef { id: "i".into(), mime: "png".into() };
        assert_eq!(blk(skill).height, 1);
        assert_eq!(blk(compact).height, 1);
        assert_eq!(blk(notice).height, 1);
        assert_eq!(blk(img).height, 1);
    }
}
