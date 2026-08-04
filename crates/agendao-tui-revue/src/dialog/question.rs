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

impl QuestionRequest {
    /// From a server `QuestionInfo`（live `QuestionUpsert` 与 F4 catch-up
    /// `list_questions` 共用同一映射，土律·单点权威）。
    pub fn from_info(info: &agendao_client::QuestionInfo) -> QuestionRequest {
        let qtext = info.questions.first().cloned().unwrap_or_default();
        let options: Vec<QuestionOption> = if let Some(item) = info.items.first() {
            item.options
                .iter()
                .enumerate()
                .map(|(i, o)| QuestionOption {
                    id: format!("opt_{}", i),
                    label: o.label.clone(),
                    description: o.description.clone().unwrap_or_default(),
                })
                .collect()
        } else {
            // Fallback: flat string options
            info.options
                .as_ref()
                .map(|flat_opts| {
                    flat_opts
                        .iter()
                        .enumerate()
                        .map(|(i, opt)| {
                            let label = opt.first().cloned().unwrap_or_default();
                            QuestionOption {
                                id: format!("opt_{}", i),
                                label,
                                description: String::new(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        QuestionRequest {
            id: info.id.clone(),
            text: qtext,
            options,
        }
    }
}

pub struct QuestionDialog {
    pub visible: bool,
    requests: Vec<QuestionRequest>,
    selected: usize,
    toggled: Vec<bool>,
}

/// U8：handle_key 的结果三态——作答 / 显式跳过 / 未决（None）。
/// 此前 Esc 静默 remove(0)（跳过无任何痕迹）；现在跳过必须显式按 `s`，
/// 由调用方补 toast 告知后果；Esc 只收起不决策。
pub enum QuestionKeyOutcome {
    /// (question_id, answer_labels) —— 契约见 handle_key 文档。
    Answered(String, Vec<String>),
    /// 用户显式跳过 head 题（`s` 键）：请求出队，server 侧超时自决。
    Skipped,
}

impl Default for QuestionDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl QuestionDialog {
    pub fn new() -> Self { Self { visible: false, requests: Vec::new(), selected: 0, toggled: Vec::new() } }

    pub fn ask(&mut self, q: QuestionRequest) {
        // Deduplicate: 与 PermissionDialog::add_request 同口径——catch-up 与
        // live QuestionUpsert 可能先后到达同一 id，重复入队会叠弹。
        if self.requests.iter().any(|r| r.id == q.id) { return; }
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

    /// 返回按键结果（见 QuestionKeyOutcome）。answer 用 option.label 而非
    /// index——与 web `InteractionOverlays.tsx:132` 同契约,server
    /// `answer_question` 期望 `Vec<Vec<String>>` 为答案值数组(每个外层项对应
    /// 一道题、内层为该题选中的多个值);本对话框一次只承载一题,故返回单层
    /// `Vec<String>`,由调用方按 server 契约外包一层。
    ///
    /// 修复了此前 panel_dispatch 用空 id + 索引字符串发送、server 永远匹配
    /// 不上的 bug(道纪第十条「有阴无阳」:展示有了,回流断了)。
    pub fn handle_key(&mut self, key: &Key) -> Option<QuestionKeyOutcome> {
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
                 Some(QuestionKeyOutcome::Answered(qid, labels))
             }
             Key::Escape => {
                 // U8：Esc = 仅收起，请求保留队列（与 permission 同语义）；
                 // 状态栏 ⏸ 角标 / Ctrl+O 可回到同一题。
                 self.visible = false;
                 None
             }
             Key::Char('s') => {
                 // 显式跳过：出队并由调用方 toast 告知后果，无静默丢弃。
                 self.requests.remove(0);
                 if self.requests.is_empty() { self.visible = false; }
                 else if let Some(next) = self.requests.first() {
                     self.toggled = vec![false; next.options.len().max(1)];
                     self.selected = 0;
                 }
                 Some(QuestionKeyOutcome::Skipped)
             }
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
        let hint = if is_multi { "Space toggle · Enter confirm · s skip · Esc hide" } else { "↑↓ choose · Enter select · s skip · Esc hide" };

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

#[cfg(test)]
mod tests {
    use super::*;

    fn q(id: &str, text: &str) -> QuestionRequest {
        QuestionRequest {
            id: id.into(),
            text: text.into(),
            options: vec![
                QuestionOption { id: "opt_0".into(), label: "Yes".into(), description: String::new() },
                QuestionOption { id: "opt_1".into(), label: "No".into(), description: String::new() },
            ],
        }
    }

    // ── U8：Esc = 仅收起；skip 必须显式（s）且留痕 ──

    #[test]
    fn esc_collapses_but_keeps_queue() {
        let mut d = QuestionDialog::new();
        d.ask(q("q1", "Proceed?"));
        assert_eq!(d.pending_count(), 1);
        let out = d.handle_key(&Key::Escape);
        assert!(out.is_none());
        assert!(!d.visible);
        assert_eq!(d.pending_count(), 1);
        // 重开后 Enter 作答的仍是同一题。
        d.visible = true;
        let out = d.handle_key(&Key::Enter);
        assert!(matches!(
            out,
            Some(QuestionKeyOutcome::Answered(id, labels)) if id == "q1" && labels == vec!["Yes".to_string()]
        ));
        assert_eq!(d.pending_count(), 0);
    }

    #[test]
    fn explicit_skip_dequeues_and_reports() {
        let mut d = QuestionDialog::new();
        d.ask(q("q1", "First?"));
        d.ask(q("q2", "Second?"));
        let out = d.handle_key(&Key::Char('s'));
        assert!(matches!(out, Some(QuestionKeyOutcome::Skipped)));
        assert_eq!(d.pending_count(), 1);
        assert!(d.visible); // 队列未空，弹窗留在下一题
        // 下一题才是 q2。
        let out = d.handle_key(&Key::Enter);
        assert!(matches!(out, Some(QuestionKeyOutcome::Answered(id, _)) if id == "q2"));
        assert!(!d.visible);
    }

    #[test]
    fn no_silent_drop_paths() {
        // Esc 之后队列长度不变（无静默丢弃）；空队列按键无副作用。
        let mut d = QuestionDialog::new();
        d.ask(q("q1", "Only?"));
        d.handle_key(&Key::Escape);
        assert_eq!(d.pending_count(), 1);
        d.visible = false;
        // 不可见时按键一律 None、队列不动。
        assert!(d.handle_key(&Key::Char('s')).is_none());
        assert!(d.handle_key(&Key::Enter).is_none());
        assert_eq!(d.pending_count(), 1);
    }
}
