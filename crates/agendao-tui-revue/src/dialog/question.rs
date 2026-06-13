//! 金 — Question dialog: agent asks user a question.
//!
//! Mirrors old TUI: QuestionOption(id, label, desc), single/multi-select.

use revue::prelude::*;
use revue::event::Key;

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

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let Some(req) = self.requests.first() else { return; };
        let queue_hint = if self.requests.len() > 1 { format!(" ({} more)", self.requests.len() - 1) } else { String::new() };

        let is_multi = self.toggled.iter().filter(|&&t| t).count() > 0 || req.options.len() > 1;
        let hint = if is_multi { "Space: toggle | Enter: confirm" } else { "↑↓: choose | Enter: select" };

        let mut content = vstack().gap(1)
            .child(Text::new(format!("{} {}", req.text, queue_hint)).bold().class("DialogBody"))
            .child(Text::new(hint).fg(Color::rgb(86, 95, 137)));

        for (i, opt) in req.options.iter().enumerate() {
            let marker = if is_multi {
                if self.toggled.get(i).copied().unwrap_or(false) { "☑" } else { "☐" }
            } else if i == self.selected { "▶" } else { " " };
            let color = if i == self.selected { Color::rgb(125, 207, 255) } else { Color::rgb(169, 177, 214) };
            let label = if opt.description.is_empty() { opt.label.clone() } else { format!("{} — {}", opt.label, opt.description) };
            content = content.child(Text::new(format!("{} {}", marker, label)).fg(color));
        }
        content = content.child(Text::new("Esc: skip").fg(Color::rgb(86, 95, 137)));

        let dialog = Border::rounded().title(" Question ").fg(Color::rgb(125, 207, 255)).child(content);

        let w = 54u16.min(ctx.area.width - 4);
        let h = (req.options.len() as u16 + 5).min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2;
        let y = (ctx.area.height - h) / 2;
        revue::widget::positioned(dialog).x(x as i16).y(y as i16).width(w).height(h).render(ctx);
    }
}
