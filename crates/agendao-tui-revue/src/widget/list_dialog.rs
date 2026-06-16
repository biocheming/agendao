//! ListDialogState<T> — 泛型选择列表状态机（土律归一）。
//!
//! 治理目标：多个选择类 dialog（agent_select / model_select /
//! session_list / prompt_stash / ...）各自重复 Up/Down/Enter/Esc
//! handle_key + 边界处理。收敛到一个泛型状态：持有 items + selected +
//! query，统一导航/过滤语义。
//!
//! 阴阳定位：阳面是各 dialog 的 handle_key 调用（输入生发，木律），
//! 阴面是本状态的 selected/query 唯一所有权（收束归一，土律）。
//! [`key_name`] 把 revue `Key` 归一为字符串，避免各 dialog 重复
//! `match Key` —— 木律：输入变体归一到唯一权威。

use revue::event::Key;

/// 列表对话框的按键结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListAction {
    /// Enter，带选中 index。
    Confirm(usize),
    /// Esc。
    Cancel,
    /// 未消费（导航/过滤）。
    None,
}

#[derive(Clone, Debug)]
pub struct ListDialogState<T: Clone> {
    pub items: Vec<T>,
    pub selected: usize,
    pub query: String,
}

impl<T: Clone> ListDialogState<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self { items, selected: 0, query: String::new() }
    }

    /// 统一 handle_key 核心：返回 [`ListAction`] + 自身突变。
    ///
    /// `key_name` 是归一后的按键名（由 [`key_name`] 从 revue `Key` 产出）。
    /// 空列表只响应 `esc`（其余 key 不 panic、不越界）。
    /// 导航为循环（wrap-around）：到底再 Down 回顶，到顶再 Up 到底，
    /// 对齐 claudecode 选择列表的边界行为。
    pub fn handle(&mut self, key_name: &str) -> ListAction {
        if self.items.is_empty() {
            return match key_name { "esc" => ListAction::Cancel, _ => ListAction::None };
        }
        let len = self.items.len();
        match key_name {
            "up"    => { self.selected = (self.selected + len - 1) % len; ListAction::None }
            "down"  => { self.selected = (self.selected + 1) % len; ListAction::None }
            "enter" => ListAction::Confirm(self.selected),
            "esc"   => ListAction::Cancel,
            "home"  => { self.selected = 0; ListAction::None }
            "end"   => { self.selected = len - 1; ListAction::None }
            c if c.starts_with("char:") => {
                // "char:" 是 5 个 ASCII 字节；其后的首 code point 入 query。
                self.query.push(c[5..].chars().next().unwrap_or(' '));
                ListAction::None
            }
            "backspace" => { self.query.pop(); ListAction::None }
            _ => ListAction::None,
        }
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.items.get(self.selected)
    }
}

/// 简单子串 fuzzy 过滤（大小写不敏感）。返回匹配 item 的原 index 列表。
///
/// 留作通用过滤原语：未挂 query 的扁平列表可直接用；带分组/header
/// 的领域列表（如 model_select 的 FlatRow）保留各自的过滤逻辑——
/// 那是金律成形领域语义，不在本原语职责内。
pub fn fuzzy_filter<T: AsRef<str>>(items: &[T], query: &str) -> Vec<usize> {
    if query.is_empty() { return (0..items.len()).collect(); }
    let q = query.to_lowercase();
    items.iter().enumerate()
        .filter(|(_, it)| it.as_ref().to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// 把 revue `Key` 归一为 [`ListDialogState::handle`] 能消费的按键名。
///
/// 让各 dialog 不必各自重复 `match Key` —— 木律：输入变体归一到单点。
/// 不认识的键归一为 `"other"`，`handle` 对其返回 [`ListAction::None`]。
pub fn key_name(key: &Key) -> String {
    match key {
        Key::Up        => "up".into(),
        Key::Down      => "down".into(),
        Key::Enter     => "enter".into(),
        Key::Escape    => "esc".into(),
        Key::Home      => "home".into(),
        Key::End       => "end".into(),
        Key::Backspace => "backspace".into(),
        Key::Char(c)   => format!("char:{}", c),
        _ => "other".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_wraps_around() {
        let mut s = ListDialogState::new(vec!["a", "b", "c"]);
        s.selected = 2;
        s.handle("down");
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn up_wraps_around() {
        let mut s = ListDialogState::new(vec!["a", "b", "c"]);
        s.handle("up");
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn enter_confirms_selected() {
        let mut s = ListDialogState::new(vec!["a", "b", "c"]);
        s.selected = 1;
        assert_eq!(s.handle("enter"), ListAction::Confirm(1));
    }

    #[test]
    fn esc_cancels() {
        let mut s = ListDialogState::new(vec!["a"]);
        assert_eq!(s.handle("esc"), ListAction::Cancel);
    }

    #[test]
    fn empty_list_only_esc() {
        let mut s: ListDialogState<&str> = ListDialogState::new(vec![]);
        assert_eq!(s.handle("down"), ListAction::None);
        assert_eq!(s.handle("esc"), ListAction::Cancel);
    }

    #[test]
    fn fuzzy_filter_substring() {
        let items = ["apple", "banana", "apricot"];
        assert_eq!(fuzzy_filter(&items, "ap"), vec![0, 2]);
        assert_eq!(fuzzy_filter(&items, ""), vec![0, 1, 2]);
    }

    #[test]
    fn home_end_jump() {
        let mut s = ListDialogState::new(vec!["a", "b", "c"]);
        s.selected = 1;
        s.handle("home");
        assert_eq!(s.selected, 0);
        s.handle("end");
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn key_name_normalizes_revue_key() {
        assert_eq!(key_name(&Key::Up), "up");
        assert_eq!(key_name(&Key::Down), "down");
        assert_eq!(key_name(&Key::Enter), "enter");
        assert_eq!(key_name(&Key::Escape), "esc");
        assert_eq!(key_name(&Key::Char('a')), "char:a");
        assert_eq!(key_name(&Key::Char('你')), "char:你");
    }
}
