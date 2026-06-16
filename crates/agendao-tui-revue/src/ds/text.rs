//! DsText — design-system 文本封装。
//!
//! 优先走 CSS class（.ds-text--<semantic>），颜色真值在 base.css。
//! 同时挂 class + 设 fg(resolve_color) 作 fallback，保证 CSS 未命中时仍有色。

use revue::prelude::Text;
use super::color::{Semantic, resolve_color};

/// design-system 文本。替代裸 `Text::new().fg(color)`。
pub struct DsText {
    pub text: String,
    pub semantic: Option<Semantic>,
    pub bold: bool,
}

impl DsText {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), semantic: None, bold: false }
    }

    /// 挂语义色（同时设 class 走 CSS，并记 semantic 作 fallback）。
    pub fn semantic(mut self, s: Semantic) -> Self {
        self.semantic = Some(s);
        self
    }

    pub fn bold(mut self) -> Self { self.bold = true; self }

    /// 构造 revue Text：挂语义 class + fg(resolve_color) 双保险。
    pub fn build(self) -> Text {
        let mut t = Text::new(self.text);
        if let Some(s) = self.semantic {
            t = t.class(semantic_class(s)).fg(resolve_color(s));
        }
        if self.bold { t = t.bold(); }
        t
    }
}

/// 语义 → CSS class 名（base.css 里对应规则）。
pub fn semantic_class(s: Semantic) -> &'static str {
    match s {
        Semantic::Wood   => "ds-wood",
        Semantic::Fire   => "ds-fire",
        Semantic::Earth  => "ds-earth",
        Semantic::Metal  => "ds-metal",
        Semantic::Water  => "ds-water",
        Semantic::Ok     => "ds-ok",
        Semantic::Warn   => "ds-warn",
        Semantic::Error  => "ds-error",
        Semantic::Info   => "ds-info",
        Semantic::Muted  => "ds-muted",
        Semantic::Accent => "ds-accent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_class_mapping_complete() {
        assert_eq!(semantic_class(Semantic::Wood), "ds-wood");
        assert_eq!(semantic_class(Semantic::Error), "ds-error");
        assert_eq!(semantic_class(Semantic::Accent), "ds-accent");
    }

    #[test]
    fn dstext_build_with_semantic() {
        let _ = DsText::new("hi").semantic(Semantic::Wood).bold().build();
    }
}
