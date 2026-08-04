//! 金 — Session fork dialog: fork from a specific message.
//!
//! F9 接线：open_with_messages 列出最近消息（最新在上，首行 "(latest)" =
//! 整会话 fork），↑↓ 选择，Enter 返回 (session_id, Option<message_id>)。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop::{self, ListItem};

/// 可选 fork 锚点：None = 整会话（最新），Some = 该消息。
#[derive(Clone)]
pub struct ForkMessageOption {
    pub message_id: Option<String>,
    pub label: String,
}

pub struct SessionForkDialog {
    pub visible: bool,
    pub session_id: String,
    options: Vec<ForkMessageOption>,
    selected: usize,
}

impl Default for SessionForkDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionForkDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            session_id: String::new(),
            options: Vec::new(),
            selected: 0,
        }
    }

    /// 整会话 fork（无消息列表——兼容旧调用点）。
    pub fn open(&mut self, session_id: &str, message_id: Option<&str>) {
        let options = vec![ForkMessageOption {
            message_id: message_id.map(|s| s.to_string()),
            label: format!(
                "Fork from: {}",
                message_id.unwrap_or("(latest)")
            ),
        }];
        self.open_with_options(session_id, options);
    }

    /// F9：列最近消息供选择（调用方已按偏好排序，首项通常为 latest）。
    pub fn open_with_messages(&mut self, session_id: &str, options: Vec<ForkMessageOption>) {
        self.open_with_options(session_id, options);
    }

    fn open_with_options(&mut self, session_id: &str, options: Vec<ForkMessageOption>) {
        self.session_id = session_id.to_string();
        self.options = options;
        self.selected = 0;
        self.visible = true;
    }

    pub fn close(&mut self) { self.visible = false; }
    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<(String, Option<String>)> {
        if !self.visible { return None; }
        if self.options.is_empty() {
            if matches!(key, Key::Escape) { self.close(); }
            return None;
        }
        let len = self.options.len();
        match key {
            Key::Up    => { self.selected = (self.selected + len - 1) % len; None }
            Key::Down  => { self.selected = (self.selected + 1) % len; None }
            Key::Home  => { self.selected = 0; None }
            Key::End   => { self.selected = len - 1; None }
            Key::Escape => { self.close(); None }
            Key::Enter => {
                let sid = self.session_id.clone();
                let mid = self.options.get(self.selected).and_then(|o| o.message_id.clone());
                self.close();
                Some((sid, mid))
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        let items: Vec<ListItem> = self.options.iter().enumerate().map(|(i, o)| {
            let marker = if i == self.selected { "▶ " } else { "  " };
            ListItem::Row {
                display: format!("{}{}", marker, o.label),
                muted: o.message_id.is_none(),
            }
        }).collect();
        backdrop::render_list_dialog_bottom(
            "Fork Session",
            colors::ACCENT_PURPLE(),
            &items,
            self.selected,
            "↑↓ select anchor  Home/End: jump  Enter: fork  Esc: cancel",
            ctx, geom, 10,
        );
    }
}
