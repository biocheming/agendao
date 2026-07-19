//! design-system 主题权威：4 套主题的注册、循环与切换收口（土律归一）。
//!
//! 颜色真值的运行时载体是 `crate::theme::Palette`（经
//! `crate::theme::set_current_palette` 切换、`colors::*()` 读取）；
//! 此处的 `ThemeId` 是主题身份与循环序的唯一权威，revue 侧的
//! `ThemeManager` 注册仅作 revue 生态（ThemePicker 等）的色值同步面。
//!
//! 历史教训：`Theme::custom(name)` 从 `Theme::dark()` 克隆色值，
//! `.variant(Light)` 只改 flag 不改色——light 曾是携带 dark 色的伪权威。
//! 现在每套主题都向 revue 注册真实 `Palette`/`ThemeColors`。

use revue::style::{Palette as RevuePalette, Theme, ThemeColors, ThemeVariant};

use crate::theme::Palette;

/// 主题身份（循环序即 `ALL` 顺序）。
///
/// 持久化用 `id()` 字符串（config `theme` 键），展示用 `label()`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    /// Tokyo Night Dark — 默认。
    TokyoNight,
    /// Tokyo Night Light。
    TokyoNightLight,
    /// 天青·汝窑 — 宋代美学亮主题（宣纸/墨色/天青/朱砂）。
    Tianqing,
    /// 千里江山 — 宋代美学暗主题（绢底墨青/月白/石绿/泥金）。
    Qianli,
}

impl ThemeId {
    /// 循环序：dark → light → 天青 → 千里江山。
    pub const ALL: [Self; 4] = [
        Self::TokyoNight,
        Self::TokyoNightLight,
        Self::Tianqing,
        Self::Qianli,
    ];

    /// 持久化 id（config `theme` 键的值）。
    pub fn id(self) -> &'static str {
        match self {
            Self::TokyoNight => "tokyo-night",
            Self::TokyoNightLight => "tokyo-night-light",
            Self::Tianqing => "tianqing",
            Self::Qianli => "qianli",
        }
    }

    /// 显示名（Settings pill / toast 文案，单一口径）。
    pub fn label(self) -> &'static str {
        match self {
            Self::TokyoNight => "Tokyo Night",
            Self::TokyoNightLight => "Tokyo Night Light",
            Self::Tianqing => "天青·汝窑",
            Self::Qianli => "千里江山",
        }
    }

    /// revue variant 映射（仅作 revue 侧明暗记账；颜色不读它）。
    pub fn variant(self) -> ThemeVariant {
        match self {
            Self::TokyoNight | Self::Qianli => ThemeVariant::Dark,
            Self::TokyoNightLight | Self::Tianqing => ThemeVariant::Light,
        }
    }

    /// 本主题的完整色板。
    pub fn palette(self) -> Palette {
        match self {
            Self::TokyoNight => Palette::tokyo_night(),
            Self::TokyoNightLight => Palette::tokyo_night_light(),
            Self::Tianqing => Palette::tianqing(),
            Self::Qianli => Palette::qianli(),
        }
    }

    /// 循环下一个（Enter/Space/→/Ctrl+P ToggleAppearance 共用）。
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// 循环上一个（Settings Theme 行 ←）。
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// 持久化 id → ThemeId（启动恢复用；未知值回退 None 由调用方兜底）。
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.id() == s)
    }

    /// revue `ThemeManager` 注册键。
    fn revue_key(self) -> String {
        format!("agendao-{}", self.id())
    }

    /// 向 revue 注册的 Theme（携带真实色值，修历史伪权威）。
    fn revue_theme(self) -> Theme {
        let p = self.palette();
        Theme::custom(self.label())
            .variant(self.variant())
            .palette(RevuePalette {
                primary: p.e_teal,
                secondary: p.accent_purple,
                success: p.accent_green,
                warning: p.accent_yellow,
                error: p.accent_red,
                info: p.accent_blue,
            })
            .colors(ThemeColors {
                background: p.bg_primary,
                surface: p.bg_surface,
                text: p.fg_primary,
                text_muted: p.fg_muted,
                border: p.border,
                divider: p.sidebar_divider,
                selection: p.surface_selected,
                selection_text: p.fg_primary,
                focus: p.e_teal,
            })
            .build()
    }
}

/// 注册全部 4 套主题到 revue 全局 `ThemeManager`。
///
/// 幂等：重复调用安全（HashMap 覆盖）。这是 agendao 主题的唯一注册点
/// （阴面收口）；切换经 `apply_theme` → `set_theme_by_id`。
pub fn register_agendao_themes() {
    for t in ThemeId::ALL {
        revue::style::register_theme(t.revue_key(), t.revue_theme());
    }
}

/// 换主题唯一权威（土律归一）：色板 + revue 主题信号同步。
///
/// 返回需 merge 进 stylesheet 的 `:root` 变量（CSS 面同步），由能拿到
/// `&mut App` 的层（app 事件闭包）应用到 `dom_renderer().stylesheet_mut()`。
/// store.theme_id 的记账由调用方（slash_action 单点）负责。
pub fn apply_theme(id: ThemeId) -> Vec<(String, String)> {
    crate::theme::set_current_palette(id.palette());
    revue::style::set_theme_by_id(&id.revue_key());
    root_css_vars(&id.palette())
}

/// Palette → styles/base.css `:root` 变量表。CSS 变量不再手写副本，
/// 由此单点程序化同步（启动注入 + 运行时切换同一路径）。
pub fn root_css_vars(p: &Palette) -> Vec<(String, String)> {
    fn hex(c: revue::prelude::Color) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    }
    vec![
        ("--bg-primary".into(), hex(p.bg_primary)),
        ("--bg-secondary".into(), hex(p.bg_secondary)),
        ("--bg-surface".into(), hex(p.bg_surface)),
        ("--bg-highlight".into(), hex(p.surface_selected)),
        ("--fg-primary".into(), hex(p.fg_primary)),
        ("--fg-secondary".into(), hex(p.fg_secondary)),
        ("--fg-muted".into(), hex(p.fg_muted)),
        ("--accent-cyan".into(), hex(p.accent_cyan)),
        ("--accent-green".into(), hex(p.accent_green)),
        ("--accent-yellow".into(), hex(p.accent_yellow)),
        ("--accent-red".into(), hex(p.accent_red)),
        ("--accent-purple".into(), hex(p.accent_purple)),
        ("--accent-blue".into(), hex(p.accent_blue)),
        ("--accent-orange".into(), hex(p.accent_orange)),
        ("--border-color".into(), hex(p.border)),
        ("--border-focus".into(), hex(p.e_teal)),
        ("--ds-wood".into(), hex(p.e_teal)),
        ("--ds-fire".into(), hex(p.e_amber)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_order_is_stable() {
        let mut t = ThemeId::TokyoNight;
        let mut seen = vec![t];
        for _ in 0..3 {
            t = t.next();
            seen.push(t);
        }
        assert_eq!(
            seen,
            vec![
                ThemeId::TokyoNight,
                ThemeId::TokyoNightLight,
                ThemeId::Tianqing,
                ThemeId::Qianli,
            ]
        );
        assert_eq!(t.next(), ThemeId::TokyoNight, "循环应回绕");
    }

    #[test]
    fn prev_is_next_inverse() {
        for t in ThemeId::ALL {
            assert_eq!(t.next().prev(), t);
            assert_eq!(t.prev().next(), t);
        }
    }

    #[test]
    fn from_id_roundtrip() {
        for t in ThemeId::ALL {
            assert_eq!(ThemeId::from_id(t.id()), Some(t));
        }
        assert_eq!(ThemeId::from_id("nope"), None);
    }

    #[test]
    fn register_is_idempotent_and_complete() {
        register_agendao_themes();
        register_agendao_themes();
        let ids = revue::style::theme_ids();
        for t in ThemeId::ALL {
            assert!(
                ids.iter().any(|i| i == &t.revue_key()),
                "{} missing",
                t.id()
            );
        }
    }

    #[test]
    fn revue_themes_carry_real_palette_values() {
        // 历史伪权威回归测试：light 主题不得携带 dark 色值。
        let light = ThemeId::TokyoNightLight.revue_theme();
        assert_eq!(
            light.colors.background,
            Palette::tokyo_night_light().bg_primary
        );
        assert_ne!(light.colors.background, Palette::tokyo_night().bg_primary);
    }

    #[test]
    fn apply_theme_switches_palette_and_returns_css_vars() {
        let vars = apply_theme(ThemeId::Qianli);
        assert_eq!(crate::theme::current_palette(), Palette::qianli());
        assert!(vars.iter().any(|(k, _)| k == "--ds-wood"));
        apply_theme(ThemeId::TokyoNight);
        assert_eq!(crate::theme::current_palette(), Palette::tokyo_night());
    }
}
