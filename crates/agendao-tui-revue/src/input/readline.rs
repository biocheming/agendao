//! 木 — readline 键集扩展：revue（第三方库，不可改）公开 API 之上的
//! 组合层（土律·单点权威：所有 ctrl chord / 粘贴语义只在此定义一份）。
//!
//! revue `Input` 自带的 ctrl 处理是 select_all/copy/cut/paste/undo 的
//! GUI 集，且未绑定 chord 会剥掉 ctrl 退化成插入字面字母（Ctrl+W → 'w'）。
//! 本模块以扩展 trait 补齐 readline 集（A/E/W/U/K/Z/Y、词跳、kill 行），
//! 未绑定 chord 吞掉返回 true，绝不退化。
//!
//!  kill 类操作经公开 `handle_key(Backspace/Delete)` 逐字执行——字段普遍
//!  很短（名称/URL），且每步都进 revue Input 的 undo 历史（char 粒度，
//!  CJK 安全；undo 粒度为逐字符，为可接受的折衷）。

use revue::event::{Key, KeyEvent};
use revue::widget::Input;

/// revue `Input` 的 readline 扩展。用法：`use InputReadlineExt;` 后
/// `input.insert_text(..)` / `input.readline_ctrl(..)` 与固有方法同口径调用。
pub trait InputReadlineExt {
    /// 粘贴入口：剥控制字符（Input 是单行部件），逐字经 handle_key
    /// 插入（光标处插入 + 每字一记 undo，与逐字输入同语义）。
    fn insert_text(&mut self, s: &str);

    /// readline ctrl 集。返回 true=已消费（含未绑定吞掉）——调用方
    /// 据此停止传播，绝不剥修饰键退化成插入字面字母。
    fn readline_ctrl(&mut self, event: &KeyEvent) -> bool;
}

impl InputReadlineExt for Input {
    fn insert_text(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.handle_key(&Key::Char(c));
        }
    }

    fn readline_ctrl(&mut self, event: &KeyEvent) -> bool {
        match event.key {
            // readline 语义：A=行首 E=行尾（revue 固有的 ctrl+a=select_all
            // 在弹窗字段里没有对应手势，统一为 readline 口径）。
            Key::Char('a') => {
                self.handle_key(&Key::Home);
            }
            Key::Char('e') => {
                self.handle_key(&Key::End);
            }
            Key::Char('w') | Key::Backspace => kill_word_before(self),
            Key::Char('u') => kill_to_line_start(self),
            Key::Char('k') => kill_to_line_end(self),
            Key::Char('z') => {
                self.undo();
            }
            Key::Char('y') => {
                self.redo();
            }
            // 词跳：revue 固有 ctrl+←/→ 已实现（含 shift 选区），直接透传。
            Key::Left | Key::Right => {
                self.handle_key_event(event);
            }
            // 未绑定 chord：吞掉。
            _ => {}
        }
        true
    }
}

/// 词起点（readline 口径）：先回跳过空白，再回跳过非空白。
/// 输入/输出均为 char 索引（revue Input 的 cursor 同口径）。
pub(crate) fn word_start_before(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut start = cursor.min(chars.len());
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    start
}

fn kill_word_before(input: &mut Input) {
    let start = word_start_before(input.get_value(), input.cursor());
    for _ in start..input.cursor() {
        input.handle_key(&Key::Backspace);
    }
}

fn kill_to_line_start(input: &mut Input) {
    while input.cursor() > 0 {
        input.handle_key(&Key::Backspace);
    }
}

fn kill_to_line_end(input: &mut Input) {
    while input.cursor() < input.get_value().chars().count() {
        input.handle_key(&Key::Delete);
    }
}

// ── 多行文本的 (line, col) ⇄ 线性 char 索引换算（prompt TextArea kill 用）──

/// 线性 char 索引 → (line, col)，均为 char 口径。
pub(crate) fn linear_to_line_col(content: &str, idx: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in content.chars().enumerate() {
        if i == idx {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// (line, col) → 线性 char 索引（越界 clamp 到行尾/文末）。
pub(crate) fn line_col_to_linear(content: &str, line: usize, col: usize) -> usize {
    let mut idx = 0;
    for (l, text) in content.split('\n').enumerate() {
        if l == line {
            return idx + col.min(text.chars().count());
        }
        idx += text.chars().count() + 1; // + '\n'
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    #[test]
    fn insert_text_strips_control_chars() {
        let mut input = Input::new().focused(true);
        input.insert_text("ab\ncd\te");
        assert_eq!(input.get_value(), "abcde");
    }

    #[test]
    fn readline_ctrl_word_and_line_kills() {
        let mut input = Input::new().focused(true);
        input.insert_text("hello world");
        input.readline_ctrl(&ctrl(Key::Char('w')));
        assert_eq!(input.get_value(), "hello ");
        input.readline_ctrl(&ctrl(Key::Char('u')));
        assert_eq!(input.get_value(), "");
        input.insert_text("abc");
        input.readline_ctrl(&ctrl(Key::Char('a')));
        assert_eq!(input.cursor(), 0);
        input.readline_ctrl(&ctrl(Key::Char('k')));
        assert_eq!(input.get_value(), "");
    }

    #[test]
    fn readline_ctrl_unbound_swallowed_without_letter() {
        let mut input = Input::new().focused(true);
        input.insert_text("x");
        assert!(input.readline_ctrl(&ctrl(Key::Char('g'))));
        assert_eq!(input.get_value(), "x", "未绑定 chord 不得退化成插入字母");
    }

    #[test]
    fn readline_ctrl_undo_redo_roundtrip() {
        let mut input = Input::new().focused(true);
        // CJK 逐字插入 + undo/redo（revue Input undo 是 char 粒度,安全）。
        input.insert_text("你好a");
        input.readline_ctrl(&ctrl(Key::Char('z')));
        assert_eq!(input.get_value(), "你好");
        input.readline_ctrl(&ctrl(Key::Char('y')));
        assert_eq!(input.get_value(), "你好a");
    }

    #[test]
    fn line_col_linear_roundtrip() {
        let content = "ab\ncde\nf";
        assert_eq!(line_col_to_linear(content, 1, 2), 5);
        assert_eq!(linear_to_line_col(content, 5), (1, 2));
        assert_eq!(
            line_col_to_linear(content, 0, 99),
            2,
            "col 越界 clamp 到行尾"
        );
        assert_eq!(linear_to_line_col(content, 0), (0, 0));
    }

    #[test]
    fn word_start_before_skips_trailing_spaces() {
        assert_eq!(word_start_before("hello world", 11), 6);
        assert_eq!(word_start_before("hello ", 6), 0);
        assert_eq!(word_start_before("你好 世界", 4), 3);
    }
}
