//! 金 — Question dialog: agent asks user a question.
//!
//! 内联形态:pending question 作为 transcript 流末尾的顶格块渲染
//! (`? {text}` header + ❯/☑ 选项),而非居中浮层。状态所有权不变。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::screen::BlockLayout;

#[derive(Clone)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone)]
pub struct QuestionRequest {
    pub id: String,
    pub text: String,
    pub options: Vec<QuestionOption>,
}

pub struct QuestionDialog {
    pub visible: bool,
    requests: Vec<QuestionRequest>,
    selected: usize,
    toggled: Vec<bool>,
}

impl QuestionDialog {
    pub fn new() -> Self { Self { visible: false, requests: Vec::new(), selected: 0, toggled: Vec::new() } }

    pub fn ask(&mut self, q: QuestionRequest) {
        let n = q.options.len();
        self.toggled = vec![false; n.max(1)];
        self.selected = 0;
        self.requests.push(q);
        self.visible = true;
    }

    pub fn pending_count(&self) -> usize { self.requests.len() }

    /// Close the dialog without clearing pending requests.
    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<Vec<usize>> {
        if !self.visible || self.requests.is_empty() { return None; }
        let req = &self.requests[0];
        let n = req.options.len();
        match key {
            Key::Up => { self.selected = self.selected.saturating_sub(1); None }
            Key::Down => { self.selected = (self.selected + 1).min(n.saturating_sub(1)); None }
            Key::Char(' ') => {
                if let Some(t) = self.toggled.get_mut(self.selected) { *t = !*t; }
                None
            }
            Key::Enter => {
                let result: Vec<usize> = self.toggled.iter().enumerate()
                    .filter(|(_, &t)| t).map(|(i, _)| i).collect();
                let result = if result.is_empty() { vec![self.selected] } else { result };
                self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                else if let Some(next) = self.requests.first() {
                    self.toggled = vec![false; next.options.len().max(1)];
                    self.selected = 0;
                }
                Some(result)
            }
            Key::Escape => { self.requests.remove(0); if self.requests.is_empty() { self.visible = false; } None }
            _ => None,
        }
    }

    /// 内联成形:pending question 渲染成 transcript 流末尾顶格块
    /// (`? {text}` header + ❯ 单选 / ☑ 多选 选项)。无 modal 边框。
    /// 鼠标 hit-test 省略(同 permission:内联位置随滚动变)。
    pub fn render_inline(&self) -> Option<BlockLayout> {
        if !self.visible { return None; }
        let req = self.requests.first()?;

        let queue_hint = if self.requests.len() > 1 {
            format!(" ({}/{})", 1, self.requests.len())
        } else { String::new() };

        let is_multi = self.toggled.iter().filter(|&&t| t).count() > 0 || req.options.len() > 1;
        let hint = if is_multi { "Space toggle · Enter confirm · Esc skip" } else { "↑↓ choose · Enter select · Esc skip" };

        let mut content = vstack().gap(0)
            .child_sized(
                Text::new(format!(" ? {}{}", req.text, queue_hint))
                    .bold()
                    .fg(colors::ACCENT_CYAN),
                1,
            );
        let mut height: u16 = 1;

        for (i, opt) in req.options.iter().enumerate() {
            let marker = if is_multi {
                if self.toggled.get(i).copied().unwrap_or(false) { "☑ " } else { "☐ " }
            } else if i == self.selected { "❯ " } else { "  " };
            let color = if (i == self.selected && !is_multi)
                || (is_multi && self.toggled.get(i).copied().unwrap_or(false)) {
                colors::ACCENT_CYAN
            } else {
                colors::FG_SECONDARY
            };
            let label = if opt.description.is_empty() {
                opt.label.clone()
            } else {
                format!("{} — {}", opt.label, opt.description)
            };
            content = content.child_sized(
                Text::new(format!("{}{}", marker, label)).fg(color),
                1,
            );
            height += 1;
        }

        content = content.child_sized(
            Text::new(format!(" {}", hint)).fg(colors::FG_MUTED),
            1,
        );
        height += 1;

        Some(BlockLayout { height, view: content })
    }
}
