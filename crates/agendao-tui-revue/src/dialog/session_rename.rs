//! 金 — Session rename dialog: inline text input for new title.

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

pub struct SessionRenameDialog {
    pub visible: bool,
    pub session_id: String,
    pub current_title: String,
    input: revue::widget::Input,
}

impl SessionRenameDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            session_id: String::new(),
            current_title: String::new(),
            input: revue::widget::Input::new().placeholder("New session name..."),
        }
    }

    pub fn open(&mut self, session_id: &str, current_title: &str) {
        self.session_id = session_id.to_string();
        self.current_title = current_title.to_string();
        self.input = revue::widget::Input::new()
            .placeholder("New session name...")
            .value(current_title);
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    pub fn is_open(&self) -> bool { self.visible }

    pub fn handle_key(&mut self, key: &Key) -> Option<(String, String)> {
        if !self.visible { return None; }
        match key {
            Key::Enter => {
                let new_title = self.input.text().trim().to_string();
                if !new_title.is_empty() && new_title != self.current_title {
                    let sid = self.session_id.clone();
                    let title = new_title;
                    self.close();
                    return Some((sid, title));
                }
                self.close();
                None
            }
            Key::Escape => { self.close(); None }
            _ => { self.input.handle_key(key); None }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext, geom: backdrop::PromptGeom) {
        if !self.visible { return; }
        let content = vstack().gap(1)
            .child(Text::new("Rename session:").bold().fg(colors::ACCENT_CYAN))
            // child_sized(border, 3)：revue Border 是 unsized 子件，stack 只给自然高
            // (≈2 行)→ inner_h=height-2=0，Input 输入域不渲染，只剩 ╭╰ 紧贴。显式
            // 要 3 行（╭/│input│/╰），inner_h=1 容纳 Input。max_h 同步抬到 8 让
            // content 区(label1+gap1+border3=5 行)放得下。既有 sizing 缺陷，验证时暴露。
            .child_sized(
                Border::rounded().fg(colors::BORDER).child(self.input.clone()),
                3,
            );

        backdrop::render_dialog_bottom(
            "Rename Session",
            colors::ACCENT_CYAN,
            content,
            "Enter: confirm  Esc: cancel",
            ctx, geom, 8,
        );
    }
}
