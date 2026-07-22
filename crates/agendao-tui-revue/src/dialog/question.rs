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

impl Default for QuestionDialog {
    fn default() -> Self {
        Self::new()
    }
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

    /// 返回 (question_id, answer_labels)。answer 用 option.label 而非 index——
     /// 与 web `InteractionOverlays.tsx:132` 同契约,server `answer_question`
     /// 期望 `Vec<Vec<String>>` 为答案值数组(每个外层项对应一道题、内层为该题
     /// 选中的多个值);本对话框一次只承载一题,故返回单层 `Vec<String>`,由
     /// 调用方按 server 契约外包一层。
     ///
     /// 修复了此前 panel_dispatch 用空 id + 索引字符串发送、server 永远匹配
     /// 不上的 bug(道纪第十条「有阴无阳」:展示有了,回流断了)。
     pub fn handle_key(&mut self, key: &Key) -> Option<(String, Vec<String>)> {
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
                 // 收集选中 indices;空选回退到当前 selected(单选语义)。
                 let chosen: Vec<usize> = self.toggled.iter().enumerate()
                     .filter(|(_, &t)| t).map(|(i, _)| i).collect();
                 let chosen = if chosen.is_empty() { vec![self.selected] } else { chosen };
                 // 在 remove(0) 之前先把 id + labels 取出——之后 requests.first()
                 // 已是下一题,无法回查当前题的 options。
                 let qid = req.id.clone();
                 let labels: Vec<String> = chosen.iter()
                     .filter_map(|&i| req.options.get(i).map(|o| o.label.clone()))
                     .collect();
                 self.requests.remove(0);
                 if self.requests.is_empty() { self.visible = false; }
                 else if let Some(next) = self.requests.first() {
                     self.toggled = vec![false; next.options.len().max(1)];
                     self.selected = 0;
                 }
                 Some((qid, labels))
             }
             Key::Escape => { self.requests.remove(0); if self.requests.is_empty() { self.visible = false; } None }
             _ => None,
         }
     }

    /// 内联成形:pending question 渲染成 transcript 流末尾顶格块
    /// (`? {text}` header + ❯ 单选 / ☑ 多选 选项)。无 modal 边框。
    /// 鼠标 hit-test 省略（permission 块已改为 render 发布命中矩形；
    /// question 无折叠交互，仍只走键盘）。
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
                    .fg(colors::ACCENT_CYAN()),
                1,
            );
        let mut height: u16 = 1;

        for (i, opt) in req.options.iter().enumerate() {
            let marker = if is_multi {
                if self.toggled.get(i).copied().unwrap_or(false) { "☑ " } else { "☐ " }
            } else if i == self.selected { "❯ " } else { "  " };
            let color = if (i == self.selected && !is_multi)
                || (is_multi && self.toggled.get(i).copied().unwrap_or(false)) {
                colors::ACCENT_CYAN()
            } else {
                colors::FG_SECONDARY()
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
            Text::new(format!(" {}", hint)).fg(colors::FG_MUTED()),
            1,
        );
        height += 1;

        Some(BlockLayout { height, view: content })
    }
}
