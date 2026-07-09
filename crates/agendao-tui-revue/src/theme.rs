//! 土 — Shared visual theme: Tokyo Night color palette.
//!
//! Central authority for all colors used across the TUI.
//! Matches the CSS variables in `styles/base.css`.

use revue::prelude::Color;

/// Tokyo Night color palette — single source of truth for inline colors.
/// CSS classes in base.css reference the same values via `:root` variables.
pub mod colors {
    use super::Color;

    // ── Backgrounds ──
    pub const BG_PRIMARY: Color = Color::rgb(26, 27, 38); // #1a1b26
    // 深井（Trench）：比 BG_PRIMARY 更暗的下沉容器背景（深川·流白「井底」#050507
    // 之于深渊 #0a0a0d）。终端默认背景 = BG_PRIMARY（非纯黑，见 dialog/backdrop 用它
    // 「融入终端」），故 BG_DEEP 视觉「下沉」成立——用于 ToolResult 整体缩进深井
    // （Gemini 第二轮指令#2）。明度差 ≈ 9 阶 > JND 阈值 8，终端可辨。
    pub const BG_DEEP: Color = Color::rgb(16, 17, 26); // #10111a
    pub const BG_SECONDARY: Color = Color::rgb(36, 40, 59); // #24283b
    pub const BG_SURFACE: Color = Color::rgb(47, 51, 70); // #2f3346

    // ── Mockup E "glass tactile" semi-transparent surfaces ──
    //
    // The HTML mockup builds depth via rgba() overlays on the dark
    // background. Terminals can't render alpha, so we pre-composite
    // each tint against BG_PRIMARY (#1a1b26) and store the resulting
    // opaque color. This keeps the tactile-card feel without needing
    // alpha support.
    //
    // Composite formula: out = bg * (1 - α) + tint * α
    // SURFACE_RAISED 是无色相提亮层（rgba(255,255,255,0.025) on BG_PRIMARY），
    // 当前无消费者——保留为 opencode 式明度阶梯 / 容器背景的储备。P0 去色相后
    // 块背景不再用色相涂整片（已移除带色相的 SURFACE_USER 青 / SURFACE_TOOL 橙 /
    // SURFACE_THINK 琥珀），改靠符号标记 + 焦点条区分角色，避免多色相块背景过载。
    pub const SURFACE_RAISED: Color = Color::rgb(32, 33, 44);
    pub const SURFACE_SELECTED: Color = Color::rgb(45, 70, 65); // rgba(60,184,162,0.18) — selected dialog row (青背景)

    pub const BG_HIGHLIGHT: Color = SURFACE_SELECTED; // alias — selected row 高亮（SURFACE_SELECTED 青调玻璃面）。

    // ── Foregrounds ──
    pub const FG_PRIMARY: Color = Color::rgb(192, 202, 245); // #c0caf5
    pub const FG_SECONDARY: Color = Color::rgb(169, 177, 214); // #a9b1d6
    pub const FG_MUTED: Color = Color::rgb(86, 95, 137); // #565f89
    pub const FG_TRACE: Color = Color::rgb(59, 66, 82); // #3b4252 — 背景流 Trace（井内列表项/计数），压到「刚好看清」的极限暗度，让正文独占视觉舞台（深川·流白景深）。

    // ── 纯黑合一边界 ──
    // Nord polar night 2：比 FG_TRACE（#3b4252）再暗一阶，作 sidebar↔主区
    // 之间那根「极暗淡」垂直 `│` 分隔线的唯一前景色（纯黑合一指令：sidebar
    // 不再有 BG_DEEP 底色，与主窗口共享终端纯黑背景，仅靠此线划界）。
    pub const SIDEBAR_DIVIDER: Color = Color::rgb(46, 52, 64); // #2e3440

    // ── Accents ──
    pub const ACCENT_CYAN: Color = Color::rgb(125, 207, 255); // #7dcfff
    pub const ACCENT_GREEN: Color = Color::rgb(158, 206, 106); // #9ece6a
    pub const ACCENT_YELLOW: Color = Color::rgb(224, 175, 104); // #e0af68
    pub const ACCENT_RED: Color = Color::rgb(247, 118, 142); // #f7768e
    pub const ACCENT_PURPLE: Color = Color::rgb(187, 154, 247); // #bb9af7
    pub const ACCENT_BLUE: Color = Color::rgb(122, 162, 247); // #7aa2f7
    pub const ACCENT_ORANGE: Color = Color::rgb(255, 184, 108); // #ffb86c (tips)
    pub const NORD_ORANGE: Color = Color::rgb(208, 135, 112); // #d08770 — 工具调用刚性算子 ⚒（Nord orange，Gemini 终极符号令）。

    // ── Mockup E signature accents ──
    // These two replace ACCENT_CYAN/ACCENT_YELLOW for E-style components
    // (badges, tool chips, group headers, selected rows). They're a
    // deeper, more muted hue that reads as "design accent" rather than
    // the brighter Tokyo Night Cyan/Yellow which work better as inline
    // text accents (links, code).
    pub const E_TEAL: Color = Color::rgb(60, 184, 162); // #3cb8a2 — user bubbles, selected, success badges
    pub const E_AMBER: Color = Color::rgb(240, 168, 82); // #f0a852 — tool chips, group headers, send button

    // ── Borders ──
    pub const BORDER: Color = Color::rgb(59, 66, 97); // #3b4261

    // ── Semantic ──
    pub const STATUS_OK: Color = ACCENT_GREEN;
    pub const STATUS_WARN: Color = ACCENT_YELLOW;
    pub const STATUS_ERROR: Color = ACCENT_RED;
    pub const STATUS_INFO: Color = ACCENT_CYAN;
}

/// Helper: format token counts with K suffix (e.g. "1.2k", "456").
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// Helper: format cost with appropriate precision.
pub fn fmt_cost(cost: f64) -> String {
    if cost < 0.001 {
        format!("${:.4}", cost)
    } else if cost < 1.0 {
        format!("${:.3}", cost)
    } else {
        format!("${:.2}", cost)
    }
}
