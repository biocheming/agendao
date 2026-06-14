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
    pub const SURFACE_RAISED: Color = Color::rgb(32, 33, 44); // rgba(255,255,255,0.025) on BG_PRIMARY — assistant bubble
    pub const SURFACE_USER: Color = Color::rgb(28, 36, 47); // rgba(60,184,162,0.06) — user bubble (青底淡)
    pub const SURFACE_TOOL: Color = Color::rgb(44, 39, 41); // rgba(240,168,82,0.08) — tool chip (橙底淡)
    pub const SURFACE_THINK: Color = Color::rgb(53, 42, 38); // rgba(240,168,82,0.12) — reasoning slab (warm amber wash)
    pub const SURFACE_SELECTED: Color = Color::rgb(45, 70, 65); // rgba(60,184,162,0.18) — selected dialog row (青背景)

    pub const BG_HIGHLIGHT: Color = SURFACE_SELECTED; // alias — selected row uses the cyan-tinted glass surface from the mockup, not the previous saturated blue. Cyan signals "your current pick" consistently with user-bubble tint.

    // Subtle borders (also pre-composited from rgba)
    pub const BORDER_FAINT: Color = Color::rgb(35, 36, 44); // rgba(255,255,255,0.06)
    pub const BORDER_USER: Color = Color::rgb(38, 50, 56); // rgba(60,184,162,0.15) — user bubble border
    pub const BORDER_TOOL: Color = Color::rgb(58, 56, 53); // rgba(240,168,82,0.18) — tool chip border
    pub const BORDER_SEL: Color = Color::rgb(60, 184, 162); // solid cyan #3cb8a2 — left-bar accent on selected row

    // ── Foregrounds ──
    pub const FG_PRIMARY: Color = Color::rgb(192, 202, 245); // #c0caf5
    pub const FG_SECONDARY: Color = Color::rgb(169, 177, 214); // #a9b1d6
    pub const FG_MUTED: Color = Color::rgb(86, 95, 137); // #565f89

    // ── Accents ──
    pub const ACCENT_CYAN: Color = Color::rgb(125, 207, 255); // #7dcfff
    pub const ACCENT_GREEN: Color = Color::rgb(158, 206, 106); // #9ece6a
    pub const ACCENT_YELLOW: Color = Color::rgb(224, 175, 104); // #e0af68
    pub const ACCENT_RED: Color = Color::rgb(247, 118, 142); // #f7768e
    pub const ACCENT_PURPLE: Color = Color::rgb(187, 154, 247); // #bb9af7
    pub const ACCENT_BLUE: Color = Color::rgb(122, 162, 247); // #7aa2f7
    pub const ACCENT_ORANGE: Color = Color::rgb(255, 184, 108); // #ffb86c (tips)

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
    pub const BORDER_FOCUS: Color = Color::rgb(125, 207, 255); // = ACCENT_CYAN

    // ── Semantic ──
    pub const STATUS_OK: Color = ACCENT_GREEN;
    pub const STATUS_WARN: Color = ACCENT_YELLOW;
    pub const STATUS_ERROR: Color = ACCENT_RED;
    pub const STATUS_INFO: Color = ACCENT_CYAN;
}

/// Helper: format a duration in human-readable form (e.g. "2.3s", "45ms").
#[allow(dead_code)]
pub fn fmt_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
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
