//! 水 — SessionSidebar: telemetry panels via Revue widgets.
//!
//! Panels: TokenUsage, CacheStats, Pricing, ContextMeter,
//! SessionTree, WorkspaceTree, McpLsp.
//! All styled via CSS classes. Toggle with Ctrl+B.

use revue::prelude::*;
use crate::store::types::{
    ActiveTool, CacheStats, McpLspInfo, Pricing, SidebarTrees, ToolPhase, TokenUsage,
    TreeNode as SidebarNode, TreeIntent,
};
use crate::theme::colors;

/// Sidebar component — renders when visible.
pub struct SessionSidebar {
    pub visible: bool,
    pub width: u16,
}

impl SessionSidebar {
    pub fn new() -> Self { Self { visible: false, width: 30 } }
    pub fn toggle(&mut self) { self.visible = !self.visible; }

    /// Build the full sidebar widget tree.
    pub fn build(
        token: &TokenUsage,
        cache: &CacheStats,
        price: &Pricing,
        ctx_pct: u8,
        trees: &SidebarTrees,
        mcp: &McpLspInfo,
        tools: &[ActiveTool],
    ) -> revue::widget::Stack {
        // Each panel's height = border (2) + content rows. Compute once
        // and use child_sized so vstack stops squeezing dead air between
        // panels by stretching every Border to fill its share of height.
        let token_h = 2 + (3 + if token.cache_read > 0 || token.cache_miss > 0 { 5 } else { 0 });
        let cache_h = 2 + 3;
        let price_h = 2 + 2;
        let meter_h = 2 + 2;
        let tools_rows = if tools.is_empty() { 1 } else { 1 + tools.len().min(8) as u16 };
        let tools_h = 2 + tools_rows;
        let mcp_h = 2 + 2;

        let mut sidebar = vstack().gap(1)
            .child_sized(Self::panel("📊 Token Usage", Self::token_panel(token)), token_h)
            .child_sized(Self::panel("💾 Cache", Self::cache_panel(cache)), cache_h)
            .child_sized(Self::panel("💰 Pricing", Self::pricing_panel(price)), price_h)
            .child_sized(Self::panel("📐 Context", Self::meter_panel(ctx_pct)), meter_h)
            .child_sized(Self::panel("🔧 Tools", Self::tools_panel(tools)), tools_h)
            .child_sized(Self::panel("🌐 MCP/LSP", Self::mcp_panel(mcp)), mcp_h);

        if !trees.session_nodes.is_empty() {
            let tree_rows = trees.session_nodes.iter().map(|n| 1 + n.children.len() as u16).sum::<u16>().min(30);
            sidebar = sidebar.child_sized(
                Self::panel("🌳 Sessions", Self::tree_panel(&trees.session_nodes)),
                2 + tree_rows,
            );
        }
        sidebar
    }

    /// Wrap content in a Border panel. Title gets a leading and trailing
    /// space so it reads as " 📊 Token Usage " — symmetric padding —
    /// instead of "📊Token Usage   " from the bare emoji-prefix style.
    fn panel(title: &str, content: revue::widget::Stack) -> revue::widget::Border {
        Border::rounded()
            .title(format!(" {} ", title))
            .child(content)
            .class("SidebarPanel")
    }

    // ── Individual panels ──

    fn token_panel(t: &TokenUsage) -> revue::widget::Stack {
        let mut s = vstack()
            .child_sized(Text::new(format!("Input:  {:>8}", fmt_count(t.input))).class("SidebarText"), 1)
            .child_sized(Text::new(format!("Output: {:>8}", fmt_count(t.output))).class("SidebarText"), 1)
            .child_sized(Text::new(format!("Total:  {:>8}", fmt_count(t.total))).class("SidebarText"), 1);

        // Per-turn breakdown (from cache tokens)
        if t.cache_read > 0 || t.cache_miss > 0 {
            let turn_total = t.cache_read + t.cache_miss;
            s = s
                .child_sized(Text::new("─".repeat(20)).fg(colors::BORDER), 1)
                .child_sized(Text::new(format!("Turn read: {:>5}", fmt_count(t.cache_read))).class("SidebarText"), 1)
                .child_sized(Text::new(format!("Turn miss: {:>5}", fmt_count(t.cache_miss))).class("SidebarText"), 1)
                .child_sized(Text::new(format!("Turn write:{:>5}", fmt_count(t.cache_write))).class("SidebarText"), 1)
                .child_sized(Text::new(format!("Turn total:{:>5}", fmt_count(turn_total))).class("SidebarText"), 1);
        }
        s
    }

    fn cache_panel(c: &CacheStats) -> revue::widget::Stack {
        vstack()
            .child_sized(Text::new(format!("Hits:   {:>8}", c.hits)).class("SidebarText"), 1)
            .child_sized(Text::new(format!("Misses: {:>8}", c.misses)).class("SidebarText"), 1)
            .child_sized(Text::new(format!("Writes: {:>8}", c.writes)).class("SidebarText"), 1)
    }

    fn pricing_panel(p: &Pricing) -> revue::widget::Stack {
        vstack()
            .child_sized(Text::new(format!("In:  ${:.6}/1k", p.input_per_1k)).class("SidebarText"), 1)
            .child_sized(Text::new(format!("Out: ${:.6}/1k", p.output_per_1k)).class("SidebarText"), 1)
    }

    fn meter_panel(pct: u8) -> revue::widget::Stack {
        let bar = Self::meter_bar(pct);
        vstack()
            .child_sized(Text::new(format!("{}% used", pct)).class("SidebarText"), 1)
            .child_sized(bar, 1)
    }

    /// Build a progress bar using revue's Progress widget.
    fn meter_bar(pct: u8) -> revue::widget::Progress {
        let color = if pct > 80 { colors::ACCENT_RED }
                   else if pct > 50 { colors::ACCENT_YELLOW }
                   else { colors::ACCENT_GREEN };
        revue::widget::progress(pct as f32 / 100.0)
            .filled_color(color)
            .show_percentage(true)
    }

    fn tools_panel(tools: &[ActiveTool]) -> revue::widget::Stack {
        let mut s = vstack();
        let running = tools.iter().filter(|t| t.phase == ToolPhase::Running).count();
        let done = tools.iter().filter(|t| t.phase == ToolPhase::Done).count();
        let starting = tools.iter().filter(|t| t.phase == ToolPhase::Starting).count();
        if tools.is_empty() {
            s = s.child_sized(Text::new("(idle)").class("SidebarText"), 1);
        } else {
            s = s.child_sized(Text::new(format!("▶ {}  ◉ {}  ● {}", starting, running, done))
                .fg(colors::ACCENT_BLUE), 1);
            for t in tools.iter().take(8) {
                let icon = match t.phase {
                    ToolPhase::Starting => "○",
                    ToolPhase::Running => "◉",
                    ToolPhase::Done => "●",
                };
                s = s.child_sized(Text::new(format!("  {} {}", icon, t.name)).class("SidebarText"), 1);
            }
        }
        s
    }

    fn mcp_panel(m: &McpLspInfo) -> revue::widget::Stack {
        let mcp_status = if m.mcp_total == 0 {
            "MCP: (none)".to_string()
        } else {
            format!("MCP: {}/{} connected", m.mcp_connected, m.mcp_total)
        };
        let lsp_status = if m.lsp_active.is_empty() {
            "LSP: (none)".to_string()
        } else {
            format!("LSP: {}", m.lsp_active.join(", "))
        };
        vstack()
            .child_sized(Text::new(mcp_status).class("SidebarText"), 1)
            .child_sized(Text::new(lsp_status).class("SidebarText"), 1)
    }

    /// Render tree nodes flat (max 30 items, indent via "  ".repeat(depth)).
    fn tree_panel(nodes: &[SidebarNode]) -> revue::widget::Stack {
        // Flatten the tree into lines first, then build stack once.
        // Each row is `child_sized(_, 1)` so panel rows don't get
        // stretched apart by Auto distribution inside the Border.
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

    /// Render the sidebar at the given area.
    pub fn render_sidebar(&self, ctx: &mut RenderContext, content: revue::widget::Stack) {
        if !self.visible { return; }
        content.render(ctx);
    }
}

fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000 { format!("{:.1}K", n as f64 / 1_000.0) }
    else { n.to_string() }
}
