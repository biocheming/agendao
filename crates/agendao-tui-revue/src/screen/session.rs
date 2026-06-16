//! Session Screen — renders transcript using Revue widgets.
//!
//! All blocks rendered via revue widgets (Text, Markdown, JsonViewer, Callout).
//! Fold state controls reveal of long content.
//! Colors use theme::colors for consistent Tokyo Night identity.

use revue::prelude::*;

use crate::store::types::*;
use crate::theme::colors;

const FOLD_PREVIEW_LINES: usize = 3;

/// Color of the `▌` left-bar for one block, by role.
///
/// The TranscriptFeed renders every block with a 1-column-wide `▌` on
/// the left edge so the eye can scan a long scroll and pick out role
/// boundaries without reading the chip text. The mapping here is the
/// single source of truth — both the cursor bar and the static role
/// bar in the transcript vstack consume this helper so old (historical)
/// and new (cursor-fold) blocks look identical.
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

pub fn layout_block(block: &TranscriptBlock, tick: u64) -> BlockLayout {
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
                            {
                                let (chip, sem) = crate::widget::role_chip::role_chip(
                                    crate::widget::role_chip::Role::User,
                                );
                                Text::new(format!(" {} ", chip)).bold()
                                    .fg(crate::ds::color::resolve_color(sem))
                                    .bg(colors::SURFACE_USER)
                            },
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
        // claudecode 风格：● 圆点（text 色）+ markdown，无 chip 标签。
        // RevueMarkdown 构造一次，height 与 view 共享。
        TranscriptBlock::AssistantMsg { content, .. } => {
            // ● 左侧标记列（3 列）与 markdown 首行同行——对应 UserPrompt
            // （▾ You 首行）/ Thinking（✻ 首行）的"符号+首行"紧凑形态。旧版用
            // vstack.child_sized(_, 3)，在 Column 里 3 是 height，让 ● 占 3 行
            // （既漂移 height 公式，又造成 ● 后空行）；改 hstack 让 ● 与 md 同行，
            // md 续行天然缩进 3 列对齐 ● 之后。
            if content.is_empty() {
                let row = hstack().gap(0)
                    .child_sized(Text::new(" ● ").fg(colors::FG_PRIMARY), 3)
                    .child_sized(Text::new("…").fg(colors::FG_MUTED), 1);
                BlockLayout { height: 1, view: vstack().gap(0).child(row) }
            } else {
                let mut md = crate::markdown::RevueMarkdown::new();
                md.set_content(content);
                let lines = md.line_count().max(1) as u16;
                let row = hstack().gap(0)
                    .child_sized(Text::new(" ● ").fg(colors::FG_PRIMARY), 3)
                    .child_flex(md.as_stack(), 1.0);
                BlockLayout { height: lines, view: vstack().gap(0).child(row) }
            }
        }

        // ── Thinking / Reasoning ──
        // ✻ 与 Spinner Claude 字形同族（·✢✳✶✻✽）。Folded 收起给摘要行；
        // 展开态符号后直接接推理首行（紧凑，去 bold THINKING 单独 header），
        // 续行缩进 3 对齐 ` ✻ ` 之后。
        TranscriptBlock::Thinking { content, fold, duration_ms, .. } => {
            use crate::store::types::FoldState;
            let wc = content.split_whitespace().count();
            match fold {
                FoldState::Folded => {
                    let summary = if *duration_ms > 0 {
                        format!(" ✻ thinking · {} words · {}ms", wc, duration_ms)
                    } else {
                        format!(" ✻ thinking · {} words", wc)
                    };
                    BlockLayout {
                        height: 1,
                        view: vstack().child(
                            Text::new(summary).fg(colors::FG_MUTED).italic().bg(colors::SURFACE_THINK),
                        ),
                    }
                }
                FoldState::Truncated | FoldState::Expanded => {
                    let total = content.lines().count();
                    let limit = if matches!(fold, FoldState::Truncated) {
                        FOLD_PREVIEW_LINES.min(total)
                    } else {
                        total
                    };
                    let mut body = vstack().gap(0);
                    let mut height = 0u16;
                    if total == 0 {
                        body = body.child_sized(
                            Text::new(" ✻ …").fg(colors::FG_MUTED).italic(), 1,
                        );
                        height = 1;
                    } else {
                        for (i, line) in content.lines().take(limit).enumerate() {
                            let text = if i == 0 {
                                format!(" ✻ {}", line)
                            } else {
                                format!("   {}", line)
                            };
                            body = body.child_sized(
                                Text::new(text).fg(colors::FG_MUTED).italic(), 1,
                            );
                            height += 1;
                        }
                        if total > limit {
                            body = body.child_sized(
                                Text::new(format!("   … +{} more lines", total - limit))
                                    .fg(colors::FG_MUTED).italic(),
                                1,
                            );
                            height += 1;
                        }
                    }
                    BlockLayout { height, view: vstack().child(body.class("ThinkingBlock")) }
                }
            }
        }

        // ── Tool Call ──
        // 修正：原 height 对带参返回 2，但 view 只有 1 行 → 统一 1 行。
        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            use crate::widget::blink::blink_visible;
            // claudecode 风格：⏺ 状态点。执行中 dimColor + 600ms 闪烁；
            // Done success 绿稳定（失败着色在 ToolResult 块）。
            let (dot, dot_color) = match phase {
                ToolPhase::Starting | ToolPhase::Running => {
                    let shown = if blink_visible(tick) { "⏺" } else { " " };
                    (shown, colors::FG_MUTED)
                }
                ToolPhase::Done => ("⏺", colors::E_TEAL),
            };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
            // 参数改 (params) 括号（claudecode 风格），去 `⚒ tool` 标签
            let params_disp = if params.is_empty() {
                String::new()
            } else if params.len() > 40 {
                format!("({}…)", &params.chars().take(37).collect::<String>())
            } else {
                format!("({})", params)
            };
            BlockLayout {
                height: 1,
                view: vstack().child(
                    hstack().gap(0)
                        .child_sized(Text::new(format!(" {} ", dot)).fg(dot_color), 3)
                        .child_sized(
                            Text::new(name_display.clone()).bold().fg(colors::FG_PRIMARY),
                            name_display.chars().count() as u16,
                        )
                        .child_flex(
                            Text::new(format!(" {}", params_disp)).fg(colors::FG_MUTED),
                            1.0,
                        ),
                ),
            }
        }

        // ── Tool Result ──
        TranscriptBlock::ToolResult { name, result, is_error, fold, .. } => {
            use crate::store::types::FoldState;
            let total_lines = result.lines().count();
            let (icon, accent) = crate::widget::status_icon::status_icon(
                if *is_error {
                    crate::widget::status_icon::Status::ResultError
                } else {
                    crate::widget::status_icon::Status::ResultOk
                }
            );
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
                            .child_sized(crate::widget::message_response::indented_prefix(), 5)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(
                                Text::new(format!(" · {} lines", total_lines))
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
                            .child_sized(crate::widget::message_response::indented_prefix(), 5)
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
                            .child_sized(crate::widget::message_response::indented_prefix(), 5)
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
                        let (icon, color) = crate::widget::status_icon::status_icon(
                            crate::widget::status_icon::Status::Todo(item.status)
                        );
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
                        let (icon, color) = crate::widget::status_icon::status_icon(
                            crate::widget::status_icon::Status::Todo(item.status)
                        );
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
            let (status_icon, status_color) = {
                use crate::widget::status_icon as si;
                si::status_icon(si::Status::Stage(si::stage_state(status)))
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


#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::store::types::{FoldState, TodoItem, TodoStatus, ToolPhase};

    fn blk(b: TranscriptBlock) -> BlockLayout { layout_block(&b, 0) }

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
    fn assistant_msg_empty_is_one_row() {
        let b = TranscriptBlock::AssistantMsg { id: "a".into(), content: String::new() };
        // ● 与 … 同行（hstack，单行）；修复旧版 ● 占 3 行的 height↔view 漂移。
        assert_eq!(blk(b).height, 1);
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
