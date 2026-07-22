//! Session Screen — renders transcript using Revue widgets.
//!
//! All blocks rendered via revue widgets (Text, Markdown, JsonViewer, Callout).
//! Fold state controls reveal of long content.
//! Colors use theme::colors for consistent Tokyo Night identity.

use revue::prelude::*;

use crate::store::types::*;
use crate::theme::colors;

const FOLD_PREVIEW_LINES: usize = 3;

/// ToolResult 的可见 body：diff 预览（edit/write/apply_patch）优先，
/// 否则 detail 纯文本。layout 与 group layout 共用（金律：口径单点）。
pub(crate) fn tool_result_body<'a>(result: &'a str, diff: &'a Option<DiffPreview>) -> &'a str {
    diff.as_ref().map(|d| d.text.as_str()).unwrap_or(result)
}

/// Unified diff 行级着色（对齐 CLI `render_diff_preview` 口径）：
/// `+` 增行绿 / `-` 删行红 / `@@` hunk 头青 / `diff `、`index `、`---`、`+++`
/// 头行 muted / 上下文行默认正文色。`---`/`+++` 必须先于单字符前缀判定。
pub(crate) fn diff_line_color(line: &str) -> Color {
    if line.starts_with("+++")
        || line.starts_with("---")
        || line.starts_with("diff ")
        || line.starts_with("index ")
    {
        colors::FG_MUTED()
    } else if line.starts_with('+') {
        colors::ACCENT_GREEN()
    } else if line.starts_with('-') {
        colors::ACCENT_RED()
    } else if line.starts_with("@@") {
        colors::ACCENT_CYAN()
    } else {
        colors::FG_SECONDARY()
    }
}

/// ToolResult body 的折叠 hint（"+M more lines" / 服务端截断标注）。
/// `total` = body 总行数，`limit` = 本次实际渲染的行数。
/// 返回 None = 无需 hint 行（body 已完整且未被服务端截断）。
pub(crate) fn tool_result_hint(total: usize, limit: usize, server_truncated: bool) -> Option<String> {
    let suffix = if server_truncated { " · server-truncated" } else { "" };
    if total > limit {
        Some(format!("  … +{} more lines{}", total - limit, suffix))
    } else if server_truncated {
        Some(format!("  … server-truncated"))
    } else {
        None
    }
}

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
        TranscriptBlock::UserPrompt { .. } => colors::E_TEAL(),
        TranscriptBlock::AssistantMsg { .. } => colors::ACCENT_BLUE(),
        TranscriptBlock::Thinking { .. } => colors::FG_MUTED(),
        TranscriptBlock::ToolCall { .. } => colors::NORD_ORANGE(),
        TranscriptBlock::ToolResult { .. } => colors::E_AMBER(),
        TranscriptBlock::StageUpdate { .. } => colors::FG_MUTED(),
        TranscriptBlock::SkillActivated { .. } => colors::FG_MUTED(),
        TranscriptBlock::TodoList { .. } => colors::FG_MUTED(),
        TranscriptBlock::CompactionHint { .. } => colors::FG_MUTED(),
        TranscriptBlock::SystemNotice { .. } => colors::FG_MUTED(),
        TranscriptBlock::ImageRef { .. } => colors::FG_MUTED(),
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
        TranscriptBlock::ToolResult { .. } => Some(colors::BG_DEEP()),
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
    // 兼容入口：无宽度上下文时取典型内容宽 80（与 transcript 常见 inner_w 相当）。
    layout_block_ctx(block, tick, false, 80)
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
///
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
        Text::new(format!("ℹ Found {} results", n)).fg(colors::FG_MUTED()).italic(),
        1,
    );
    height += 1;
    row_owners.push(Some(0));

    // 各项：每条 ToolResult 一行；cursor 项 ▶ 高亮。仅展开模式才 per-block 展开
    // 详情。每行同步 push row_owners——行结构单点（渲染与命中同源）。
    for (offset, block) in blocks.iter().enumerate().take(shown) {
        if let TranscriptBlock::ToolResult { name, result, is_error, fold, diff, .. } = block {
            let is_cursor = cursor_idx == Some(base_index + offset);
            let (icon, _) = status_icon(if *is_error {
                Status::ResultError
            } else {
                Status::ResultOk
            });
            // body 口径与单块 layout 同源：diff 预览优先于 detail。
            let body = tool_result_body(result, diff);
            let total_lines = body.lines().count();
            let name_display = if name.len() > 24 {
                format!("{}…", &name.chars().take(21).collect::<String>())
            } else {
                name.clone()
            };
            let marker = if is_cursor { "▶" } else { " " };
            let item_color = if *is_error {
                colors::ACCENT_RED()
            } else if is_cursor {
                colors::FG_PRIMARY()
            } else {
                colors::FG_TRACE()
            };
            let kind_label = if diff.is_some() { "diff" } else { "result" };
            let line = format!(
                "{:>2}. {} {} {} {} · {} lines",
                offset + 1, marker, icon, kind_label, name_display, total_lines
            );
            stack = stack.child_sized(Text::new(line).fg(item_color), 1);
            height += 1;
            row_owners.push(Some(offset));

            // 展开项：详情行也归该项。diff 块按行前缀 ±/@@ 着色。
            if !collapsed && matches!(fold, FoldState::Expanded) {
                for body_line in body.lines().take(20) {
                    let line_color = if *is_error {
                        colors::ACCENT_RED()
                    } else if diff.is_some() {
                        diff_line_color(body_line)
                    } else {
                        colors::FG_SECONDARY()
                    };
                    stack = stack.child_sized(
                        Text::new(format!("    {}", body_line)).fg(line_color),
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
        let more_color = if cursor_in_folded { colors::FG_PRIMARY() } else { colors::FG_TRACE() };
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
        Text::new(format!("ℹ {} thoughts", n)).fg(colors::FG_MUTED()).italic(),
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
            let item_color = if is_cursor { colors::FG_PRIMARY() } else { colors::FG_TRACE() };
            let line = format!("{:>2}. {} ⚇ {}", offset + 1, marker, summary);
            stack = stack.child_sized(Text::new(line).fg(item_color).italic(), 1);
            height += 1;
            row_owners.push(Some(offset));

            if !collapsed && matches!(fold, FoldState::Expanded) {
                for body_line in content.lines().take(20) {
                    stack = stack.child_sized(
                        Text::new(format!("    {}", body_line)).fg(colors::FG_TRACE()).italic(),
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
        let more_color = if cursor_in_folded { colors::FG_PRIMARY() } else { colors::FG_TRACE() };
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

/// 视图层窗口范围（scroll_top + viewport_h 都是行单位，绝对坐标对齐全量布局原点）。
/// `build_render_units` 接 `Option<ViewportRange>`：`None` = 全量真布局（供
/// mouse 命中、tests）；`Some` = viewport 外不构造 view（占位）、只填高度元数据，
/// 把每帧 markdown parse 的代价从 O(N) 压到 O(viewport)。
pub struct ViewportRange {
    pub scroll_top: u16,
    pub viewport_h: u16,
}

/// 单元元数据（纯量）：聚合决策 + 高度 + glyph/accent/bg + 是否聚合组。
/// `build_render_units` 与 `transcript_total_height` 都消费——聚合决策与高度
/// 口径单点（金律：触点 1）。`detect_unit_at` 不构造 view、不持 `Stack`，故可
/// 在不需要 view 的路径（全量高度统计 / viewport 外占位 unit）零代价复用。
struct UnitSpan {
    /// 跨多少个块。
    span: usize,
    height: u16,
    glyph: &'static str,
    glyph_w: u16,
    accent: Color,
    bg: Option<Color>,
    is_well: bool,
    /// 是否聚合组（ToolResult 组 / Thinking 组）——构造 view 时分流。
    is_group: bool,
    /// 单块 Thinking 续接判定（在同 turn 内被夹断）——传给 layout_block_ctx。
    thinking_continuation: bool,
}

/// 检测 i 起点的视觉单元：聚合还是单块、高度多少、glyph/accent/bg 各是什么。
/// 纯量——不构造 view（聚合分支会调 layout_*_group 但只用其 height，丢 view）。
/// height==0 表示该 i 处不产 unit（show_thinking=false 下的单块 Thinking），
/// 调用方应跳过 push 但仍按 span 推进。
fn detect_unit_at(
    msgs: &[TranscriptBlock],
    i: usize,
    tick: u64,
    show_thinking: bool,
    turn_has_thinking: bool,
    text_width: u16,
) -> UnitSpan {
    let head = &msgs[i];
    let (glyph, glyph_w) = block_glyph(head);
    let accent = block_accent(head);
    let bg = block_bg(head);

    // 聚合决策：ToolResult 组优先，再 Thinking 组，否则单块。与原 build_render_units
    // 的判定逻辑完全同源——这里只算 height/span，view 在调用方按需构造。
    if let Some(end) = tool_result_run(msgs, i).filter(|&e| e - i >= 2) {
        let g = layout_tool_result_group(&msgs[i..end], i, None);
        return UnitSpan {
            span: end - i,
            height: g.height,
            glyph, glyph_w, accent, bg,
            is_well: true,
            is_group: true,
            thinking_continuation: false,
        };
    }
    if show_thinking {
        if let Some(end) = thinking_run(msgs, i).filter(|&e| e - i >= 2) {
            let g = layout_thinking_group(&msgs[i..end], i, None);
            return UnitSpan {
                span: end - i,
                height: g.height,
                glyph, glyph_w, accent, bg,
                is_well: true,
                is_group: true,
                thinking_continuation: false,
            };
        }
    }

    // 单块路径。show_thinking=false 下的单块 Thinking → height=0、span=1（跳过 push）。
    if !show_thinking && matches!(head, TranscriptBlock::Thinking { .. }) {
        return UnitSpan {
            span: 1,
            height: 0,
            glyph, glyph_w, accent, bg,
            is_well: false,
            is_group: false,
            thinking_continuation: false,
        };
    }

    let thinking_continuation =
        matches!(head, TranscriptBlock::Thinking { .. }) && turn_has_thinking;
    let bl = layout_block_ctx(head, tick, thinking_continuation, text_width);
    UnitSpan {
        span: 1,
        height: bl.height,
        glyph, glyph_w, accent, bg,
        is_well: matches!(head, TranscriptBlock::ToolResult { .. }),
        is_group: false,
        thinking_continuation,
    }
}

/// 把 msgs 折成视觉单元序列——聚合决策单点（金律：触点 1）。连续 ToolResult /
/// 连续 Thinking 各自聚合成井，其余逐块。包装属性（glyph/accent/bg/is_well）统一
/// 填入 unit；渲染循环、鼠标命中、total_h 都消费此序列，不认块类型。新增聚合种类：
/// 在此加一段 `*_run` 检测 + 一个 `layout_*_group`，余处不动。
///
/// `viewport=None`（老语义）：全量真布局——所有 unit 都有真 view + 真 row_owners。
/// 供鼠标命中（必须真 row_owners 才能映射 y→块）、tests 用。
///
/// `viewport=Some(r)`：viewport 外的 unit 用占位 view（空 vstack）+ `vec![Some(0); h]`
/// row_owners——绝不 paint 也不命中（viewport 外用户点不到），但高度 / glyph /
/// accent / bg 真实，scrollbar / scroll_top / cursor 计算照旧正确。
/// SAFETY_PAD 上下各多 layout 8 行，容许 1~2 行抖动不重布局。
///
/// 坐标系口径（金律：渲染/滚动/命中同一坐标）：`viewport.scroll_top` 由调用方从
/// `transcript_total_height`（含块间 gap 行）推出，渲染层 vstack 也在每个 unit 后
/// 插 1 行 gap（compact_density 时无）。因此本函数的累加坐标 `acc_top` 必须同步
/// 计入 gap 行——此前只累加 unit.height，长会话（gap 总数 > SAFETY_PAD + viewport
/// 余量）下窗口与所有 unit 错开，全部判成占位 → 整屏只剩图标没有文字。
pub(crate) fn build_render_units(
    msgs: &[TranscriptBlock],
    cursor_idx: Option<usize>,
    tick: u64,
    show_thinking: bool,
    viewport: Option<ViewportRange>,
    text_width: u16,
    compact_density: bool,
) -> Vec<RenderUnit> {
    /// viewport 上下 padding（行）：抗 1-2 行抖动 + 吸收调用方先于内联 dialog
    /// 估算 scroll_top 的小幅偏差（permission/question 内联块在 build 之后才追加，
    /// 致 viewport 起点可能下移 5~10 行；16 行余量覆盖之）。
    const SAFETY_PAD: u16 = 16;

    // 块间 gap 行高（与渲染循环 child_sized(Text::new(""), 1)、
    // transcript_total_height 的 `gap = count` 同口径）。
    let gap_row: u16 = if compact_density { 0 } else { 1 };

    let mut units = Vec::new();
    let mut i = 0usize;
    let mut acc_top: u16 = 0;
    // turn 级思考状态机：UserPrompt 起新 turn（重置），Thinking 置位。单块 Thinking
    // 据此定续接符（✻ 首个 / ┆ 同 turn 内被夹断的后续）——与原渲染循环逐块维护一致。
    let mut turn_has_thinking = false;

    while i < msgs.len() {
        let span = detect_unit_at(msgs, i, tick, show_thinking, turn_has_thinking, text_width);

        // 推进 turn 状态（无视后续是否 push）——与原循环同序。
        match &msgs[i] {
            TranscriptBlock::UserPrompt { .. } => turn_has_thinking = false,
            TranscriptBlock::Thinking { .. } => turn_has_thinking = true,
            _ => {}
        }

        // 0 高度（show_thinking=false 单块 Thinking）：仅推进 i，不 push、不累加 acc_top。
        if span.height == 0 {
            i += span.span;
            continue;
        }

        let unit_top = acc_top;
        let unit_bottom = acc_top.saturating_add(span.height);

        // 是否落在 viewport 窗口内（含 SAFETY_PAD）。None → 永远内，即全量布局。
        let inside = match &viewport {
            None => true,
            Some(v) => {
                let view_top = v.scroll_top.saturating_sub(SAFETY_PAD);
                let view_bottom = v.scroll_top
                    .saturating_add(v.viewport_h)
                    .saturating_add(SAFETY_PAD);
                unit_bottom > view_top && unit_top < view_bottom
            }
        };

        let (content, row_owners) = if inside {
            // 真布局：重新走聚合/单块分支构造 view + row_owners。
            if span.is_group {
                let end = i + span.span;
                let g = if matches!(&msgs[i], TranscriptBlock::ToolResult { .. }) {
                    layout_tool_result_group(&msgs[i..end], i, cursor_idx)
                } else {
                    layout_thinking_group(&msgs[i..end], i, cursor_idx)
                };
                (g.view, g.row_owners)
            } else {
                let bl = layout_block_ctx(&msgs[i], tick, span.thinking_continuation, text_width);
                let h = bl.height as usize;
                (bl.view, vec![Some(0); h])
            }
        } else {
            // 占位：空 stack（viewport 外不会被 paint），row_owners 全 Some(0) 维持
            // 命中契约（.len()==height）；此 unit 不在 viewport 内，用户也点不到。
            let placeholder = vstack().gap(0);
            (placeholder, vec![Some(0); span.height as usize])
        };

        units.push(RenderUnit {
            base_index: i,
            block_span: span.span,
            height: span.height,
            content,
            row_owners,
            glyph: span.glyph,
            glyph_w: span.glyph_w,
            accent: span.accent,
            bg: span.bg,
            is_well: span.is_well,
        });

        acc_top = unit_bottom.saturating_add(gap_row);
        i += span.span;
    }
    units
}

/// 聚合总高（与渲染同口径）。走纯量路径 `detect_unit_at`——零 view 构造。
/// show_thinking 与渲染同口径，否则高度错位。
/// compact_density 与渲染块间空行同口径：紧凑模式块间 0 间隔，gap 也为 0，
/// 否则 total_h 比实际渲染高 → max_offset 偏大 → 点击/scroll 漂移。
pub(crate) fn transcript_total_height(msgs: &[TranscriptBlock], show_thinking: bool, compact_density: bool, text_width: u16) -> u16 {
    let mut total: u16 = 0;
    let mut count: u16 = 0;
    let mut turn_has_thinking = false;
    let mut i = 0usize;
    while i < msgs.len() {
        let span = detect_unit_at(msgs, i, 0, show_thinking, turn_has_thinking, text_width);
        match &msgs[i] {
            TranscriptBlock::UserPrompt { .. } => turn_has_thinking = false,
            TranscriptBlock::Thinking { .. } => turn_has_thinking = true,
            _ => {}
        }
        if span.height > 0 {
            total = total.saturating_add(span.height);
            count = count.saturating_add(1);
        }
        i += span.span;
    }
    let gap = if compact_density { 0 } else { count };
    total.saturating_add(gap).saturating_add(1)
}

/// 带上下文的成形版本。`thinking_continuation`：当前 Thinking 是否属于同一
/// turn 内已出现过的思考的延续（被中间的 text/tool 夹断）——若是，用 ` ┆ `
/// 续接符代替 ` ✻ `，避免 reasoning model 的「思考→工具→再思考」流被拆成
/// 一串重复的 ✻ 独立块。土（编排层 turn 上下文）生金（成形符号）。
///
/// `text_width` = transcript 实际内容宽（markdown 行数估算与渲染同口径）。
pub(crate) fn layout_block_ctx(
    block: &TranscriptBlock,
    tick: u64,
    thinking_continuation: bool,
    text_width: u16,
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
                    Text::new(first_line.to_string()).fg(colors::FG_PRIMARY()),
                    1,
                );
            let mut height = 1u16;
            for line in &rest {
                stack = stack.child_sized(
                    Text::new(line.to_string()).fg(colors::FG_PRIMARY()),
                    1,
                );
                height += 1;
            }
            if let Some(hint) = more_hint {
                stack = stack.child_sized(
                    Text::new(format!("  {}", hint))
                        .fg(colors::FG_MUTED()).italic(),
                    1,
                );
                height += 1;
            }
            BlockLayout { height, view: stack }
        }

        // ── Assistant Message ──
        // 终端原生风格：● 圆点（text 色）+ markdown，无 chip 标签。
        // RevueMarkdown 构造一次，height 与 view 共享。
        TranscriptBlock::AssistantMsg { content, fold, .. } => {
            use crate::store::types::FoldState;
            // 符号清理（P1）：去 ● 前缀。角色锚点已由 app 层左竖线（紫）承担，
            // 无需块首再叠一个角色点（● 是纯角色冗余，别无折叠/状态/续接功能）。
            // markdown 直接顶格成形——呼应方案1（Carbon Obsidian）「AssistantMsg
            // 无独立角色符号，靠竖线 + 内容成形」。其余块首符号均保留功能语义：
            // ▸/▾（UserPrompt 折叠）、⏺（ToolCall 执行状态）、✻/┆（Thinking +
            // turn 续接）、⎿（ToolResult 子结果缩进）、◈（TodoList 标记）。
            if content.is_empty() {
                BlockLayout {
                    height: 1,
                    view: vstack().child(Text::new("…").fg(colors::FG_MUTED())),
                }
            } else {
                let total = content.lines().count();
                match fold {
                    // 全折：单行摘要（长回答默认 Truncated，此为第三态）。
                    FoldState::Folded => BlockLayout {
                        height: 1,
                        view: vstack().child(
                            Text::new(format!("answer · {} lines", total))
                                .fg(colors::FG_MUTED()).italic(),
                        ),
                    },
                    // 截断：前 N 行 markdown 预览 + "+M more lines" hint（点击/Space 展开）。
                    // 不用 truncate_lines——它自带 "..." 后缀行，会与下方 hint 重复。
                    FoldState::Truncated if total > FOLD_PREVIEW_LINES => {
                        let preview = content
                            .lines()
                            .take(FOLD_PREVIEW_LINES)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let mut md = crate::markdown::RevueMarkdown::new();
                        md.set_content(&preview, text_width);
                        let lines = md.line_count().max(1);
                        let view = vstack().gap(0)
                            .child_sized(md.as_stack(), lines)
                            .child_sized(
                                Text::new(format!("  … +{} more lines", total - FOLD_PREVIEW_LINES))
                                    .fg(colors::FG_MUTED()).italic(),
                                1,
                            );
                        BlockLayout { height: lines + 1, view }
                    }
                    FoldState::Truncated | FoldState::Expanded => {
                        let mut md = crate::markdown::RevueMarkdown::new();
                        md.set_content(content, text_width);
                        let lines = md.line_count().max(1);
                        BlockLayout { height: lines, view: md.as_stack() }
                    }
                }
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
                            Text::new(summary).fg(colors::FG_MUTED()).italic(),
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
                            Text::new("…".to_string()).fg(colors::FG_MUTED()).italic(), 1,
                        );
                        height = 1;
                    } else {
                        for line in content.lines().take(limit) {
                            body = body.child_sized(
                                Text::new(line.to_string()).fg(colors::FG_MUTED()).italic(), 1,
                            );
                            height += 1;
                        }
                        if total > limit {
                            body = body.child_sized(
                                Text::new(format!("  … +{} more lines", total - limit))
                                    .fg(colors::FG_MUTED()).italic(),
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
                    if blink_visible(tick) { colors::FG_MUTED() } else { colors::BG_PRIMARY() }
                }
                ToolPhase::Done => colors::E_AMBER(),
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
                            Text::new(format!(" {}", params_disp)).fg(colors::FG_MUTED()),
                            1.0,
                        ),
                ),
            }
        }

        // ── Tool Result ──
        // 符号归一（Gemini 第三轮）：去 ⎿ indented_prefix。引导符 ┊ 在 app 层统一加，
        // 深井左缩进 6 提供层级。header 直接 "result · name · N lines · icon"。
        // diff 预览（edit/write/apply_patch）：body 换为 unified diff 文本，行级 ±
        // 着色（diff_line_color），chip 标签改 "diff"；preview.truncated 时 hint
        // 注明 server-truncated（部分 diff 不得读作完整）。
        TranscriptBlock::ToolResult { name, result, is_error, fold, diff, .. } => {
            use crate::store::types::FoldState;
            let body = tool_result_body(result, diff);
            let server_truncated = diff.as_ref().map(|d| d.truncated).unwrap_or(false);
            let total_lines = body.lines().count();
            let (icon, accent) = crate::widget::status_icon::status_icon(
                if *is_error {
                    crate::widget::status_icon::Status::ResultError
                } else {
                    crate::widget::status_icon::Status::ResultOk
                }
            );
            let kind_label = if diff.is_some() { "diff" } else { "result" };
            let name_display = if name.len() > 20 {
                format!("{}…", &name.chars().take(17).collect::<String>())
            } else {
                name.clone()
            };
            let name_w = name_display.chars().count() as u16 + 3;
            // body 行着色：error 恒红；diff 按行前缀 ±/@@ 分类；普通 detail 统一正文色。
            let line_color = |line: &str| {
                if *is_error {
                    colors::ACCENT_RED()
                } else if diff.is_some() {
                    diff_line_color(line)
                } else {
                    colors::FG_SECONDARY()
                }
            };
            let header = || {
                hstack().gap(0)
                    .child_sized(Text::new(format!("{} ", icon)).fg(accent), 2)
                    .child_sized(Text::new(kind_label).fg(colors::E_AMBER()).italic(), 6)
                    .child_sized(Text::new(format!(" · {}", name_display)).fg(colors::FG_PRIMARY()), name_w)
            };
            match fold {
                FoldState::Folded => BlockLayout {
                    height: 1,
                    view: vstack().child(
                        header().child_flex(
                            Text::new(format!(" · {} lines", total_lines))
                                .fg(colors::FG_MUTED()),
                            1.0,
                        ),
                    ),
                },
                FoldState::Truncated => {
                    let limit = FOLD_PREVIEW_LINES.min(total_lines);
                    let hint = tool_result_hint(total_lines, limit, server_truncated);
                    let mut stack = vstack().gap(0).child_sized(
                        header().child_flex(Text::new(""), 1.0),
                        1,
                    );
                    let mut height = 1u16;
                    for line in body.lines().take(limit) {
                        stack = stack.child_sized(Text::new(line.to_string()).fg(line_color(line)), 1);
                        height += 1;
                    }
                    if let Some(hint) = hint {
                        stack = stack.child_sized(
                            Text::new(hint).fg(colors::FG_MUTED()).italic(),
                            1,
                        );
                        height += 1;
                    }
                    BlockLayout { height, view: stack }
                }
                FoldState::Expanded => {
                    let view_lines = total_lines.min(20);
                    let hint = tool_result_hint(total_lines, view_lines, server_truncated);
                    let mut stack = vstack().gap(0).child_sized(
                        header().child_flex(Text::new(""), 1.0),
                        1,
                    );
                    let mut height = 1u16;
                    for line in body.lines().take(view_lines) {
                        stack = stack.child_sized(Text::new(line.to_string()).fg(line_color(line)), 1);
                        height += 1;
                    }
                    if let Some(hint) = hint {
                        stack = stack.child_sized(
                            Text::new(hint).fg(colors::FG_MUTED()).italic(),
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
                .child_sized(Text::new(header).fg(colors::FG_MUTED()).bold(), 1);
            let mut height = 1u16;
            match fold {
                FoldState::Folded => {
                    s = s.child_sized(
                        Text::new(format!("  … {} pending, {} completed", pending, done))
                            .fg(colors::FG_MUTED()).italic(),
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
                                .fg(colors::FG_MUTED()).italic(),
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
            view: vstack().child(Text::new(format!("skill · {}", name)).fg(colors::FG_MUTED())),
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
                    .child_sized(Text::new(" ◆ ").fg(colors::FG_MUTED()), 3)
                    .child(Text::new(format!("stage · {}", name)).bold().fg(colors::FG_PRIMARY()))
                    .child(Text::new(format!(" {} {}", status_icon, status)).fg(status_color)),
                1,
            );
            let mut height = 1u16;
            if let Some(ref detail) = metadata {
                for line in detail.lines() {
                    if line.is_empty() { continue; }
                    stack = stack.child_sized(
                        Text::new(format!("   {}", line)).fg(colors::FG_MUTED()), 1,
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
            ).fg(colors::FG_MUTED()).italic()),
        },

        // ── System Notice ──
        TranscriptBlock::SystemNotice { text, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!(" ℹ  {}", text)).fg(colors::FG_MUTED())),
        },

        // ── Image Reference ──
        TranscriptBlock::ImageRef { mime, .. } => BlockLayout {
            height: 1,
            view: vstack().child(Text::new(format!("[{}]", mime)).fg(colors::FG_MUTED())),
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
    use crate::store::types::{DiffPreview, FoldState, TodoItem, TodoStatus, ToolPhase};

    fn blk(b: TranscriptBlock) -> BlockLayout { layout_block(&b, 0) }

    fn tr(name: &str, result: &str, fold: FoldState) -> TranscriptBlock {
        TranscriptBlock::ToolResult {
            id: name.into(),
            name: name.into(),
            result: result.into(),
            is_error: false,
            fold,
            diff: None,
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
                layout_block_ctx(&b, 0, false, 100).height,
                layout_block_ctx(&b, 0, true, 100).height,
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
        let b = TranscriptBlock::AssistantMsg { id: "a".into(), content: String::new(), fold: FoldState::Expanded };
        // ● 与 … 同行（hstack，单行）；修复旧版 ● 占 3 行的 height↔view 漂移。
        assert_eq!(blk(b).height, 1);
    }

    #[test]
    fn assistant_msg_with_content_at_least_two_rows() {
        let b = TranscriptBlock::AssistantMsg { id: "a".into(), content: "# hi\nbody".into(), fold: FoldState::Expanded };
        assert!(blk(b).height >= 2);
    }

    #[test]
    fn assistant_msg_truncated_previews_three_lines_plus_hint() {
        // 长回答默认 Truncated：3 行预览（markdown 行数）+ 1 行 "+M more lines" hint。
        // 与「同预览内容 Expanded」对比，不钉死 markdown 内部行数口径。
        let truncated = blk(TranscriptBlock::AssistantMsg {
            id: "a".into(),
            content: "l1\nl2\nl3\nl4\nl5".into(),
            fold: FoldState::Truncated,
        });
        let preview_only = blk(TranscriptBlock::AssistantMsg {
            id: "b".into(),
            content: "l1\nl2\nl3".into(),
            fold: FoldState::Expanded,
        });
        assert_eq!(truncated.height, preview_only.height + 1);
    }

    #[test]
    fn assistant_msg_folded_is_one_row() {
        let b = TranscriptBlock::AssistantMsg {
            id: "a".into(),
            content: "l1\nl2\nl3\nl4\nl5".into(),
            fold: FoldState::Folded,
        };
        assert_eq!(blk(b).height, 1);
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
            is_error: false, fold: FoldState::Folded, diff: None,
        };
        assert_eq!(blk(b).height, 1);
    }

    // ── diff 预览渲染（edit/write/apply_patch）─────────────────────────────

    fn diff_tr(result: &str, diff_text: &str, truncated: bool, fold: FoldState) -> TranscriptBlock {
        TranscriptBlock::ToolResult {
            id: "t".into(),
            name: "edit".into(),
            result: result.into(),
            is_error: false,
            fold,
            diff: Some(DiffPreview { text: diff_text.into(), truncated }),
        }
    }

    /// 行级着色分类（对齐 CLI render_diff_preview 口径）：+ 绿 / - 红 / @@ 青 /
    /// diff/index/---/+++ 头行 muted / 上下文行正文色。---/+++ 必须先于单字符前缀。
    #[test]
    fn diff_line_color_classifies_by_prefix() {
        assert_eq!(diff_line_color("+added"), colors::ACCENT_GREEN());
        assert_eq!(diff_line_color("-removed"), colors::ACCENT_RED());
        assert_eq!(diff_line_color("@@ -1,2 +1,2 @@"), colors::ACCENT_CYAN());
        assert_eq!(diff_line_color("--- a/f.rs"), colors::FG_MUTED());
        assert_eq!(diff_line_color("+++ b/f.rs"), colors::FG_MUTED());
        assert_eq!(diff_line_color("diff --git a/f b/f"), colors::FG_MUTED());
        assert_eq!(diff_line_color("index abc..def 100644"), colors::FG_MUTED());
        assert_eq!(diff_line_color(" context"), colors::FG_SECONDARY());
        assert_eq!(diff_line_color(""), colors::FG_SECONDARY());
    }

    /// diff 块 body = diff 文本（非 detail）：>3 行时 Truncated = header(1) +
    /// 3 预览行 + 1 hint；高度与 store 层 height() 估计同口径。
    #[test]
    fn diff_result_truncated_previews_three_lines_plus_hint() {
        let diff = "@@ -1,5 +1,5 @@\n-a\n+b\n-c\n+d\n ctx";
        let b = diff_tr("detail 不应成为 body", diff, false, FoldState::Truncated);
        assert_eq!(blk(b.clone()).height, 5, "header + 3 预览 + hint");
        assert_eq!(b.height(), 5, "store height() 必须与 layout 同口径");
    }

    /// diff 行数 ≤3 且无服务端截断：无 hint 行。
    #[test]
    fn diff_result_short_no_hint() {
        let b = diff_tr("", "+a\n-b", false, FoldState::Truncated);
        assert_eq!(blk(b.clone()).height, 3, "header + 2 body，无 hint");
        assert_eq!(b.height(), 3);
    }

    /// preview.truncated=true：即使 body 行数 ≤ 预览上限也要标注 server-truncated
    /// （部分 diff 不得读作完整），Expanded 同样标注。
    #[test]
    fn diff_result_server_truncated_always_hints() {
        let b = diff_tr("", "+a\n-b", true, FoldState::Truncated);
        assert_eq!(blk(b.clone()).height, 4, "header + 2 body + server-truncated 标注");
        assert_eq!(b.height(), 4);
        let b2 = diff_tr("", "+a\n-b", true, FoldState::Expanded);
        assert_eq!(blk(b2.clone()).height, 4, "Expanded 同样标注");
        assert_eq!(b2.height(), 4);
        // hint 文案单测（tool_result_hint 为唯一成形点）。
        assert_eq!(tool_result_hint(2, 2, true).as_deref(), Some("  … server-truncated"));
        assert_eq!(
            tool_result_hint(6, 3, true).as_deref(),
            Some("  … +3 more lines · server-truncated")
        );
        assert_eq!(tool_result_hint(2, 2, false), None);
    }

    /// Expanded 超 20 行：20 行 body + more-lines hint（带截断后缀）。
    #[test]
    fn diff_result_expanded_caps_at_20_lines() {
        let diff: String = (0..30).map(|i| format!("+line{i}")).collect::<Vec<_>>().join("\n");
        let b = diff_tr("", &diff, false, FoldState::Expanded);
        assert_eq!(blk(b.clone()).height, 22, "header + 20 body + hint");
        assert_eq!(b.height(), 22);
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

    // ── 视图层窗口化（render-side viewport windowing）─────────────────────
    //
    // 高度口径单点（detect_unit_at）。viewport=None 与 viewport=Some(...) 必产同形
    // unit 序列、同 height、同 row_owners.len()；唯一差异是 content（窗口外占位）。
    // total_height 走纯量路径，必须等于 Σ unit.height + count + 1。

    fn make_mixed_msgs() -> Vec<TranscriptBlock> {
        let mut v = Vec::new();
        // 多 turn 混合块：User / Assistant / ToolCall / ToolResult 组 / Thinking
        for turn in 0..5 {
            v.push(TranscriptBlock::UserPrompt {
                id: format!("u{}", turn),
                content: format!("question turn {}", turn),
                fold: FoldState::Expanded,
            });
            v.push(TranscriptBlock::AssistantMsg {
                id: format!("a{}", turn),
                content: format!("# header turn {}\nbody line 1\nbody line 2", turn),
                fold: FoldState::Expanded,
            });
            v.push(TranscriptBlock::ToolCall {
                id: format!("c{}_1", turn),
                name: "read".into(),
                params: "{}".into(),
                phase: ToolPhase::Done,
            });
            // 连续 ToolResult → 触发聚合井
            v.push(TranscriptBlock::ToolResult {
                id: format!("r{}_1", turn), name: "read".into(), result: "x".into(),
                is_error: false, fold: FoldState::Folded, diff: None,
            });
            v.push(TranscriptBlock::ToolResult {
                id: format!("r{}_2", turn), name: "read".into(), result: "y".into(),
                is_error: false, fold: FoldState::Folded, diff: None,
            });
        }
        v
    }

    #[test]
    fn viewport_window_matches_full_layout_heights() {
        let msgs = make_mixed_msgs();
        let full = build_render_units(&msgs, None, 0, true, None, 100, false);
        let windowed = build_render_units(
            &msgs, None, 0, true,
            Some(ViewportRange { scroll_top: 30, viewport_h: 20 }),
            100,
            false,
        );

        assert_eq!(full.len(), windowed.len(), "viewport 切换不应改变 unit 数");
        for (idx, (f, w)) in full.iter().zip(windowed.iter()).enumerate() {
            assert_eq!(f.height, w.height,
                "unit {} height 必须同口径（idx full={}, win={})", idx, f.height, w.height);
            assert_eq!(f.row_owners.len(), w.row_owners.len(),
                "unit {} row_owners.len() 必须 == height（命中契约）", idx);
            assert_eq!(f.row_owners.len(), f.height as usize,
                "unit {} row_owners.len() == height 自检", idx);
            assert_eq!(f.base_index, w.base_index, "unit {} base_index 漂移", idx);
            assert_eq!(f.block_span, w.block_span, "unit {} block_span 漂移", idx);
            assert_eq!(f.glyph, w.glyph, "unit {} glyph 漂移", idx);
            assert_eq!(f.is_well, w.is_well, "unit {} is_well 漂移", idx);
        }
        let total_full: u32 = full.iter().map(|u| u.height as u32).sum();
        let total_window: u32 = windowed.iter().map(|u| u.height as u32).sum();
        assert_eq!(total_full, total_window, "viewport 窗口必须保持总高一致");
    }

    #[test]
    fn total_height_matches_sum_of_units() {
        let msgs = make_mixed_msgs();
        // transcript_total_height 现走纯量 detect_unit_at —— 必须与全量 build_render_units
        // 的 Σ height + count + 1 等同（compact_density=false 下每 unit 后 1 行 gap）。
        let total = transcript_total_height(&msgs, true, false, 100);
        let units = build_render_units(&msgs, None, 0, true, None, 100, false);
        let sum_h: u16 = units.iter().filter(|u| u.height > 0)
            .map(|u| u.height).sum();
        let count = units.iter().filter(|u| u.height > 0).count() as u16;
        assert_eq!(total, sum_h.saturating_add(count).saturating_add(1));

        // compact_density=true 下无块间空行 gap 应为 0
        let total_compact = transcript_total_height(&msgs, true, true, 100);
        assert_eq!(total_compact, sum_h.saturating_add(1));
    }

    /// 回归（Bug A）：长 transcript 钉底时，可见窗口内的 unit 必须是真布局
    /// （能渲染出文本），不得是占位空 view。
    ///
    /// 修复前 `build_render_units` 的 `acc_top` 只累加 unit 高度、不计块间
    /// gap 行，而 `ViewportRange.scroll_top` 由 `transcript_total_height`
    /// （含 gap）推出——块数一多（gap 总数 > viewport + SAFETY_PAD）窗口与
    /// 所有 unit 错开，全部判成占位 → 旧会话整屏只剩图标没有文字。
    #[test]
    fn bottom_viewport_layouts_real_content_for_long_transcripts() {
        let mut msgs = Vec::new();
        for i in 0..120 {
            msgs.push(TranscriptBlock::UserPrompt {
                id: format!("u{i}"),
                content: format!("question {i}"),
                fold: FoldState::Expanded,
            });
            msgs.push(TranscriptBlock::AssistantMsg {
                id: format!("a{i}"),
                content: format!("answer {i}"),
                fold: FoldState::Expanded,
            });
        }
        let text_width: u16 = 100;
        let available: u16 = 41;
        let total_h = transcript_total_height(&msgs, true, false, text_width);
        let scroll_top = total_h.saturating_sub(available);
        let units = build_render_units(
            &msgs, None, 0, true,
            Some(ViewportRange { scroll_top, viewport_h: available }),
            text_width, false,
        );
        // 末尾 unit（钉底时必在可见窗口内）渲染其 content，必须出现文本字符。
        let last = units.last().expect("units 非空");
        let h = last.height.max(1);
        let mut buf = Buffer::new(120, h);
        let area = Rect::new(0, 0, 120, h);
        let mut ctx = RenderContext::new(&mut buf, area);
        last.content.render(&mut ctx);
        let any_text = (0..h).any(|y| {
            (0..120).any(|x| buf.get(x, y).map(|c| c.symbol != ' ').unwrap_or(false))
        });
        assert!(any_text, "钉底可见 unit 必须是真布局（修复前为占位空 view → 只剩图标）");
    }
}
