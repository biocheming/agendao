//! resolve_color 单点：语义色 → 具体颜色。
//!
//! 吸取 claudecode 教训（ThemedBox/ThemedText/color.ts 三处重复 resolveColor），
//! agendao 第一天就收成单一函数。颜色真值来自 styles/base.css 的 :root 变量；
//! 此处返回 Rust Color 供那些无法走 CSS class 的场景（如动态生成的 Text）使用。
//! **优先用 DsText 挂 class，resolve_color 仅作 fallback。**

use revue::prelude::Color;
use crate::theme::colors;

/// 五行 + 状态语义色。所有"这个块代表什么角色"的颜色都经此枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semantic {
    // 五行角色
    Wood,    // 木：用户输入
    Fire,    // 火：执行（工具调用）
    Earth,   // 土：编排（系统通知）
    Metal,   // 金：输出（助手/成形）
    Water,   // 水：回流（遥测/think）
    // 状态
    Ok,
    Warn,
    Error,
    Info,
    Muted,
    Accent,  // 签名青
}

/// 语义 → 颜色单点映射。**这是 agendao 唯一的语义色收口。**
///
/// 新增语义色只改这一处。colors::* 的常量仍是底层色值定义，但
/// "角色→色"的语义判断只允许经过这里。
pub fn resolve_color(s: Semantic) -> Color {
    match s {
        Semantic::Wood   => colors::E_TEAL,        // 用户
        Semantic::Fire   => colors::E_AMBER,       // 工具
        Semantic::Earth  => colors::FG_SECONDARY,  // 系统
        Semantic::Metal  => colors::FG_PRIMARY,    // 助手
        Semantic::Water  => colors::ACCENT_PURPLE, // think/遥测
        Semantic::Ok     => colors::STATUS_OK,
        Semantic::Warn   => colors::STATUS_WARN,
        Semantic::Error  => colors::STATUS_ERROR,
        Semantic::Info   => colors::STATUS_INFO,
        Semantic::Muted  => colors::FG_MUTED,
        Semantic::Accent => colors::ACCENT_CYAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wood_resolves_to_teal() {
        assert_eq!(resolve_color(Semantic::Wood), colors::E_TEAL);
    }

    #[test]
    fn status_semantics_distinct() {
        assert_ne!(resolve_color(Semantic::Ok), resolve_color(Semantic::Error));
        assert_ne!(resolve_color(Semantic::Warn), resolve_color(Semantic::Info));
    }

    #[test]
    fn all_variants_resolve() {
        // 确保每个语义都映射到一个具体颜色（不 panic）
        for s in [Semantic::Wood, Semantic::Fire, Semantic::Earth, Semantic::Metal,
                  Semantic::Water, Semantic::Ok, Semantic::Warn, Semantic::Error,
                  Semantic::Info, Semantic::Muted, Semantic::Accent] {
            let _ = resolve_color(s);
        }
    }
}
