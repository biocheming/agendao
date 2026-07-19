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

/// Sidebar session tree 的一个可点击导航命中(阳面命中口径)。
/// `y` = 该行绝对屏幕 y(sidebar 顶 y=0);`session_id` = 点击后要打开的会话。
/// build 渲染时算好并随返回值发布,keymap click 据此 hit-test(水生木:
/// 会话树不再只上色,而是能回到"打开会话"这一输入动作)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarNavHit {
    pub y: u16,
    pub session_id: String,
    /// 节点深度（箭头列 = depth*2 起的 2 列，与 flatten_nodes 缩进口径同源）。
    pub depth: u8,
    /// 是否有子节点（决定是否可展开/折叠）。
    pub has_children: bool,
}

/// 展平后的一行(渲染 + 命中同源:label/color 给渲染,intent 给命中)。
struct FlatRow {
    label: String,
    color: Color,
    intent: Option<TreeIntent>,
    depth: u8,
    has_children: bool,
}

impl SessionSidebar {
    /// 构建 sidebar 内容树。返回 `(Stack, tab_y, nav_hits)`:
    ///   - `tab_y` = tab 符号行绝对 y(点击切 tab 命中)
    ///   - `nav_hits` = session tree 里 NavigateSession 行的 (绝对 y, session_id)
    ///     列表(点击打开会话命中)。渲染与命中同源(金律·成形语法唯一)。
    // 渲染入口：聚合 sidebar 所需的全部遥测数据源（token/cache/价格/树/MCP/
    // 工具/tab 状态），各源类型异构、来自不同子系统，平铺传参最直白。
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        token: &TokenUsage,
        cache: &CacheStats,
        price: &Pricing,
        ctx_pct: u8,
        trees: &SidebarTrees,
        mcp: &McpLspInfo,
        tools: &[ActiveTool],
        active_tab: usize,
        active_session_id: Option<&str>,
        viewport_height: u16,
    ) -> (revue::widget::Stack, u16, Vec<SidebarNavHit>) {
        let (logo_view, logo_h) = Self::logo();
        let (tab_view, tab_h) = Self::tab_bar(active_tab);
        let (detail_view, detail_h) = Self::detail(active_tab, token, cache, price, ctx_pct, mcp, tools);
        let session_header = Text::new("▣ Session Tree").fg(colors::FG_SECONDARY()).bold();

        // Session graph 起始绝对 y(固定高度累加):
        //   顶空(2) + logo(logo_h) + 空(1) + 分隔(1) + 空(1) + tab(tab_h) + detail(detail_h)
        //   + 空(1) + 分隔(1) + 空(1) + header(1) + 分隔(1)
        // = 2 + logo_h + 3 + tab_h + detail_h + 5。logo_h=4/tab_h=2 → 16 + detail_h。
        let graph_start_y = 2 + logo_h + 3 + tab_h + detail_h + 5;
        // 可视行数 = 总高 - graph 起点 - user_bar(1)。渲染侧 revue 会把溢出行裁掉，
        // 命中区必须同口径裁剪——否则屏幕外的「幽灵」session 行会盖住 user_bar
        // （⚙ 点击被 open_session 拦截）乃至整列底部区域（金律：渲染/命中同一份展平）。
        let graph_visible = viewport_height.saturating_sub(graph_start_y + 1) as usize;
        let (graph, nav_hits) = Self::session_graph(trees, graph_start_y, active_session_id, graph_visible);

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
        (sidebar, tab_y, nav_hits)
    }

    /// 水平分隔线：⎻ × (SIDEBAR_WIDTH-1) FG_TRACE + 右留 1 列（呼吸感，不顶右边）。
    fn divider() -> revue::widget::Stack {
        let w = crate::app::SIDEBAR_WIDTH.saturating_sub(1);
        hstack().gap(0)
            .child_sized(Text::new("⎻".repeat(w as usize)).fg(colors::FG_TRACE()), w)
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
            row = row.child_sized(Text::new(format!("| {} ", s)).fg(colors::FG_MUTED()), 4);
        }
        row = row.child_sized(Text::new("|").fg(colors::FG_MUTED()), 1);
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
            .child_sized(Text::new("-".repeat(before as usize)).fg(colors::FG_TRACE()), before)
            .child_sized(Text::new("━").fg(colors::E_TEAL()), 1)
            .child_sized(Text::new("-".repeat(after as usize)).fg(colors::FG_TRACE()), after)
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
        let title = |t: &str| Text::new(t).fg(colors::FG_SECONDARY()).bold();
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
                        Text::new(if idle { " (idle)" } else { "" }).fg(colors::FG_TRACE()),
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
            .child_sized(Text::new(line).fg(colors::FG_MUTED()), w)
            .child_flex(Text::new(""), 1.0)
    }

    /// 进度条（Context 用）：颜色随 pct 变（绿/黄/红）。
    fn meter_bar(pct: u8) -> revue::widget::Progress {
        let color = if pct > 80 { colors::ACCENT_RED() }
                   else if pct > 50 { colors::ACCENT_YELLOW() }
                   else { colors::ACCENT_GREEN() };
        revue::widget::progress(pct as f32 / 100.0)
            .filled_color(color)
            .show_percentage(false)
    }

    // ── Session Tree（底部常驻，独立于 tab）──

    /// session graph：有会话 → 展平树（最多 min(30, visible_rows) 项）；无 →
    /// (no sessions) 提示。`graph_start_y` = 该 graph 首行绝对 y,用于把每个
    /// NavigateSession 行换算成点击命中 y。`visible_rows` = flex 槽实际可视行数,
    /// 超出的行渲染/命中同步裁掉（金律·两者同一份展平结果）。
    fn session_graph(
        trees: &SidebarTrees,
        graph_start_y: u16,
        active_session_id: Option<&str>,
        visible_rows: usize,
    ) -> (revue::widget::Stack, Vec<SidebarNavHit>) {
        let mut flat: Vec<FlatRow> = Vec::new();
        if !trees.session_nodes.is_empty() {
            Self::flatten_nodes(&trees.session_nodes, &mut flat, active_session_id);
        }
        flat.truncate(30.min(visible_rows));

        if flat.is_empty() {
            let s = vstack().gap(0)
                .child_sized(Text::new(" (no sessions)").fg(colors::FG_TRACE()), 1);
            return (s, Vec::new());
        }

        // gap(0):行连续,命中 y = graph_start_y + i 才成立(金律·成形语法唯一)。
        let mut s = vstack().gap(0);
        let mut hits = Vec::new();
        for (i, row) in flat.iter().enumerate() {
            s = s.child_sized(Text::new(row.label.as_str()).fg(row.color), 1);
            if let Some(TreeIntent::NavigateSession(id)) = &row.intent {
                hits.push(SidebarNavHit {
                    y: graph_start_y + i as u16,
                    session_id: id.clone(),
                    depth: row.depth,
                    has_children: row.has_children,
                });
            }
        }
        (s, hits)
    }

    // ── 底部用户栏（将来功能占位）──

    /// 底部用户栏：`☺ username` 靠左（首位留空，符号亮/名暗）· flex 楔子 · `| ⛾ | ⚙` 靠右
    /// （分隔暗、符号亮）。rugu=远程链接应用、gear=配置——功能后加，UI 先占位。
    /// username 取 $USER（取不到用 "user"）。画在 session graph 之下、sidebar 最底行。
    fn user_bar() -> revue::widget::Stack {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let user_w = user.chars().count() as u16;
        hstack().gap(0)
            .child_sized(Text::new(" ☺ ").fg(colors::FG_SECONDARY()), 3)
            .child_sized(Text::new(user).fg(colors::FG_MUTED()), user_w)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new("| ").fg(colors::FG_TRACE()), 2)
            .child_sized(Text::new("⛾  ").fg(colors::FG_SECONDARY()), 3)
            .child_sized(Text::new(" | ").fg(colors::FG_TRACE()), 3)
            .child_sized(Text::new("⚙  ").fg(colors::FG_SECONDARY()), 3)
    }

    /// 展平树节点为 FlatRow(label + color + intent),深度用缩进表达,最多 30 项。
    /// 渲染与点击命中共用此结果——单点权威,不会出现"看得见但点不中"的错位。
    fn flatten_nodes(
        nodes: &[SidebarNode],
        rows: &mut Vec<FlatRow>,
        active_session_id: Option<&str>,
    ) {
        for n in nodes {
            if rows.len() >= 30 { break; }
            let indent = "  ".repeat((n.depth as usize).min(6));
            let icon = if !n.children.is_empty() {
                if n.expanded { "▼ " } else { "▶ " }
            } else { "  " };
            let label = format!("{}{}{}", indent, icon, n.label);
            let color = match &n.intent {
                Some(TreeIntent::NavigateSession(id)) if active_session_id == Some(id.as_str()) => {
                    colors::E_AMBER()
                }
                Some(TreeIntent::NavigateSession(_)) => colors::ACCENT_CYAN(),
                Some(TreeIntent::OpenFile(_)) => colors::ACCENT_GREEN(),
                None => colors::FG_SECONDARY(),
            };
            rows.push(FlatRow {
                label,
                color,
                intent: n.intent.clone(),
                depth: n.depth,
                has_children: !n.children.is_empty(),
            });
            if n.expanded {
                Self::flatten_nodes(&n.children, rows, active_session_id);
            }
        }
    }

    // ── logo（深川明度阶，14 字符 art，右留呼吸列）──

    /// AGENDAO logo：4 行 ASCII art（14 字符宽，trailing flex 留呼吸列）。
    /// ░FG_TRACE / ▒FG_MUTED / █●○E_TEAL 明度阶（深川·流白景深），AGENDAO 翠青粗体。
    fn logo() -> (revue::widget::Stack, u16) {
        use colors::*;
        let r1 = Self::art_row(&[
            (FG_TRACE(), "░░░", false), (FG_MUTED(), "▒▒▒▒", false),
            (E_TEAL(), "████", false), (FG_TRACE(), "░░░", false),
        ]);
        let r2 = Self::art_row(&[
            (FG_MUTED(), "▒▒▒▒▒▒▒", false), (E_TEAL(), "███████", false),
            (FG_MUTED(), "     ", false), (E_TEAL(), "AGENDAO", true),
        ]);
        let r3 = Self::art_row(&[
            (FG_TRACE(), "░░░", false), (FG_MUTED(), "▒", false),
            (E_TEAL(), "●", false), (FG_MUTED(), "▒▒", false),
            (E_TEAL(), "██", false), (E_TEAL(), "○", false),
            (E_TEAL(), "█", false), (FG_TRACE(), "░░░", false),
            (FG_MUTED(), "The Dao of Agent", false),
        ]);
        let r4 = Self::art_row(&[
            (FG_TRACE(), "░░░░", false), (FG_MUTED(), "▒▒▒", false),
            (E_TEAL(), "███", false), (FG_TRACE(), "░░░░", false),
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

    /// ⚙ 真实落点 vs keymap 命中口径：渲染 32×24 全尺寸 sidebar，
    /// 找到 ⚙ glyph 实际渲染行/列，对照命中公式 (y == h-1, x >= W-3)。
    #[test]
    fn gear_render_position_matches_hit_formula() {
        let (sidebar, _tab_y, _hits) = SessionSidebar::build(
            &TokenUsage::default(),
            &CacheStats::default(),
            &Pricing::default(),
            0,
            &SidebarTrees::default(),
            &McpLspInfo::default(),
            &[],
            0,
            None,
            24,
        );
        let (w, h) = (crate::app::SIDEBAR_WIDTH as usize, 24u16);
        let mut buf = Buffer::new(w as u16, h);
        let area = Rect::new(0, 0, w as u16, h);
        let mut ctx = RenderContext::new(&mut buf, area);
        sidebar.render(&mut ctx);
        let mut gear_pos = None;
        for y in 0..h {
            for x in 0..w as u16 {
                if let Some(cell) = buf.get(x, y) {
                    if cell.symbol == '⚙' {
                        gear_pos = Some((x, y));
                    }
                }
            }
        }
        let (gx, gy) = gear_pos.expect("⚙ glyph should be rendered");
        assert_eq!(gy, h - 1, "⚙ row {} != last row {}", gy, h - 1);
        assert!(
            gx + SIDEBAR_GEAR_X_FROM_END >= crate::app::SIDEBAR_WIDTH,
            "⚙ col {} outside hit zone (x+{} >= {})",
            gx, SIDEBAR_GEAR_X_FROM_END, crate::app::SIDEBAR_WIDTH
        );
    }

    /// 真实页面构图复现：sidebar 作为 hstack 左 child（+VLine+右列 flex），
    /// 验证 ⚙ 是否仍在末行——若 hstack 不向 child 传满高，user_bar 会上浮，
    /// 而 keymap 命中公式写的是 y == terminal_h-1（「没反应」的候选根因）。
    #[test]
    fn gear_position_inside_page_hstack() {
        let (sidebar, _tab_y, _hits) = SessionSidebar::build(
            &TokenUsage::default(),
            &CacheStats::default(),
            &Pricing::default(),
            0,
            &SidebarTrees::default(),
            &McpLspInfo::default(),
            &[],
            0,
            None,
            24,
        );
        let h: u16 = 24;
        let page = hstack().gap(0)
            .child_sized(sidebar, crate::app::SIDEBAR_WIDTH)
            .child_sized(crate::widget::VLine::new(colors::SIDEBAR_DIVIDER()), 1)
            .child_flex(vstack().child_sized(Text::new("main"), 1), 1.0);
        let mut buf = Buffer::new(80, h);
        let area = Rect::new(0, 0, 80, h);
        let mut ctx = RenderContext::new(&mut buf, area);
        page.render(&mut ctx);
        let mut gear_pos = None;
        for y in 0..h {
            for x in 0..crate::app::SIDEBAR_WIDTH {
                if let Some(cell) = buf.get(x, y) {
                    if cell.symbol == '⚙' {
                        gear_pos = Some((x, y));
                    }
                }
            }
        }
        let (gx, gy) = gear_pos.expect("⚙ glyph should render inside page hstack");
        assert_eq!(gy, h - 1, "⚙ row {} != page last row {} — 命中公式将落空", gy, h - 1);
        let _ = gx;
    }

    /// Session tree 点击命中:NavigateSession 节点应产出 (绝对 y, session_id),
    /// 且 y 与渲染行一致(graph_start_y + 展平索引)。detail_h 随 active_tab 变,
    /// 命中 y 也随之偏移——验证 y 计算跟随 detail 高度(金律·成形同源)。
    #[test]
    fn session_tree_nav_hits_map_rows_to_session_ids() {
        let trees = SidebarTrees {
            session_nodes: vec![
                SidebarNode {
                    label: "root".into(),
                    depth: 0,
                    expanded: true,
                    children: vec![SidebarNode {
                        label: "child".into(),
                        depth: 1,
                        expanded: false,
                        children: vec![],
                        intent: Some(TreeIntent::NavigateSession("sess-child".into())),
                    }],
                    intent: Some(TreeIntent::NavigateSession("sess-root".into())),
                },
            ],
            workspace_nodes: vec![],
        };
        let token = TokenUsage::default();
        let cache = CacheStats::default();
        let price = Pricing::default();
        let mcp = McpLspInfo::default();
        // active_tab=0 → detail_h=4 → graph_start_y = 16 + 4 = 20。
        let (_stack, _tab_y, hits) =
            SessionSidebar::build(&token, &cache, &price, 0, &trees, &mcp, &[], 0, None, 40);
        assert_eq!(hits.len(), 2, "both nodes carry NavigateSession intent");
        assert_eq!(hits[0].session_id, "sess-root");
        assert_eq!(hits[0].y, 20, "root row at graph_start_y");
        assert_eq!(hits[1].session_id, "sess-child");
        assert_eq!(hits[1].y, 21, "child row one line below");
    }

    /// 回归：30 个会话 + 24 行视口 → 命中 y 不得越过 user_bar 行
    /// （幽灵命中曾盖满屏幕外区域，把最底行 ⚙ 的点击拦成 open_session）。
    #[test]
    fn nav_hits_clipped_to_viewport() {
        let nodes: Vec<SidebarNode> = (0..30)
            .map(|i| SidebarNode {
                label: format!("s{i}"),
                depth: 0,
                expanded: false,
                children: vec![],
                intent: Some(TreeIntent::NavigateSession(format!("s{i}"))),
            })
            .collect();
        let trees = SidebarTrees {
            session_nodes: nodes,
            workspace_nodes: vec![],
        };
        let (_s, _t, hits) = SessionSidebar::build(
            &TokenUsage::default(),
            &CacheStats::default(),
            &Pricing::default(),
            0,
            &trees,
            &McpLspInfo::default(),
            &[],
            0,
            None,
            24,
        );
        // active_tab=0 → detail_h=4 → graph_start_y=20；user_bar 行=23。
        assert!(
            hits.iter().all(|h| h.y < 23),
            "ghost hits beyond viewport: {:?}",
            hits.iter().map(|h| h.y).collect::<Vec<_>>()
        );
        assert_eq!(hits.len(), 3, "visible rows = 24 - 20 - 1 = 3");
    }

    /// 无会话时无命中(空 Vec),点击不触发导航。
    #[test]
    fn session_tree_no_hits_when_empty() {
        let trees = SidebarTrees::default();
        let token = TokenUsage::default();
        let cache = CacheStats::default();
        let price = Pricing::default();
        let mcp = McpLspInfo::default();
        let (_stack, _tab_y, hits) =
            SessionSidebar::build(&token, &cache, &price, 0, &trees, &mcp, &[], 0, None, 40);
        assert!(hits.is_empty());
    }

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
