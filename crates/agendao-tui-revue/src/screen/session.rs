//! Session Screen — renders transcript using Revue widgets.
//!
//! All blocks rendered via revue widgets (Text, Markdown, JsonViewer, Callout).
//! Fold state controls reveal of long content.
//! Colors use theme::colors for consistent Tokyo Night identity.

use revue::prelude::*;

use crate::store::types::*;
use crate::theme::colors;

const FOLD_PREVIEW_LINES: usize = 3;

/// 某类 block 的语义强调色——左竖线锚点色（app/mod.rs 渲染循环里每块左侧的
/// `▌`/`▎`），让竖线颜色反映该块角色。方案1（Carbon Obsidian）蓝本：角色
/// 区分靠竖线色相、不靠明度差（避免色块拼接显脏）。
///
/// 四色锚点（对齐方案1「一色一魂」，适中饱和——不极降以免褪色失焦）：
///   - USER   = E_TEAL         （窗光蓝：用户输入）
///   - ASSIST = ACCENT_PURPLE  （余烬紫：助手输出）
///   - TOOL   = E_AMBER         （铜锈金：工具调用/结果）
///   - 其余   = FG_MUTED       （think/stage/skill/todo/system/compact/image）
///
/// 颜色只承担「这条是谁说的」语义；次要角色一律 muted，不制造视觉噪音。
pub fn block_accent(block: &TranscriptBlock) -> revue::prelude::Color {
    use crate::theme::colors;
    match block {
        TranscriptBlock::UserPrompt { .. } => colors::E_TEAL,
        TranscriptBlock::AssistantMsg { .. } => colors::ACCENT_BLUE,
        TranscriptBlock::Thinking { .. } => colors::FG_MUTED,
        TranscriptBlock::ToolCall { .. } => colors::NORD_ORANGE,
        TranscriptBlock::ToolResult { .. } => colors::E_AMBER,
        TranscriptBlock::StageUpdate { .. } => colors::FG_MUTED,
        TranscriptBlock::SkillActivated { .. } => colors::FG_MUTED,
        TranscriptBlock::TodoList { .. } => colors::FG_MUTED,
        TranscriptBlock::CompactionHint { .. } => colors::FG_MUTED,
        TranscriptBlock::SystemNotice { .. } => colors::FG_MUTED,
        TranscriptBlock::ImageRef { .. } => colors::FG_MUTED,
    }
}

/// 块背景策略权威（Gemini 第三轮的背景色唯一来源）。`None` = 裸奔（保持终端
/// BG_PRIMARY 主背景——对话/思考/工具调用一律纯文本 + 飞白，指令#4）。
///
/// 第三轮极简化：所有普通对话文本背景 = 全屏背景。仅 `ToolResult` 保留
/// `BG_DEEP` 下沉深井（指令#2）。User 气泡不再有填充色——右靠限宽的纯文字
/// 即气泡形态（去 BG_SECONDARY 填充）。宽度/缩进/右断由 app 层几何分支决定，
/// 本函数只提供背景色唯一权威（金律：唯一成形语法），fill 由 `BgStack` 完成。
pub fn block_bg(block: &TranscriptBlock) -> Option<revue::prelude::Color> {
    use crate::theme::colors;
    match block {
        TranscriptBlock::ToolResult { .. } => Some(colors::BG_DEEP),
        _ => None,
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

/// 视觉单元 = 渲染 + 命中 + 高度的统一单位。一个或多个连续 `TranscriptBlock`
/// 合并而成（如连续 ToolResult / 连续 Thinking 聚合成井）。把「视觉聚合」从数据
/// 块里提出来成一等类型——渲染、鼠标命中、total_h 三处都遍历 `Vec<RenderUnit>`，
/// **不认块类型**；新增聚合种类只动 `build_render_units`（触点 1，金律）。
pub(crate) struct RenderUnit {
    /// 段首在 msgs 的索引。
    pub base_index: usize,
    /// 跨多少个块（单块 1，聚合组 n）。
    pub block_span: usize,
    pub height: u16,
    /// 井内/块内容（不含引导符/井几何/PAD——这些由渲染循环按包装属性加）。
    pub content: revue::widget::Stack,
    /// 每行归哪个组内块（相对 offset）。`None` = 装饰行（井沿等）不命中。单块恒
    /// `[Some(0); height]`。鼠标命中据此把屏幕 y 映射到块——行结构单点（渲染与
    /// 命中同源：row_owners 在渲染每一行时同步 push），消除「连续结果区域点不准」。
    pub row_owners: Vec<Option<usize>>,
    /// 引导符（`❯ ` 对话 / `    ┊ ` 其余）。
    pub glyph: &'static str,
    pub glyph_w: u16,
    pub accent: Color,
    /// 整块背景色（`BgStack`）；`None` = 无背景（保持终端 BG_PRIMARY）。
    pub bg: Option<Color>,
    /// 是否走「深井」包装几何（左缩进 2 + 右断 15%）。ToolResult 组/单块 true。
    pub is_well: bool,
}

/// `layout_*_group` 的产出：高度 + 视图 + 行→块映射（不含包装属性，包装由
/// `build_render_units` 统一填）。让聚合组的「行结构」与渲染同源。
pub(crate) struct GroupLayout {
    pub height: u16,
    pub view: revue::widget::Stack,
    pub row_owners: Vec<Option<usize>>,
}

pub fn layout_block(block: &TranscriptBlock, tick: u64) -> BlockLayout {
    layout_block_ctx(block, tick, false)
}

/// 连续 ToolResult 的「单井聚合」成形（Gemini 收敛性整改令 + 可逐条展开）。
///
/// 一次交互常冒出一串工具结果（如 skill_search 后逐个 skill_view，或并行多
/// 工具）。若每个都按独立深井 chip（`✓ result · name · N lines`）渲染，N 个
/// 叠起来就是「百叶窗」——Gemini 指出的密集恐惧源。本函数把一串连续
/// ToolResult 收进**一个**连贯深井：
///
/// ```text
/// ┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈
/// ℹ Found N results
///   1. ✓ name_a · 5 lines
/// ▶ 2. ✓ name_b · 12 lines        ← cursor 在此（▶ 标记 + 主色高亮）
///     ... detail lines ...         ← 该块 fold=Expanded 时展开详情
///   3. ✕ name_c · 8 lines
/// ┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈
/// ```
///
/// 交互复用既有 per-block cursor/fold：Up/Down(Tab) 逐块移动光标（停在组内
/// 每一条 ToolResult 上），Space 切当前块 fold——Expanded 项 inline 展开该
/// 结果详情。本函数只产出井内内容（顶/底虚线 + ℹ 汇总 + 各项含展开）与高度；
/// 深井背景（BG_DEEP）由 app 层 BgStack 包络。`base_index` = 段首在 msgs 的
/// 索引，用于把组内偏移换算成绝对 cursor 比较。金律：同一语义批次共享成形语法。

/// 单井聚合：连续 ToolResult 组项数超此阈值时折叠成前 N 项 + `[+N more]`。
/// layout（本模块 `layout_tool_result_group`）与 session_store 的 toggle 折叠
/// 判定共享此阈值——金律：成形阈值单点，避免两处魔法数漂移导致渲染与交互错位。
pub(crate) const TOOL_GROUP_PREVIEW: usize = 3;

/// 引导符（金律：成形符号单点）。按块类型赋语义符号（Gemini 终极符号令，反转
/// task #18 的「仅 ❯/┊ 归一」——多符号换取表意分量）：☻ 用户意图锚点 / ☯ AI
/// 知性输出 / ⚒ 工具刚性算子 / ⚇ 暗流思考 / ┊ 其余（ToolResult 井、Todo、Stage…）。
/// 均 2 宽、内容起点对齐（PAD 后第 2 列），缩进由全局 PAD 承担、符号紧贴气口。
fn block_glyph(block: &TranscriptBlock) -> (&'static str, u16) {
    match block {
        TranscriptBlock::UserPrompt { .. } => ("☻ ", 2),
        TranscriptBlock::AssistantMsg { .. } => ("☯ ", 2),
        TranscriptBlock::ToolCall { .. } => ("⚒ ", 2),
        TranscriptBlock::Thinking { .. } => ("⚇ ", 2),
        _ => ("┊ ", 2),
    }
}

/// 连续同类型段终点（exclusive end）。`pred` 判定段成员。聚合段检测的单点原语——
/// `tool_result_run` / `thinking_run` 复用，避免每类聚合重写一遍 while 循环。
fn group_run(msgs: &[TranscriptBlock], start: usize, pred: fn(&TranscriptBlock) -> bool) -> Option<usize> {
    if !pred(msgs.get(start)?) { return None; }
    let mut end = start + 1;
    while end < msgs.len() && pred(&msgs[end]) { end += 1; }
    Some(end)
}

/// 连续 ToolResult 段终点。供 `build_render_units` 判定聚合。
pub(crate) fn tool_result_run(msgs: &[TranscriptBlock], start: usize) -> Option<usize> {
    group_run(msgs, start, |b| matches!(b, TranscriptBlock::ToolResult { .. }))
}

/// 连续 Thinking 段终点。供 `build_render_units` 判定聚合。
fn thinking_run(msgs: &[TranscriptBlock], start: usize) -> Option<usize> {
    group_run(msgs, start, |b| matches!(b, TranscriptBlock::Thinking { .. }))
}

pub(crate) fn layout_tool_result_group(
    blocks: &[TranscriptBlock],
    base_index: usize,
    cursor_idx: Option<usize>,
) -> GroupLayout {
    use crate::store::types::FoldState;
    use crate::widget::status_icon::{status_icon, Status};
    let n = blocks.len();
    // 组级展开开关 = 段首块 fold（复用，免新增状态字段）。段首 Folded + 项数超
    // 阈值（TOOL_GROUP_PREVIEW）→ 折叠模式（前 N 项 + [+N more]）；否则展开全部
    // 项，每项可 per-block 展开详情。cursor 落在折叠组内时，toggle_fold_at_cursor
    // 切段首 fold 展开组。
    let head_expanded = matches!(
        blocks.first(),
        Some(TranscriptBlock::ToolResult { fold: FoldState::Expanded, .. })
    );
    let collapsed = !head_expanded && n > TOOL_GROUP_PREVIEW;
    let shown = if collapsed { TOOL_GROUP_PREVIEW.min(n) } else { n };

    let mut stack = vstack().gap(0);
    let mut height = 0u16;
    let mut row_owners: Vec<Option<usize>> = Vec::new();

    // 无顶/底横线（隐形边界令）：容器边界仅靠左侧 ┊ + 缩进表达，背景纯黑自然
    // 流入井上下方，消除「抽屉盒」感。ℹ 汇总即首行 → 命中段首（点击展开/折叠）。
    stack = stack.child_sized(
        Text::new(format!("ℹ Found {} results", n)).fg(colors::FG_MUTED).italic(),
        1,
    );
    height += 1;
    row_owners.push(Some(0));

    // 各项：每条 ToolResult 一行；cursor 项 ▶ 高亮。仅展开模式才 per-block 展开
    // 详情。每行同步 push row_owners——行结构单点（渲染与命中同源）。
    for (offset, block) in blocks.iter().enumerate().take(shown) {
        if let TranscriptBlock::ToolResult { name, result, is_error, fold, .. } = block {
            let is_cursor = cursor_idx == Some(base_index + offset);
            let (icon, _) = status_icon(if *is_error {
                Status::ResultError
            } else {
                Status::ResultOk
            });
            let total_lines = result.lines().count();
            let name_display = if name.len() > 24 {
                format!("{}…", &name.chars().take(21).collect::<String>())
            } else {
                name.clone()
            };
            let marker = if is_cursor { "▶" } else { " " };
            let item_color = if *is_error {
                colors::ACCENT_RED
            } else if is_cursor {
                colors::FG_PRIMARY
            } else {
                colors::FG_TRACE
            };
            let line = format!(
                "{:>2}. {} {} {} · {} lines",
                offset + 1, marker, icon, name_display, total_lines
            );
            stack = stack.child_sized(Text::new(line).fg(item_color), 1);
            height += 1;
            row_owners.push(Some(offset));

            // 展开项：详情行也归该项。
            if !collapsed && matches!(fold, FoldState::Expanded) {
                let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                for body_line in result.lines().take(20) {
                    stack = stack.child_sized(
                        Text::new(format!("    {}", body_line)).fg(body_color),
                        1,
                    );
                    height += 1;
                    row_owners.push(Some(offset));
                }
            }
        }
    }

    // 折叠模式：[+N more] 行 → 命中段首（展开整组）。
    if collapsed {
        let more = n - shown;
        let cursor_in_folded = cursor_idx
            .map(|c| c >= base_index + shown && c < base_index + n)
            .unwrap_or(false);
        let marker = if cursor_in_folded { "▶" } else { " " };
        let more_color = if cursor_in_folded { colors::FG_PRIMARY } else { colors::FG_TRACE };
        stack = stack.child_sized(
            Text::new(format!("{} … [+{} more · Space to expand]", marker, more))
                .fg(more_color)
                .italic(),
            1,
        );
        height += 1;
        row_owners.push(Some(0));
    }

    debug_assert_eq!(row_owners.len(), height as usize, "row_owners 必须与 height 同步");
    GroupLayout { height, view: stack, row_owners }
}

/// 连续 Thinking 的聚合井（与 `layout_tool_result_group` 同构）。reasoning model
/// 的「思考→工具→再思考」常产出一串 Thinking 块；聚合成一个井避免各自独立摘要
/// 叠成「百叶窗」。muted 色、无深色背景（`bg=None`），井几何由 `build_render_units`
/// 设 `is_well=true`。`row_owners` 行结构与 ToolResult 井一致。
pub(crate) fn layout_thinking_group(
    blocks: &[TranscriptBlock],
    base_index: usize,
    cursor_idx: Option<usize>,
) -> GroupLayout {
    use crate::store::types::FoldState;
    let n = blocks.len();
    let head_expanded = matches!(
        blocks.first(),
        Some(TranscriptBlock::Thinking { fold: FoldState::Expanded, .. })
    );
    let collapsed = !head_expanded && n > TOOL_GROUP_PREVIEW;
    let shown = if collapsed { TOOL_GROUP_PREVIEW.min(n) } else { n };

    let mut stack = vstack().gap(0);
    let mut height = 0u16;
    let mut row_owners: Vec<Option<usize>> = Vec::new();

    // 无顶/底横线（与 ToolResult 井同构，隐形边界令）。
    stack = stack.child_sized(
        Text::new(format!("ℹ {} thoughts", n)).fg(colors::FG_MUTED).italic(),
        1,
    );
    height += 1;
    row_owners.push(Some(0));

    for (offset, block) in blocks.iter().enumerate().take(shown) {
        if let TranscriptBlock::Thinking { content, fold, duration_ms, .. } = block {
            let is_cursor = cursor_idx == Some(base_index + offset);
            let wc = content.split_whitespace().count();
            let summary = if *duration_ms > 0 {
                format!("thinking · {} words · {}ms", wc, duration_ms)
            } else {
                format!("thinking · {} words", wc)
            };
            let marker = if is_cursor { "▶" } else { " " };
            let item_color = if is_cursor { colors::FG_PRIMARY } else { colors::FG_TRACE };
            let line = format!("{:>2}. {} ⚇ {}", offset + 1, marker, summary);
            stack = stack.child_sized(Text::new(line).fg(item_color).italic(), 1);
            height += 1;
            row_owners.push(Some(offset));

            if !collapsed && matches!(fold, FoldState::Expanded) {
                for body_line in content.lines().take(20) {
                    stack = stack.child_sized(
                        Text::new(format!("    {}", body_line)).fg(colors::FG_TRACE).italic(),
                        1,
                    );
                    height += 1;
                    row_owners.push(Some(offset));
                }
            }
        }
    }

    if collapsed {
        let more = n - shown;
        let cursor_in_folded = cursor_idx
            .map(|c| c >= base_index + shown && c < base_index + n)
            .unwrap_or(false);
        let marker = if cursor_in_folded { "▶" } else { " " };
        let more_color = if cursor_in_folded { colors::FG_PRIMARY } else { colors::FG_TRACE };
        stack = stack.child_sized(
            Text::new(format!("{} … [+{} more · Space to expand]", marker, more))
                .fg(more_color)
                .italic(),
            1,
        );
        height += 1;
        row_owners.push(Some(0));
    }

    debug_assert_eq!(row_owners.len(), height as usize, "row_owners 必须与 height 同步");
    GroupLayout { height, view: stack, row_owners }
}

/// 把 msgs 折成视觉单元序列——聚合决策单点（金律：触点 1）。连续 ToolResult /
/// 连续 Thinking 各自聚合成井，其余逐块。包装属性（glyph/accent/bg/is_well）统一
/// 填入 unit；渲染循环、鼠标命中、total_h 都消费此序列，不认块类型。新增聚合种类：
/// 在此加一段 `*_run` 检测 + 一个 `layout_*_group`，余处不动。
pub(crate) fn build_render_units(
    msgs: &[TranscriptBlock],
    cursor_idx: Option<usize>,
    tick: u64,
    show_thinking: bool,
) -> Vec<RenderUnit> {
    let mut units = Vec::new();
    let mut i = 0usize;
    // turn 级思考状态机：UserPrompt 起新 turn（重置），Thinking 置位。单块 Thinking
    // 据此定续接符（✻ 首个 / ┆ 同 turn 内被夹断的后续）——与原渲染循环逐块维护一致。
    // 聚合井（◇）不看此位；只有未聚合的单块 Thinking 走 layout_block_ctx 用它。
    let mut turn_has_thinking = false;
    while i < msgs.len() {
        // 聚合决策：ToolResult 组优先，再 Thinking 组，否则单块。
        let (group, next): (Option<GroupLayout>, usize) =
            if let Some(end) = tool_result_run(msgs, i).filter(|&e| e - i >= 2) {
                (Some(layout_tool_result_group(&msgs[i..end], i, cursor_idx)), end)
            } else if show_thinking {
                if let Some(end) = thinking_run(msgs, i).filter(|&e| e - i >= 2) {
                    (Some(layout_thinking_group(&msgs[i..end], i, cursor_idx)), end)
                } else {
                    (None, i + 1)
                }
            } else {
                (None, i + 1)
            };
        let head = &msgs[i];
        let (glyph, glyph_w) = block_glyph(head);
        let thinking_continuation =
            matches!(head, TranscriptBlock::Thinking { .. }) && turn_has_thinking;
        if let Some(g) = group {
            units.push(RenderUnit {
                base_index: i,
                block_span: next - i,
                height: g.height,
                content: g.view,
                row_owners: g.row_owners,
                glyph,
                glyph_w,
                accent: block_accent(head),
                bg: block_bg(head),
                is_well: true,
            });
        } else {
            // show_thinking=false：单块 Thinking 不 push（0 高度，transcript 收紧）。
            // 聚合井已在 group 决策里跳过；turn 状态机仍在下方 match 推进。
            if !show_thinking && matches!(head, TranscriptBlock::Thinking { .. }) {
                // skip
            } else {
                let bl = layout_block_ctx(head, tick, thinking_continuation);
                units.push(RenderUnit {
                    base_index: i,
                    block_span: 1,
                    height: bl.height,
                    content: bl.view,
                    row_owners: vec![Some(0); bl.height as usize],
                    glyph,
                    glyph_w,
                    accent: block_accent(head),
                    bg: block_bg(head),
                    is_well: matches!(head, TranscriptBlock::ToolResult { .. }),
                });
            }
        }
        match head {
            TranscriptBlock::UserPrompt { .. } => turn_has_thinking = false,
            TranscriptBlock::Thinking { .. } => turn_has_thinking = true,
            _ => {}
        }
        i = next;
    }
    units
}

/// 聚合总高（与渲染同口径）。复用 `build_render_units`——高度口径单点（金律）。
/// cursor/tick 不影响高度，故传 `None`/0。show_thinking 与渲染同口径，否则高度错位。
/// compact_density 与渲染块间空行同口径：紧凑模式块间 0 间隔，gap 也为 0，
/// 否则 total_h 比实际渲染高 → max_offset 偏大 → 点击/scroll 漂移。
pub(crate) fn transcript_total_height(msgs: &[TranscriptBlock], show_thinking: bool, compact_density: bool) -> u16 {
    let units = build_render_units(msgs, None, 0, show_thinking);
    let total: u16 = units.iter().map(|u| u.height).sum();
    let gap = if compact_density { 0 } else { units.len() as u16 };
    total.saturating_add(gap).saturating_add(1)
}

/// 带上下文的成形版本。`thinking_continuation`：当前 Thinking 是否属于同一
/// turn 内已出现过的思考的延续（被中间的 text/tool 夹断）——若是，用 ` ┆ `
/// 续接符代替 ` ✻ `，避免 reasoning model 的「思考→工具→再思考」流被拆成
/// 一串重复的 ✻ 独立块。土（编排层 turn 上下文）生金（成形符号）。
pub(crate) fn layout_block_ctx(
    block: &TranscriptBlock,
    tick: u64,
    thinking_continuation: bool,
) -> BlockLayout {
    match block {
        // ── User Prompt ──
        // height 随行累加。修正：原 transcript_block_height 在 Truncated
        // total≤3 时返回 total+1（多 1 行空白），现以 view 实际行数为准。
        TranscriptBlock::UserPrompt { content, fold, .. } => {
            use crate::store::types::FoldState;
            let total = content.lines().count();
            // 符号归一（Gemini 第三轮）：去 ▸/▾ 折叠箭头 + You chip。引导符 ❯
            // 在 app 层统一加；折叠用文字 hint（无箭头字符污染）。纯文本裸奔——
            // 背景与全屏一致（block_bg 已返回 None）。
            let (body_text, more_hint) = match fold {
                FoldState::Folded => (String::new(), None),
                FoldState::Truncated if total > FOLD_PREVIEW_LINES => (
                    truncate_lines(content, FOLD_PREVIEW_LINES),
                    Some(format!("… +{} more lines · Space to expand", total - FOLD_PREVIEW_LINES)),
                ),
                FoldState::Truncated | FoldState::Expanded => (content.clone(), None),
            };
            let first_line = body_text.lines().next().unwrap_or("");
            let rest: Vec<&str> = body_text.lines().skip(1).collect();

            let mut stack = vstack().gap(0)
                .child_sized(
                    Text::new(first_line.to_string()).fg(colors::FG_PRIMARY),
                    1,
                );
            let mut height = 1u16;
            for line in &rest {
                stack = stack.child_sized(
                    Text::new(line.to_string()).fg(colors::FG_PRIMARY),
                    1,
                );
                height += 1;
            }
            if let Some(hint) = more_hint {
                stack = stack.child_sized(
                    Text::new(format!("  {}", hint))
                        .fg(colors::FG_MUTED).italic(),
                    1,
                );
                height += 1;
            }
            BlockLayout { height, view: stack }
        }

        // ── Assistant Message ──
        // 终端原生风格：● 圆点（text 色）+ markdown，无 chip 标签。
        // RevueMarkdown 构造一次，height 与 view 共享。
        TranscriptBlock::AssistantMsg { content, .. } => {
            // 符号清理（P1）：去 ● 前缀。角色锚点已由 app 层左竖线（紫）承担，
            // 无需块首再叠一个角色点（● 是纯角色冗余，别无折叠/状态/续接功能）。
            // markdown 直接顶格成形——呼应方案1（Carbon Obsidian）「AssistantMsg
            // 无独立角色符号，靠竖线 + 内容成形」。其余块首符号均保留功能语义：
            // ▸/▾（UserPrompt 折叠）、⏺（ToolCall 执行状态）、✻/┆（Thinking +
            // turn 续接）、⎿（ToolResult 子结果缩进）、◈（TodoList 标记）。
            if content.is_empty() {
                BlockLayout {
                    height: 1,
                    view: vstack().child(Text::new("…").fg(colors::FG_MUTED)),
                }
            } else {
                let mut md = crate::markdown::RevueMarkdown::new();
                md.set_content(content);
                let lines = md.line_count().max(1) as u16;
                BlockLayout { height: lines, view: md.as_stack() }
            }
        }

        // ── Thinking / Reasoning ──
        // 符号归一（Gemini 第三轮）：去 ✻/┆ marker。引导符 ┊ 在 app 层统一加
        // （工具/思考共用 `┊ `）。纯内容裸奔——Folded 给摘要，展开直接列推理行。
        // 原 ✻/┆ 的 turn 续接语义放弃，换取全局符号统一（仅 ❯ 与 ┊ 两种引导）。
        TranscriptBlock::Thinking { content, fold, duration_ms, .. } => {
            use crate::store::types::FoldState;
            let _ = thinking_continuation; // 续接符已废，参数保留兼容签名
            let wc = content.split_whitespace().count();
            match fold {
                FoldState::Folded => {
                    let summary = if *duration_ms > 0 {
                        format!("thinking · {} words · {}ms", wc, duration_ms)
                    } else {
                        format!("thinking · {} words", wc)
                    };
                    BlockLayout {
                        height: 1,
                        view: vstack().child(
                            Text::new(summary).fg(colors::FG_MUTED).italic(),
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
                            Text::new("…".to_string()).fg(colors::FG_MUTED).italic(), 1,
                        );
                        height = 1;
                    } else {
                        for line in content.lines().take(limit) {
                            body = body.child_sized(
                                Text::new(line.to_string()).fg(colors::FG_MUTED).italic(), 1,
                            );
                            height += 1;
                        }
                        if total > limit {
                            body = body.child_sized(
                                Text::new(format!("  … +{} more lines", total - limit))
                                    .fg(colors::FG_MUTED).italic(),
                                1,
                            );
                            height += 1;
                        }
                    }
                    BlockLayout { height, view: vstack().child(body) }
                }
            }
        }

        // ── Tool Call ──
        // 符号归一（Gemini 第三轮）：去 ⏺ 状态点。引导符 ┊ 在 app 层统一加。
        // 执行状态改由 name 色变表达：执行中 muted（闪烁时隐入背景呼吸）/ Done 琥珀。
        TranscriptBlock::ToolCall { name, params, phase, .. } => {
            use crate::widget::blink::blink_visible;
            let name_color = match phase {
                ToolPhase::Starting | ToolPhase::Running => {
                    if blink_visible(tick) { colors::FG_MUTED } else { colors::BG_PRIMARY }
                }
                ToolPhase::Done => colors::E_AMBER,
            };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
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
                        .child_sized(
                            Text::new(name_display.clone()).bold().fg(name_color),
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
        // 符号归一（Gemini 第三轮）：去 ⎿ indented_prefix。引导符 ┊ 在 app 层统一加，
        // 深井左缩进 6 提供层级。header 直接 "result · name · N lines · icon"。
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
            let name_w = name_display.chars().count() as u16 + 3;
            match fold {
                FoldState::Folded => BlockLayout {
                    height: 1,
                    view: vstack().child(
                        hstack().gap(0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(
                                Text::new(format!(" · {} lines", total_lines))
                                    .fg(colors::FG_MUTED),
                                1.0,
                            ),
                    ),
                },
                FoldState::Truncated => {
                    let body_color = if *is_error { colors::ACCENT_RED } else { colors::FG_SECONDARY };
                    let limit = FOLD_PREVIEW_LINES.min(total_lines);
                    let mut stack = vstack().gap(0).child_sized(
                        hstack().gap(0)
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(Text::new(""), 1.0),
                        1,
                    );
                    let mut height = 1u16;
                    for line in result.lines().take(limit) {
                        stack = stack.child_sized(Text::new(format!("{}", line)).fg(body_color), 1);
                        height += 1;
                    }
                    if total_lines > limit {
                        stack = stack.child_sized(
                            Text::new(format!("  … +{} more lines", total_lines - limit))
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
                            .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2)
                            .child_sized(Text::new("result").fg(colors::E_AMBER).italic(), 6)
                            .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY), name_w)
                            .child_flex(Text::new(""), 1.0),
                        1,
                    );
                    let mut height = 1u16;
                    for line in result.lines().take(view_lines) {
                        stack = stack.child_sized(Text::new(format!("{}", line)).fg(body_color), 1);
                        height += 1;
                    }
                    if total_lines > view_lines {
                        stack = stack.child_sized(
                            Text::new(format!("  … +{} more lines", total_lines - view_lines))
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
            let mut header = String::from("Tasks");
            if let Some(ref s) = summary {
                if !s.phase.is_empty() { header.push_str(&format!(": {}", s.phase)); }
                if !s.duration.is_empty() { header.push_str(&format!(" · {}", s.duration)); }
                if !s.tokens.is_empty() { header.push_str(&format!(" · {}", s.tokens)); }
            }
            let mut s = vstack().gap(0)
                .child_sized(Text::new(header).fg(colors::FG_MUTED).bold(), 1);
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
            BlockLayout { height, view: s }
        }

        // ── Skill Activated ──
        TranscriptBlock::SkillActivated { name, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!("skill · {}", name)).fg(colors::FG_MUTED)),
        },

        // ── Stage Update ──
        // ◆ 符号标记（与 ◈ Todo / ⎿ ToolResult 同族轻量容器），去四面框：
        // 统一容器手法（金律——唯一成形语法），避免框/背景/竖线三种混用。
        // height = 标题行(1) + 非空 metadata 行数。
        TranscriptBlock::StageUpdate { name, status, metadata, .. } => {
            let (status_icon, status_color) = {
                use crate::widget::status_icon as si;
                si::status_icon(si::Status::Stage(si::stage_state(status)))
            };
            let mut stack = vstack().gap(0).child_sized(
                hstack().gap(1)
                    .child_sized(Text::new(" ◆ ").fg(colors::FG_MUTED), 3)
                    .child(Text::new(format!("stage · {}", name)).bold().fg(colors::FG_PRIMARY))
                    .child(Text::new(format!(" {} {}", status_icon, status)).fg(status_color)),
                1,
            );
            let mut height = 1u16;
            if let Some(ref detail) = metadata {
                for line in detail.lines() {
                    if line.is_empty() { continue; }
                    stack = stack.child_sized(
                        Text::new(format!("   {}", line)).fg(colors::FG_MUTED), 1,
                    );
                    height += 1;
                }
            }
            BlockLayout { height, view: stack }
        }

        // ── Compaction Hint ──
        TranscriptBlock::CompactionHint { before_tokens, after_tokens, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(
                format!("compact · {} → {} tokens", before_tokens, after_tokens),
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
            view: vstack().child(Text::new(format!("[{}]", mime)).fg(colors::FG_MUTED)),
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

    fn tr(name: &str, result: &str, fold: FoldState) -> TranscriptBlock {
        TranscriptBlock::ToolResult {
            id: name.into(),
            name: name.into(),
            result: result.into(),
            is_error: false,
            fold,
        }
    }

    /// 单井聚合的行→块映射（金律：行结构单点）。验证组内每行 row_owners 落到正确块——
    /// 这是「连续结果区域点击点不准」修复的核心不变量：行映射由 layout_*_group 渲染时
    /// 一次性产出，命中直接读取，渲染与命中真正同源（同一次函数调用）。
    #[test]
    fn group_row_owners_small_group_not_collapsed() {
        // 2 项（n ≤ 阈值，不折叠）：顶线 / ℹ / item0 / item1 / 底线。
        let blocks = vec![tr("a", "x", FoldState::Folded), tr("b", "y", FoldState::Folded)];
        let g = layout_tool_result_group(&blocks, 0, None);
        assert_eq!(g.row_owners, vec![Some(0), Some(0), Some(1)]);
        assert_eq!(g.height, 3);
    }

    #[test]
    fn group_row_owners_collapsed_more_maps_head() {
        // 5 项（n > 阈值，head Folded → 折叠）：顶线/ℹ/3 项/[+more]/底线。
        let blocks: Vec<_> = (0..5).map(|i| tr(&format!("n{}", i), "r", FoldState::Folded)).collect();
        let g = layout_tool_result_group(&blocks, 0, None);
        // [+more] 行归段首 Some(0)（点击展开整组）。
        assert_eq!(g.row_owners, vec![Some(0), Some(0), Some(1), Some(2), Some(0)]);
        assert_eq!(g.height, 5);
    }

    #[test]
    fn group_row_owners_expanded_detail_maps_owner() {
        // 2 项展开，item0 Expanded（2 行详情）：item0 的详情行也归 item0。
        let blocks = vec![
            tr("a", "line1\nline2", FoldState::Expanded),
            tr("b", "y", FoldState::Folded),
        ];
        let g = layout_tool_result_group(&blocks, 0, None);
        // ℹ/item0标题/item0详情1/item0详情2/item1（无横线装饰行）。
        assert_eq!(g.row_owners, vec![Some(0), Some(0), Some(0), Some(0), Some(1)]);
        assert_eq!(g.height, 5);
    }

    #[test]
    fn thinking_group_row_owners_collapsed() {
        // 5 连续 Thinking（n > 阈值，折叠）：行结构与 ToolResult 井同构（金律：成形
        // 语法唯一，聚合种类复用同一行结构）。
        let mk = || TranscriptBlock::Thinking {
            id: "m".into(), content: "word".into(), fold: FoldState::Folded, duration_ms: 0,
        };
        let blocks: Vec<_> = (0..5).map(|_| mk()).collect();
        let g = layout_thinking_group(&blocks, 0, None);
        assert_eq!(g.row_owners.clone(), vec![Some(0), Some(0), Some(1), Some(2), Some(0)]);
        assert_eq!(g.height, 5);
    }

    #[test]
    fn thinking_continuation_marker_preserves_height() {
        // 续接符（✻ → ┆）只改前缀符号，不改 block 高度。关键不变量：
        // total_h 用 layout_block（continuation=false）求和，主渲染循环用
        // layout_block_ctx（continuation 随 turn 变化），两者 height 必须一致，
        // 否则 scrollbar 命中映射与滚动 viewport 会错位。
        let mk = |fold: FoldState| TranscriptBlock::Thinking {
            id: "m1".into(),
            content: "思考第一行\n思考第二行".into(),
            fold,
            duration_ms: 0,
        };
        for fold in [FoldState::Folded, FoldState::Truncated, FoldState::Expanded] {
            let b = mk(fold.clone());
            assert_eq!(
                layout_block_ctx(&b, 0, false).height,
                layout_block_ctx(&b, 0, true).height,
                "continuation marker must not change height for {fold:?}",
            );
        }
    }

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
        assert_eq!(blk(b).height, 3); // 标题行(1) + 2 metadata（去框后）
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
