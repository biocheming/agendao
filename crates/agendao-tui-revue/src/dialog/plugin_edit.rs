//! 金 — Plugin Install Dialog（Settings→Plugins 的 a 入口）。
//!
//! 表单：Name(config key) + Path（两字段，与 ModelEditDialog 同范式）。
//! 仅 Add——安装 = 向 config.plugin 写一条 `type: "file"` 条目
//! （PUT `/config/plugin/{key}`，走 `install_plugin_action` 单点权威）。
//!
//! 验证（Enter 时）：Name / Path 均必填；不满足静默不提交（同 model_edit 口径）。

use revue::event::Key;
use revue::prelude::*;
use revue::widget::Border;

use crate::dialog::backdrop;
use crate::theme::colors;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PluginEditField {
    Name,
    Path,
}

impl PluginEditField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Path,
            Self::Path => Self::Name,
        }
    }
    fn prev(self) -> Self {
        self.next() // 两字段：prev == next
    }
}

pub enum PluginEditAction {
    Submit(Box<PluginEditSubmission>),
    Cancel,
}

/// 提交载荷：AppHandler 组装 `PluginConfig { type: "file", path }`。
pub struct PluginEditSubmission {
    pub name: String,
    pub path: String,
}

pub struct PluginEditDialog {
    pub visible: bool,
    name_input: revue::widget::Input,
    path_input: revue::widget::Input,
    focus: PluginEditField,
}

impl PluginEditDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            name_input: revue::widget::Input::new().placeholder("e.g. my-plugin"),
            path_input: revue::widget::Input::new()
                .placeholder("e.g. /abs/path/to/plugin/index.ts"),
            focus: PluginEditField::Name,
        }
    }

    pub fn open_add(&mut self) {
        self.name_input = revue::widget::Input::new().placeholder("e.g. my-plugin");
        self.path_input = revue::widget::Input::new()
            .placeholder("e.g. /abs/path/to/plugin/index.ts");
        self.focus = PluginEditField::Name;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.name_input.clear();
        self.path_input.clear();
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<PluginEditAction> {
        if !self.visible {
            return None;
        }
        match key {
            Key::Escape => {
                self.close();
                Some(PluginEditAction::Cancel)
            }
            Key::Enter => {
                let name = self.name_input.text().trim().to_string();
                let path = self.path_input.text().trim().to_string();
                if name.is_empty() || path.is_empty() {
                    return None; // 均必填；静默不提交。
                }
                let submission = PluginEditSubmission { name, path };
                self.close();
                Some(PluginEditAction::Submit(Box::new(submission)))
            }
            Key::Tab => {
                self.focus = self.focus.next();
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev();
                None
            }
            _ => {
                match self.focus {
                    PluginEditField::Name => {
                        let _ = self.name_input.handle_key(key);
                    }
                    PluginEditField::Path => {
                        let _ = self.path_input.handle_key(key);
                    }
                }
                None
            }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) -> Option<revue::prelude::Rect> {
        if !self.visible {
            return None;
        }
        let name_field = field_input(
            "Name (config key)",
            self.name_input.clone(),
            self.focus == PluginEditField::Name,
        );
        let path_field = field_input(
            "Path (file plugin entry .ts/.js/.mjs)",
            self.path_input.clone(),
            self.focus == PluginEditField::Path,
        );
        let content = vstack()
            .gap(0)
            .child_sized(name_field, 4)
            .child_sized(path_field, 4);

        // 返回外框 Rect（绝对坐标）：发布给 keymap 做鼠标字段命中（金律·几何同源）。
        Some(backdrop::render_dialog(
            " Install Plugin ",
            colors::ACCENT_CYAN(),
            content,
            "Tab: next   Enter: install   Esc: cancel",
            ctx,
            70,
            16,
        ))
    }
}

impl PluginEditDialog {
    /// 鼠标点击设置当前字段（与 Tab 切换同一 `focus` 权威）。
    pub(crate) fn set_focus(&mut self, field: PluginEditField) {
        self.focus = field;
    }

    /// 全部字段（渲染顺序）：鼠标按行块反查字段用。
    pub(crate) const FIELDS: [PluginEditField; 2] =
        [PluginEditField::Name, PluginEditField::Path];
}

impl Default for PluginEditDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn field_input(
    label: &str,
    mut input: revue::widget::Input,
    focused: bool,
) -> revue::widget::Stack {
    let label_color = if focused {
        colors::E_AMBER()
    } else {
        colors::FG_SECONDARY()
    };
    let border_color = if focused {
        colors::E_AMBER()
    } else {
        colors::BORDER()
    };
    input = input.focused(focused);
    vstack()
        .gap(0)
        .child_sized(Text::new(format!(" {}", label)).fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(input), 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_add_focuses_name() {
        let mut d = PluginEditDialog::new();
        d.open_add();
        assert!(d.is_open());
        assert_eq!(d.focus, PluginEditField::Name);
    }

    #[test]
    fn enter_with_empty_fields_does_not_submit() {
        let mut d = PluginEditDialog::new();
        d.open_add();
        assert!(d.handle_key(&Key::Enter).is_none());
        for c in "p".chars() {
            d.handle_key(&Key::Char(c));
        }
        // 有 name 缺 path → 不提交。
        assert!(d.handle_key(&Key::Enter).is_none());
        assert!(d.is_open());
    }

    #[test]
    fn submit_carries_name_and_path() {
        let mut d = PluginEditDialog::new();
        d.open_add();
        for c in "my-plugin".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → Path
        for c in "/tmp/p/index.ts".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(PluginEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.name, "my-plugin");
        assert_eq!(s.path, "/tmp/p/index.ts");
        assert!(!d.is_open());
    }

    #[test]
    fn esc_returns_cancel_and_closes() {
        let mut d = PluginEditDialog::new();
        d.open_add();
        let action = d.handle_key(&Key::Escape);
        assert!(matches!(action, Some(PluginEditAction::Cancel)));
        assert!(!d.is_open());
    }
}
