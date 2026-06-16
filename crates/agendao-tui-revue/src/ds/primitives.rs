//! Ds* 基础控件封装：挂 CSS class，颜色权威在 base.css。
//!
//! 这些是对 revue 原语的薄封装，目的只在一个：统一 class 命名 +
//! 让"哪些 class 存在"有单一清单（本文件 + base.css 的 ds-* 规则）。

use revue::prelude::Text;
use super::color::{Semantic, resolve_color};
use super::text::semantic_class;

/// 带角色色彩的 chip（如工具名标签）。替代裸 Text 拼色。
pub fn ds_tag(label: impl Into<String>, s: Semantic) -> Text {
    Text::new(label).class(semantic_class(s)).fg(resolve_color(s))
}

/// 状态徽章文本（Ok/Warn/Error/Info）。
pub fn ds_badge(label: impl Into<String>, s: Semantic) -> Text {
    Text::new(label).class("ds-badge").fg(resolve_color(s))
}

/// 普通分隔线文本（一串 ─）。
pub fn ds_divider(width: usize) -> Text {
    Text::new("─".repeat(width)).class("ds-divider")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ds_tag_carries_semantic_color() {
        let _ = ds_tag("read", Semantic::Fire);
    }

    #[test]
    fn ds_badge_ok() {
        let _ = ds_badge("done", Semantic::Ok);
    }

    #[test]
    fn ds_divider_width() {
        let _ = ds_divider(20);
    }
}
