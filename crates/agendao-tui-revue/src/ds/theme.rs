//! design-system 主题：Tokyo Night（dark/light）。
//!
//! 颜色真值的单一权威是 `styles/base.css` 的 `:root` 变量（土律归一）；
//! 此处的 `Theme` 仅驱动运行时 variant 切换（revue `Signal<Theme>`），
//! 不重复定义色值。OSC11 终端背景探测（`ds/osc11`，后续 Task）会覆盖
//! 此处的默认 dark 选择。

use revue::style::{Theme, ThemeVariant};

/// Tokyo Night Dark — agendao 默认主题。
pub fn tokyo_night_dark() -> Theme {
    Theme::custom("Tokyo Night Dark")
        .variant(ThemeVariant::Dark)
        .build()
}

/// Tokyo Night Light — OSC11 检测到亮色终端时使用。
pub fn tokyo_night_light() -> Theme {
    Theme::custom("Tokyo Night Light")
        .variant(ThemeVariant::Light)
        .build()
}

/// variant → theme 映射（agendao 仅注册 dark/light 两套主题；HighContrast
/// 无对应主题，fallback dark，避免 set_theme 指向未注册主题）。
pub fn theme_for(v: ThemeVariant) -> Theme {
    match v {
        ThemeVariant::Dark | ThemeVariant::HighContrast => tokyo_night_dark(),
        ThemeVariant::Light => tokyo_night_light(),
    }
}

/// 翻转 variant（agendao 在 dark/light 间切；HighContrast → Dark）。
/// 唯一翻转入口——ToggleAppearance 经此调，不在 keymap 自判（土律归一）。
pub fn toggle_variant(v: ThemeVariant) -> ThemeVariant {
    match v {
        ThemeVariant::Dark => ThemeVariant::Light,
        _ => ThemeVariant::Dark,
    }
}

/// variant → 显示标签（toast 文案用，单一口径）。
pub fn variant_label(v: ThemeVariant) -> &'static str {
    match v {
        ThemeVariant::Dark => "dark",
        ThemeVariant::Light => "light",
        ThemeVariant::HighContrast => "high-contrast",
    }
}

/// 注册 agendao 的 dark/light 主题到 revue 全局 `ThemeManager`。
///
/// 幂等：重复调用安全（HashMap 覆盖）。这是 agendao 主题的唯一注册点
/// （阴面收口）；切换经 `revue::style::set_theme_by_id("agendao-dark"|"agendao-light")`。
pub fn register_agendao_themes() {
    revue::style::register_theme("agendao-dark", tokyo_night_dark());
    revue::style::register_theme("agendao-light", tokyo_night_light());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_is_dark_variant() {
        let t = tokyo_night_dark();
        assert_eq!(t.name, "Tokyo Night Dark");
        assert!(t.is_dark());
        assert!(!t.is_light());
    }

    #[test]
    fn light_theme_is_light_variant() {
        let t = tokyo_night_light();
        assert_eq!(t.name, "Tokyo Night Light");
        assert!(t.is_light());
        assert!(!t.is_dark());
    }

    #[test]
    fn register_agendao_themes_is_idempotent() {
        // register_theme 内部用全局 ThemeManager HashMap，重复 register
        // 同 id 应覆盖不 panic。
        register_agendao_themes();
        register_agendao_themes();
        let ids = revue::style::theme_ids();
        assert!(ids.iter().any(|i| i == "agendao-dark"), "agendao-dark missing");
        assert!(ids.iter().any(|i| i == "agendao-light"), "agendao-light missing");
    }
}
