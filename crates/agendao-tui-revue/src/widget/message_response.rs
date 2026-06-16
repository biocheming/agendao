//! MessageResponse — claudecode 的 `⎿` 缩进视觉语法。
//!
//! `"  ⎿  "`（5 字符）+ dimColor 前缀，用于 tool 进度/错误/结果元信息等
//! "主消息的子消息"。金律-输出成形的视觉收口。

use revue::prelude::Text;
use crate::theme::colors;

/// 给一行内容前置 `⎿` 缩进（dimColor）。返回带前缀的 Text。
pub fn indented_prefix() -> Text {
    Text::new("  ⎿  ").fg(colors::FG_MUTED)
}

/// 缩进字符常量（供 hstack 拼接用）。
pub const INDENT: &str = "  ⎿  ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indent_is_five_chars() {
        assert_eq!(INDENT.chars().count(), 5);
        assert!(INDENT.contains('⎿'));
    }
}
