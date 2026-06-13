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
        let mut sidebar = vstack().gap(1)
            .child(Self::panel("📊 Token Usage", Self::token_panel(token)))
            .child(Self::panel("💾 Cache", Self::cache_panel(cache)))
            .child(Self::panel("💰 Pricing", Self::pricing_panel(price)))
            .child(Self::panel("📐 Context", Self::meter_panel(ctx_pct)))
            .child(Self::panel("🔧 Tools", Self::tools_panel(tools)))
            .child(Self::panel("🌐 MCP/LSP", Self::mcp_panel(mcp)));

        if !trees.session_nodes.is_empty() {
            sidebar = sidebar.child(Self::panel("🌳 Sessions", Self::tree_panel(&trees.session_nodes)));
        }
        sidebar
    }

    /// Wrap content in a Border panel.
    fn panel(title: &str, content: revue::widget::Stack) -> revue::widget::Border {
        Border::rounded()
            .title(title.to_string())
            .child(content)
            .class("SidebarPanel")
    }

    // ── Individual panels ──

    fn token_panel(t: &TokenUsage) -> revue::widget::Stack {
        let mut s = vstack()
            .child(Text::new(format!("Input:  {:>8}", fmt_count(t.input))).class("SidebarText"))
            .child(Text::new(format!("Output: {:>8}", fmt_count(t.output))).class("SidebarText"))
            .child(Text::new(format!("Total:  {:>8}", fmt_count(t.total))).class("SidebarText"));

        // Per-turn breakdown (from cache tokens)
        if t.cache_read > 0 || t.cache_miss > 0 {
            let turn_total = t.cache_read + t.cache_miss;
            s = s
                .child(Text::new("─".repeat(20)).fg(Color::rgb(59, 66, 97)))
                .child(Text::new(format!("Turn read: {:>5}", fmt_count(t.cache_read))).class("SidebarText"))
                .child(Text::new(format!("Turn miss: {:>5}", fmt_count(t.cache_miss))).class("SidebarText"))
                .child(Text::new(format!("Turn write:{:>5}", fmt_count(t.cache_write))).class("SidebarText"))
                .child(Text::new(format!("Turn total:{:>5}", fmt_count(turn_total))).class("SidebarText"));
        }
        s
    }

    fn cache_panel(c: &CacheStats) -> revue::widget::Stack {
        vstack()
            .child(Text::new(format!("Hits:   {:>8}", c.hits)).class("SidebarText"))
            .child(Text::new(format!("Misses: {:>8}", c.misses)).class("SidebarText"))
            .child(Text::new(format!("Writes: {:>8}", c.writes)).class("SidebarText"))
    }

    fn pricing_panel(p: &Pricing) -> revue::widget::Stack {
        vstack()
            .child(Text::new(format!("In:  ${:.6}/1k", p.input_per_1k)).class("SidebarText"))
            .child(Text::new(format!("Out: ${:.6}/1k", p.output_per_1k)).class("SidebarText"))
    }

    fn meter_panel(pct: u8) -> revue::widget::Stack {
        let bar = Self::meter_bar(pct);
        vstack()
            .child(Text::new(format!("{}% used", pct)).class("SidebarText"))
            .child(bar)
    }

    /// Build a text-based progress bar.
    fn meter_bar(pct: u8) -> revue::widget::Text {
        let filled = (pct as usize * 20 / 100).min(20);
        let bar: String = std::iter::repeat('█').take(filled)
            .chain(std::iter::repeat('░').take(20 - filled))
            .collect();
        let color = if pct > 80 { Color::rgb(247, 118, 142) }
                   else if pct > 50 { Color::rgb(224, 175, 104) }
                   else { Color::rgb(158, 206, 106) };
        Text::new(bar).fg(color)
    }

    fn tools_panel(tools: &[ActiveTool]) -> revue::widget::Stack {
        let mut s = vstack();
        if tools.is_empty() {
            s = s.child(Text::new("(none)").class("SidebarText"));
        } else {
            for t in tools {
                let icon = match t.phase { ToolPhase::Starting => "○", ToolPhase::Running => "◉", ToolPhase::Done => "●" };
                s = s.child(Text::new(format!("{} {}", icon, t.name)).class("SidebarText"));
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
            .child(Text::new(mcp_status).class("SidebarText"))
            .child(Text::new(lsp_status).class("SidebarText"))
    }

    /// Render tree nodes flat (max 30 items, indent via "  ".repeat(depth)).
    fn tree_panel(nodes: &[SidebarNode]) -> revue::widget::Stack {
        // Flatten the tree into lines first, then build stack once
        let mut lines: Vec<(String, Color)> = Vec::new();
        Self::flatten_nodes(nodes, &mut lines);
        let mut s = vstack();
        for (label, color) in lines.iter().take(30) {
            s = s.child(Text::new(label.as_str()).fg(*color));
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
                Some(TreeIntent::NavigateSession(_)) => Color::rgb(125, 207, 255),
                Some(TreeIntent::OpenFile(_)) => Color::rgb(158, 206, 106),
                None => Color::rgb(169, 177, 214),
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
