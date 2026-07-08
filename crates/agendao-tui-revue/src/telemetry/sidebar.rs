//! 水 — SessionSidebar：全高左列（logo + tab 轨道 + 详情 + Session Tree）。
//!
//! Ctrl+B toggle 显隐。结构（深川·流白，呼吸感 + 紧凑）：
//!   顶端空行 → logo(14字符art) → ⎻分隔 → tab符号行(|分隔) + 下划线轨道(整行-, active━)
//!   → 详情(选中tab: 标题 + -字段:值) → ⎻分隔 → ▣Session Tree → ⎻分隔 → session graph(flex)。
//! tab 6 符号（⏣Token ♺Cache ☪Context ⚒Tools ⚔MCP ✎Price）；Sessions 独立成底部常驻区
//!（会话树是导航，不参与 tab 切换）。下划线轨道：整行暗 -，active 处亮 ━（轨道感）。

use revue::prelude::*;
use crate::store::types::{
    ActiveTool, CacheStats, McpLspInfo, Pricing, SidebarTrees, ToolPhase, TokenUsage,
    TreeNode as SidebarNode, TreeIntent,
};
use crate::theme::colors;
use crate::theme::fmt_tokens;

/// Sidebar 构建器（unit struct——渲染走 build 关联函数，无实例状态）。
pub struct SessionSidebar;

/// Tab 数（= 符号数）——active_tab 上界。⏣/♺/☪/⚒/⚔/✎（Sessions 独立，不计入）。
pub const SIDEBAR_TAB_COUNT: usize = 6;

/// 底部用户栏 ⚙ 按钮命中列范围（含尾随空格，相对 sidebar 左边）：
/// `| ⛾  | ⚙  ` 从右起紧贴 sidebar 右边 11 列；⚙ + 两空格 = [W-3, W)。
/// 命中宽容包尾随空格——指点反应更稳，不挤占其它按钮区。
/// 几何单点（土律：可观测性）：keymap 通过此常量算绝对 x，sidebar 几何改动只
/// 触此处 + render 顺序，避免命中漂移。
pub const SIDEBAR_GEAR_X_FROM_END: u16 = 3;

impl SessionSidebar {
    /// 构建 sidebar 内容树。返回 (Stack, tab_y)：tab_y = 符号行绝对 y（sidebar 顶 y=0），供点击命中。
    pub fn build(
        token: &TokenUsage,
        cache: &CacheStats,
        price: &Pricing,
        ctx_pct: u8,
        trees: &SidebarTrees,
        mcp: &McpLspInfo,
        tools: &[ActiveTool],
        active_tab: usize,
    ) -> (revue::widget::Stack, u16) {
        let (logo_view, logo_h) = Self::logo();
        let (tab_view, tab_h) = Self::tab_bar(active_tab);
        let (detail_view, detail_h) = Self::detail(active_tab, token, cache, price, ctx_pct, mcp, tools);
        let session_header = Text::new("▣ Session Tree").fg(colors::FG_SECONDARY).bold();
        let graph = Self::session_graph(trees);
        // gap(0) + 显式空行 child：每处间距独立可控（土律：编排单点）。
        // 紧贴（0 行）：轨道↔详情、Session Tree↔分隔、分隔↔graph；其余 1 行。
        let sidebar = vstack().gap(0)
            .child_sized(Text::new(""), 2)              // 顶端 2 行空白（呼吸感）
            .child_sized(logo_view, logo_h)
            .child_sized(Text::new(""), 1)              // logo↔分隔 1 行
            .child_sized(Self::divider(), 1)            // logo 下分隔
            .child_sized(Text::new(""), 1)              // 分隔↔tab 1 行
            .child_sized(tab_view, tab_h)               // 符号行 + 下划线轨道
            .child_sized(detail_view, detail_h)         // 详情（紧贴轨道，0 行）
            .child_sized(Text::new(""), 1)              // 详情↔分隔 1 行
            .child_sized(Self::divider(), 1)            // 详情下分隔
            .child_sized(Text::new(""), 1)              // 分隔↔Session Tree 1 行
            .child_sized(session_header, 1)             // ▣ Session Tree
            .child_sized(Self::divider(), 1)            // 标题下分隔（紧贴标题，0 行）
            .child_flex(graph, 1.0)                     // session graph（紧贴分隔，0 行）
            .child_sized(Self::user_bar(), 1);          // 底部用户栏（将来功能占位）
        // tab 符号行 y = 顶端(2) + logo(4) + 空(1) + divider(1) + 空(1) = 9。
        let tab_y = 2 + logo_h + 1 + 1 + 1;
        (sidebar, tab_y)
    }

    /// 水平分隔线：⎻ × (SIDEBAR_WIDTH-1) FG_TRACE + 右留 1 列（呼吸感，不顶右边）。
    fn divider() -> revue::widget::Stack {
        let w = crate::app::SIDEBAR_WIDTH.saturating_sub(1);
        hstack().gap(0)
            .child_sized(Text::new("⎻".repeat(w as usize)).fg(colors::FG_TRACE), w)
            .child_flex(Text::new(""), 1.0)
    }

    // ── tab 符号栏 + 下划线轨道 ──

    /// 符号行 `| ⏣ | ♺ | ☪ | ⚒ | ⚔ | ✎ |`（| 竖线分隔错开，FG_MUTED）+ 下划线轨道
    /// （整行 - FG_TRACE 暗，active 符号处 ━ E_TEAL 亮）。每 tab = `| 符号 `(4 列)，
    /// 符号 i 在列 4i+2（命中: active = m.x / 4）。
    fn tab_bar(active: usize) -> (revue::widget::Stack, u16) {
        const SYMBOLS: [&str; SIDEBAR_TAB_COUNT] = ["⏣", "♺", "☪", "⚒", "⚔", "✎"];
        let mut row = hstack().gap(0);
        for s in SYMBOLS.iter() {
            // `| 符号 ` = |(1) + 空(1) + 符号(1) + 空(1) = 4 列/单元。
            row = row.child_sized(Text::new(format!("| {} ", s)).fg(colors::FG_MUTED), 4);
        }
        row = row.child_sized(Text::new("|").fg(colors::FG_MUTED), 1);
        row = row.child_flex(Text::new(""), 1.0);

        let bar = vstack().gap(0)
            .child_sized(row, 1)
            .child_sized(Self::underline_track(active), 1);
        (bar, 2)
    }

    /// 下划线轨道：整行 `-`（FG_TRACE 暗）+ active 符号处 `━`（E_TEAL 亮）。
    /// active 符号列 = 4i+2（| 空 之后）。整行宽 SIDEBAR_WIDTH-1（右留呼吸列）。
    fn underline_track(active: usize) -> revue::widget::Stack {
        let total = crate::app::SIDEBAR_WIDTH.saturating_sub(1);
        let ux = (2 + (active.min(SIDEBAR_TAB_COUNT - 1) * 4)) as u16;
        let before = ux;
        let after = total.saturating_sub(ux.saturating_add(1));
        hstack().gap(0)
            .child_sized(Text::new("-".repeat(before as usize)).fg(colors::FG_TRACE), before)
            .child_sized(Text::new("━").fg(colors::E_TEAL), 1)
            .child_sized(Text::new("-".repeat(after as usize)).fg(colors::FG_TRACE), after)
    }

    // ── 选中 tab 详情（标题 + -字段: 值，紧凑）──

    /// active tab 详情：标题（FG_SECONDARY bold）+ 字段行（` -字段: 值` FG_MUTED）。
    /// 顺序：0 Token / 1 Cache / 2 Context / 3 Tools / 4 MCP / 5 Price。
    fn detail(
        active: usize,
        token: &TokenUsage,
        cache: &CacheStats,
        price: &Pricing,
        ctx_pct: u8,
        mcp: &McpLspInfo,
        tools: &[ActiveTool],
    ) -> (revue::widget::Stack, u16) {
        let title = |t: &str| Text::new(t).fg(colors::FG_SECONDARY).bold();
        match active {
            0 => {
                let s = vstack().gap(0)
                    .child_sized(title("Token Usage"), 1)
                    .child_sized(Self::field("Input", &fmt_tokens(token.input)), 1)
                    .child_sized(Self::field("Output", &fmt_tokens(token.output)), 1)
                    .child_sized(Self::field("Total", &fmt_tokens(token.total)), 1);
                (s, 4)
            }
            1 => {
                let s = vstack().gap(0)
                    .child_sized(title("Cache"), 1)
                    .child_sized(Self::field("Hits", &cache.hits.to_string()), 1)
                    .child_sized(Self::field("Misses", &cache.misses.to_string()), 1)
                    .child_sized(Self::field("Writes", &cache.writes.to_string()), 1);
                (s, 4)
            }
            2 => {
                let s = vstack().gap(0)
                    .child_sized(title("Context"), 1)
                    .child_sized(Self::field("Used", &format!("{}%", ctx_pct)), 1)
                    .child_sized(Self::meter_bar(ctx_pct), 1);
                (s, 3)
            }
            3 => {
                let running = tools.iter().filter(|t| t.phase == ToolPhase::Running).count();
                let done = tools.iter().filter(|t| t.phase == ToolPhase::Done).count();
                let idle = tools.is_empty();
                let s = vstack().gap(0)
                    .child_sized(title("Tools"), 1)
                    .child_sized(Self::field("Running", &running.to_string()), 1)
                    .child_sized(Self::field("Done", &done.to_string()), 1)
                    .child_sized(
                        Text::new(if idle { " (idle)" } else { "" }).fg(colors::FG_TRACE),
                        1,
                    );
                (s, 4)
            }
            4 => {
                let lsp = if mcp.lsp_active.is_empty() {
                    "-".to_string()
                } else {
                    mcp.lsp_active.join(",")
                };
                let s = vstack().gap(0)
                    .child_sized(title("MCP/LSP"), 1)
                    .child_sized(Self::field("MCP", &format!("{}/{}", mcp.mcp_connected, mcp.mcp_total)), 1)
                    .child_sized(Self::field("LSP", &lsp), 1);
                (s, 3)
            }
            _ => {
                let s = vstack().gap(0)
                    .child_sized(title("Price"), 1)
                    .child_sized(Self::field("In", &format!("${:.4}/1k", price.input_per_1k)), 1)
                    .child_sized(Self::field("Out", &format!("${:.4}/1k", price.output_per_1k)), 1);
                (s, 3)
            }
        }
    }

    /// 字段行：` -字段(左对齐6): 值` FG_MUTED + trailing flex（右呼吸列）。
    fn field(name: &str, value: &str) -> revue::widget::Stack {
        let line = format!(" -{:<6}: {}", name, value);
        let w = line.chars().count() as u16;
        hstack().gap(0)
            .child_sized(Text::new(line).fg(colors::FG_MUTED), w)
            .child_flex(Text::new(""), 1.0)
    }

    /// 进度条（Context 用）：颜色随 pct 变（绿/黄/红）。
    fn meter_bar(pct: u8) -> revue::widget::Progress {
        let color = if pct > 80 { colors::ACCENT_RED }
                   else if pct > 50 { colors::ACCENT_YELLOW }
                   else { colors::ACCENT_GREEN };
        revue::widget::progress(pct as f32 / 100.0)
            .filled_color(color)
            .show_percentage(false)
    }

    // ── Session Tree（底部常驻，独立于 tab）──

    /// session graph：有会话 → 展平树（最多 30 项）；无 → (no sessions) 提示。
    fn session_graph(trees: &SidebarTrees) -> revue::widget::Stack {
        if trees.session_nodes.is_empty() {
            vstack().child_sized(Text::new(" (no sessions)").fg(colors::FG_TRACE), 1)
        } else {
            Self::tree_panel(&trees.session_nodes)
        }
    }

    // ── 底部用户栏（将来功能占位）──

    /// 底部用户栏：`☺ username` 靠左（首位留空，符号亮/名暗）· flex 楔子 · `| ⛾ | ⚙` 靠右
    /// （分隔暗、符号亮）。rugu=远程链接应用、gear=配置——功能后加，UI 先占位。
    /// username 取 $USER（取不到用 "user"）。画在 session graph 之下、sidebar 最底行。
    fn user_bar() -> revue::widget::Stack {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let user_w = user.chars().count() as u16;
        hstack().gap(0)
            .child_sized(Text::new(" ☺ ").fg(colors::FG_SECONDARY), 3)
            .child_sized(Text::new(user).fg(colors::FG_MUTED), user_w)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new("| ").fg(colors::FG_TRACE), 2)
            .child_sized(Text::new("⛾  ").fg(colors::FG_SECONDARY), 3)
            .child_sized(Text::new(" | ").fg(colors::FG_TRACE), 3)
            .child_sized(Text::new("⚙  ").fg(colors::FG_SECONDARY), 3)
    }

    /// Render tree nodes flat (max 30 items, indent via "  ".repeat(depth)).
    fn tree_panel(nodes: &[SidebarNode]) -> revue::widget::Stack {
        let mut lines: Vec<(String, Color)> = Vec::new();
        Self::flatten_nodes(nodes, &mut lines);
        let mut s = vstack();
        for (label, color) in lines.iter().take(30) {
            s = s.child_sized(Text::new(label.as_str()).fg(*color), 1);
        }
        s
    }

    fn flatten_nodes(nodes: &[SidebarNode], lines: &mut Vec<(String, Color)>) {
        for n in nodes {
            if lines.len() >= 30 { break; }
            let indent = "  ".repeat((n.depth as usize).min(6));
            let icon = if !n.children.is_empty() {
                if n.expanded { "▼ " } else { "▶ " }
            } else { "  " };
            let label = format!("{}{}{}", indent, icon, n.label);
            let color = match &n.intent {
                Some(TreeIntent::NavigateSession(_)) => colors::ACCENT_CYAN,
                Some(TreeIntent::OpenFile(_)) => colors::ACCENT_GREEN,
                None => colors::FG_SECONDARY,
            };
            lines.push((label, color));
            if n.expanded {
                Self::flatten_nodes(&n.children, lines);
            }
        }
    }

    // ── logo（深川明度阶，14 字符 art，右留呼吸列）──

    /// AGENDAO logo：4 行 ASCII art（14 字符宽，trailing flex 留呼吸列）。
    /// ░FG_TRACE / ▒FG_MUTED / █●○E_TEAL 明度阶（深川·流白景深），AGENDAO 翠青粗体。
    fn logo() -> (revue::widget::Stack, u16) {
        use colors::*;
        let r1 = Self::art_row(&[
            (FG_TRACE, "░░░", false), (FG_MUTED, "▒▒▒▒", false),
            (E_TEAL, "████", false), (FG_TRACE, "░░░", false),
        ]);
        let r2 = Self::art_row(&[
            (FG_MUTED, "▒▒▒▒▒▒▒", false), (E_TEAL, "███████", false),
            (FG_MUTED, "     ", false), (E_TEAL, "AGENDAO", true),
        ]);
        let r3 = Self::art_row(&[
            (FG_TRACE, "░░░", false), (FG_MUTED, "▒", false),
            (E_TEAL, "●", false), (FG_MUTED, "▒▒", false),
            (E_TEAL, "██", false), (E_TEAL, "○", false),
            (E_TEAL, "█", false), (FG_TRACE, "░░░", false),
            (FG_MUTED, "The Dao of Agent", false),
        ]);
        let r4 = Self::art_row(&[
            (FG_TRACE, "░░░░", false), (FG_MUTED, "▒▒▒", false),
            (E_TEAL, "███", false), (FG_TRACE, "░░░░", false),
        ]);
        let logo = vstack().gap(0)
            .child_sized(r1, 1)
            .child_sized(r2, 1)
            .child_sized(r3, 1)
            .child_sized(r4, 1);
        (logo, 4)
    }

    /// art 单行：按 (color, text, bold) 段顺序拼接，trailing flex 留白到容器宽（右呼吸列）。
    fn art_row(parts: &[(Color, &str, bool)]) -> revue::widget::Stack {
        let mut h = hstack().gap(0);
        for (c, t, bold) in parts {
            let w = t.chars().count() as u16;
            let mut txt = Text::new(*t).fg(*c);
            if *bold { txt = txt.bold(); }
            h = h.child_sized(txt, w);
        }
        h.child_flex(Text::new(""), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 对照：无空行 child——A@y0，B@y2（A:1 + gap:1）。
    #[test]
    fn gap_only_between_children() {
        let mut buf = Buffer::new(8, 8);
        let area = Rect::new(0, 0, 8, 8);
        let mut ctx = RenderContext::new(&mut buf, area);
        let s = vstack().gap(1)
            .child_sized(Text::new("A"), 1)
            .child_sized(Text::new("B"), 1);
        s.render(&mut ctx);
        assert_eq!(buf.get(0, 0).unwrap().symbol, 'A');
        assert_eq!(buf.get(0, 2).unwrap().symbol, 'B');
    }

    /// 验证空行 child_sized(Text::new(""), 1) 确实占 1 行高度——revue stack 按
    /// ChildSize::Fixed 分配（calculate_sizes 不 measure 内容），故空 Text 在
    /// stack 里与任意 Text 高度相同。有空行 → A@y0，空行@y2，B@y4（差 2 行）。
    #[test]
    fn empty_text_child_occupies_fixed_height() {
        let mut buf = Buffer::new(8, 8);
        let area = Rect::new(0, 0, 8, 8);
        let mut ctx = RenderContext::new(&mut buf, area);
        let s = vstack().gap(1)
            .child_sized(Text::new("A"), 1)
            .child_sized(Text::new(""), 1)
            .child_sized(Text::new("B"), 1);
        s.render(&mut ctx);
        assert_eq!(buf.get(0, 0).unwrap().symbol, 'A');
        // 空行生效 → B@y4；若空行是 no-op 则 B@y2（与对照测试冲突）。
        assert_eq!(buf.get(0, 4).unwrap().symbol, 'B');
    }
}
