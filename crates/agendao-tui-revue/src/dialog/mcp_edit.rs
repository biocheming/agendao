//! 金 — MCP Server Add/Edit Dialog（Settings→MCP 的 a/e 入口）。
//!
//! 字段：Name(key) / Transport(‹ local ›/‹ remote › ←/→ 循环) / Command / Url
//! （四字段 form，与 ModelEditDialog 同范式）。enabled 不入表单——启停走列表
//! `t` 键单点权威；Edit 提交时透传原值（`open_edit` 记入，submit 带出）。
//!
//! 验证（Enter 时）：Name 必填；Transport=local → Command 必填；
//! Transport=remote → Url 必填。不满足则静默不提交（同 model_edit 口径）。
//!
//! 上游（Settings keymap）调：
//! - `dialog.handle_key(&key)` → `Some(Action::Submit(...))` 时，AppHandler
//!   组装 `McpServerConfig` 调 `put_mcp_config`（PUT `/config/mcp/{key}`）；
//! - close 在 submit / Cancel / Esc 三路对称（道纪·第九条）。

use revue::event::Key;
use revue::prelude::*;
use revue::widget::Border;

use crate::dialog::backdrop;
use crate::input::readline::InputReadlineExt;
use crate::theme::colors;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpEditMode {
    Add,
    Edit,
}

/// transport 选项：local = command 数组；remote = url（与 server
/// `parse_runtime_from_loaded_config` 判别口径同源：有 url → remote）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpTransport {
    Local,
    Remote,
}

impl McpTransport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Local => Self::Remote,
            Self::Remote => Self::Local,
        }
    }

    fn prev(self) -> Self {
        self.next() // 两选项：prev == next
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpEditField {
    Name,
    Transport,
    Command,
    Url,
}

impl McpEditField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Transport,
            Self::Transport => Self::Command,
            Self::Command => Self::Url,
            Self::Url => Self::Name,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Name => Self::Url,
            Self::Transport => Self::Name,
            Self::Command => Self::Transport,
            Self::Url => Self::Command,
        }
    }
}

pub enum McpEditAction {
    Submit(Box<McpEditSubmission>),
    Cancel,
}

/// 提交载荷。AppHandler 据此组装 `McpServerConfig::Full`：
/// local → command 按空白拆分；remote → url。`enabled` 为透传值
/// （Add = true；Edit = 原条目的启停态，不被表单重置）。
pub struct McpEditSubmission {
    pub mode: McpEditMode,
    /// server key：Add = 用户填的 name；Edit = 原 key（Name 只读）。
    pub name: String,
    pub transport: McpTransport,
    pub command: String,
    pub url: String,
    pub enabled: bool,
}

pub struct McpEditDialog {
    pub visible: bool,
    pub mode: McpEditMode,
    /// Edit 模式原 server key（Name 只读——改 key = 删旧加新，不走 Edit）。
    origin_name: String,
    name_input: revue::widget::Input,
    transport: McpTransport,
    command_input: revue::widget::Input,
    url_input: revue::widget::Input,
    enabled: bool,
    focus: McpEditField,
    /// 校验错误（U5）：Enter 校验失败置位——不关窗、聚焦出错字段、红字渲染
    /// 在 footer 上方；任何编辑键（含 ctrl chord/粘贴）清除。
    validation_error: Option<String>,
}

impl McpEditDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: McpEditMode::Add,
            origin_name: String::new(),
            name_input: revue::widget::Input::new().placeholder("e.g. filesystem"),
            transport: McpTransport::Local,
            command_input: revue::widget::Input::new()
                .placeholder("e.g. npx -y @modelcontextprotocol/server-filesystem /tmp"),
            url_input: revue::widget::Input::new()
                .placeholder("e.g. https://mcp.example.com/sse"),
            enabled: true,
            focus: McpEditField::Name,
            validation_error: None,
        }
    }

    pub fn open_add(&mut self) {
        self.mode = McpEditMode::Add;
        self.origin_name.clear();
        self.name_input = revue::widget::Input::new().placeholder("e.g. filesystem");
        self.transport = McpTransport::Local;
        self.command_input = revue::widget::Input::new()
            .placeholder("e.g. npx -y @modelcontextprotocol/server-filesystem /tmp");
        self.url_input = revue::widget::Input::new()
            .placeholder("e.g. https://mcp.example.com/sse");
        self.enabled = true;
        self.focus = McpEditField::Name;
        self.validation_error = None;
        self.visible = true;
    }

    /// Edit 预填：从 SettingsMcpRow（refresh 时已合并 config 字段）取
    /// transport/command/url/enabled；Name 置原 key（只读）。
    pub fn open_edit(&mut self, row: &crate::store::types::SettingsMcpRow) {
        self.mode = McpEditMode::Edit;
        self.origin_name = row.name.clone();
        self.name_input = revue::widget::Input::new()
            .placeholder("Server name")
            .value(row.name.clone());
        self.transport = if row.transport == "remote" {
            McpTransport::Remote
        } else {
            McpTransport::Local
        };
        self.command_input = revue::widget::Input::new()
            .placeholder("e.g. npx -y @modelcontextprotocol/server-filesystem /tmp")
            .value(row.command.clone().unwrap_or_default());
        self.url_input = revue::widget::Input::new()
            .placeholder("e.g. https://mcp.example.com/sse")
            .value(row.url.clone().unwrap_or_default());
        self.enabled = row.enabled;
        self.focus = McpEditField::Name;
        self.validation_error = None;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.origin_name.clear();
        self.name_input.clear();
        self.command_input.clear();
        self.url_input.clear();
        self.enabled = true;
        self.validation_error = None;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<McpEditAction> {
        if !self.visible {
            return None;
        }
        // 用户开始改正即撤错误红字（Enter 会按需重新置位）。
        if !matches!(key, Key::Enter) {
            self.validation_error = None;
        }
        match key {
            Key::Escape => {
                self.close();
                Some(McpEditAction::Cancel)
            }
            Key::Enter => {
                let name = if self.mode == McpEditMode::Edit {
                    self.origin_name.clone()
                } else {
                    self.name_input.text().trim().to_string()
                };
                let command = self.command_input.text().trim().to_string();
                let url = self.url_input.text().trim().to_string();
                // U5：校验失败不再静默——置错误文案（红字渲染）+ 聚焦出错字段，不关窗。
                if name.is_empty() {
                    self.validation_error = Some("Name is required".into());
                    self.focus = McpEditField::Name;
                    return None;
                }
                if self.transport == McpTransport::Local && command.is_empty() {
                    self.validation_error = Some("Command is required for local transport".into());
                    self.focus = McpEditField::Command;
                    return None;
                }
                if self.transport == McpTransport::Remote && url.is_empty() {
                    self.validation_error = Some("URL is required for remote transport".into());
                    self.focus = McpEditField::Url;
                    return None;
                }
                let submission = McpEditSubmission {
                    mode: self.mode,
                    name,
                    transport: self.transport,
                    command,
                    url,
                    enabled: self.enabled,
                };
                self.close();
                Some(McpEditAction::Submit(Box::new(submission)))
            }
            Key::Tab => {
                self.focus = self.focus.next();
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev();
                None
            }
            // Transport 字段下 ←/→ 切选项（其他字段走 Input 光标移动）。
            Key::Left | Key::Right if self.focus == McpEditField::Transport => {
                self.transport = match key {
                    Key::Left => self.transport.prev(),
                    _ => self.transport.next(),
                };
                None
            }
            _ => {
                match self.focus {
                    McpEditField::Name => {
                        // Edit 模式 name 只读（改 key = 删旧加新）。
                        if self.mode == McpEditMode::Add {
                            let _ = self.name_input.handle_key(key);
                        }
                    }
                    McpEditField::Transport => {
                        // 吞掉文字键——Transport 只接 ←/→。
                    }
                    McpEditField::Command => {
                        let _ = self.command_input.handle_key(key);
                    }
                    McpEditField::Url => {
                        let _ = self.url_input.handle_key(key);
                    }
                }
                None
            }
        }
    }

    /// Ctrl 组合键 → 当前 focus 的文本 Input（readline 编辑；未绑定 chord 由
    /// Input 吞掉，防退化插入字母/漏全局键）。choice/toggle/只读字段一律吞掉。
    pub fn handle_ctrl_key(&mut self, event: &KeyEvent) -> bool {
        if !self.visible {
            return false;
        }
        self.validation_error = None;
        match self.focus {
            McpEditField::Name => {
                if self.mode == McpEditMode::Add {
                    self.name_input.readline_ctrl(event)
                } else {
                    true
                }
            }
            McpEditField::Command => self.command_input.readline_ctrl(event),
            McpEditField::Url => self.url_input.readline_ctrl(event),
            McpEditField::Transport => true,
        }
    }

    /// 粘贴 → 当前 focus 的文本 Input；非文本字段吞掉（不落到背后的 prompt）。
    pub fn paste_text(&mut self, text: &str) -> bool {
        if !self.visible {
            return false;
        }
        self.validation_error = None;
        match self.focus {
            McpEditField::Name if self.mode == McpEditMode::Add => {
                self.name_input.insert_text(text)
            }
            McpEditField::Command => self.command_input.insert_text(text),
            McpEditField::Url => self.url_input.insert_text(text),
            _ => {}
        }
        true
    }

    pub fn render(&self, ctx: &mut RenderContext, cursor_on: bool) -> Option<revue::prelude::Rect> {
        if !self.visible {
            return None;
        }
        let title = match self.mode {
            McpEditMode::Add => " Add MCP Server ",
            McpEditMode::Edit => " Edit MCP Server ",
        };

        let name_field = field_input(
            "Name (config key)",
            self.name_input.clone(),
            self.focus == McpEditField::Name,
            self.mode == McpEditMode::Edit, // Edit 时只读 hint
            cursor_on,
        );
        let transport_field = field_choice(
            "Transport",
            self.transport.label(),
            self.focus == McpEditField::Transport,
        );
        let command_field = field_input(
            "Command (local transport)",
            self.command_input.clone(),
            self.focus == McpEditField::Command,
            false,
            cursor_on,
        );
        let url_field = field_input(
            "URL (remote transport)",
            self.url_input.clone(),
            self.focus == McpEditField::Url,
            false,
            cursor_on,
        );

        let content = vstack()
            .gap(0)
            .child_sized(name_field, 4)
            .child_sized(transport_field, 4)
            .child_sized(command_field, 4)
            .child_sized(url_field, 4);

        // U5：校验错误红字行（footer 上方），高度随行 +1。
        let (content, err_h) = if let Some(e) = &self.validation_error {
            (content.child_sized(backdrop::validation_error_line(e), 1), 1)
        } else {
            (content, 0)
        };

        // 返回外框 Rect（绝对坐标）：发布给 keymap 做鼠标字段命中（金律·几何同源）。
        Some(backdrop::render_dialog(
            title,
            colors::ACCENT_CYAN(),
            content,
            "Tab: next   ←/→: transport   Enter: save   Esc: cancel",
            ctx,
            76,
            24 + err_h,
        ))
    }
}

impl McpEditDialog {
    /// 鼠标点击设置当前字段（与 Tab 切换同一 `focus` 权威）。
    pub(crate) fn set_focus(&mut self, field: McpEditField) {
        self.focus = field;
    }

    /// 当前焦点字段（测试/命中校验用）。
    #[cfg(test)]
    pub(crate) fn focus(&self) -> McpEditField {
        self.focus
    }

    /// 全部字段（渲染顺序）：鼠标按行块反查字段用。
    pub(crate) const FIELDS: [McpEditField; 4] = [
        McpEditField::Name,
        McpEditField::Transport,
        McpEditField::Command,
        McpEditField::Url,
    ];

    /// 鼠标点击定位光标到字段内字符位置（Transport 选择器无文本，忽略）。
    pub(crate) fn set_cursor_at(&mut self, field: McpEditField, char_idx: usize) {
        match field {
            McpEditField::Name => self.name_input.set_cursor(char_idx),
            McpEditField::Command => self.command_input.set_cursor(char_idx),
            McpEditField::Url => self.url_input.set_cursor(char_idx),
            McpEditField::Transport => {}
        }
    }
}

impl Default for McpEditDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn field_input(
    label: &str,
    mut input: revue::widget::Input,
    focused: bool,
    readonly: bool,
    cursor_on: bool,
) -> revue::widget::Stack {
    let label_color = if readonly {
        colors::FG_MUTED()
    } else if focused {
        colors::E_AMBER()
    } else {
        colors::FG_SECONDARY()
    };
    let border_color = if readonly {
        colors::BORDER()
    } else if focused {
        colors::E_AMBER()
    } else {
        colors::BORDER()
    };
    input = input.focused(focused && !readonly).cursor_visible(cursor_on);
    let label_text = if readonly {
        format!(" {} (read-only)", label)
    } else {
        format!(" {}", label)
    };
    vstack()
        .gap(0)
        .child_sized(Text::new(label_text).fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(input), 3)
}

/// transport 横向选择器：`‹ local ›` 形态，focused 时高亮（与 settings
/// field_block_choice 同语义，dialog 几何内复刻）。
fn field_choice(label: &str, choice_label: &str, focused: bool) -> revue::widget::Stack {
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
    let value_color = if focused {
        colors::FG_PRIMARY()
    } else {
        colors::FG_SECONDARY()
    };
    let value = Text::new(format!("‹ {} ›  (←/→ to change)", choice_label)).fg(value_color);
    vstack()
        .gap(0)
        .child_sized(Text::new(format!(" {}", label)).fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(value), 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row() -> crate::store::types::SettingsMcpRow {
        crate::store::types::SettingsMcpRow {
            name: "fs".into(),
            status: "connected".into(),
            tools: 3,
            resources: 0,
            error: None,
            transport: "local".into(),
            command: Some("npx -y srv /tmp".into()),
            url: None,
            enabled: false,
        }
    }

    #[test]
    fn open_add_defaults_to_local_with_focus_on_name() {
        let mut d = McpEditDialog::new();
        d.open_add();
        assert!(d.is_open());
        assert_eq!(d.mode, McpEditMode::Add);
        assert_eq!(d.transport, McpTransport::Local);
        assert_eq!(d.focus, McpEditField::Name);
    }

    #[test]
    fn open_edit_prefills_and_preserves_enabled() {
        let mut d = McpEditDialog::new();
        d.open_edit(&sample_row());
        assert_eq!(d.mode, McpEditMode::Edit);
        assert_eq!(d.name_input.text(), "fs");
        assert_eq!(d.command_input.text(), "npx -y srv /tmp");
        assert!(!d.enabled, "Edit 必须透传原启停态");
    }

    #[test]
    fn edit_mode_name_is_readonly() {
        let mut d = McpEditDialog::new();
        d.open_edit(&sample_row());
        d.handle_key(&Key::Char('x'));
        assert_eq!(d.name_input.text(), "fs");
    }

    #[test]
    fn transport_left_right_cycles() {
        let mut d = McpEditDialog::new();
        d.open_add();
        d.focus = McpEditField::Transport;
        assert_eq!(d.transport, McpTransport::Local);
        d.handle_key(&Key::Right);
        assert_eq!(d.transport, McpTransport::Remote);
        d.handle_key(&Key::Left);
        assert_eq!(d.transport, McpTransport::Local);
    }

    #[test]
    fn enter_without_required_fields_does_not_submit() {
        let mut d = McpEditDialog::new();
        d.open_add();
        // 空 name。
        assert!(d.handle_key(&Key::Enter).is_none());
        // 有 name 但 local 缺 command。
        for c in "srv".chars() {
            d.handle_key(&Key::Char(c));
        }
        assert!(d.handle_key(&Key::Enter).is_none());
        assert!(d.is_open());
    }

    #[test]
    fn submit_local_carries_fields() {
        let mut d = McpEditDialog::new();
        d.open_add();
        for c in "srv".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → Transport
        d.handle_key(&Key::Tab); // → Command
        for c in "npx srv".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(McpEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.name, "srv");
        assert_eq!(s.transport, McpTransport::Local);
        assert_eq!(s.command, "npx srv");
        assert!(s.enabled);
        assert!(!d.is_open());
    }

    #[test]
    fn submit_remote_requires_url() {
        let mut d = McpEditDialog::new();
        d.open_add();
        for c in "r".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = McpEditField::Transport;
        d.handle_key(&Key::Right); // → remote
        // remote 缺 url → 不提交。
        assert!(d.handle_key(&Key::Enter).is_none());
        d.focus = McpEditField::Url;
        for c in "https://x".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(McpEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.transport, McpTransport::Remote);
        assert_eq!(s.url, "https://x");
    }

    #[test]
    fn esc_returns_cancel_and_closes() {
        let mut d = McpEditDialog::new();
        d.open_add();
        let action = d.handle_key(&Key::Escape);
        assert!(matches!(action, Some(McpEditAction::Cancel)));
        assert!(!d.is_open());
    }

    // ── U5：校验失败反馈（错误文案 + 聚焦 + 不关窗）──

    #[test]
    fn local_without_command_flags_error_and_focuses_command() {
        let mut d = McpEditDialog::new();
        d.open_add();
        d.name_input = revue::widget::Input::new().value("fs".to_string());
        // transport=Local（默认），command 空
        assert!(d.handle_key(&Key::Enter).is_none(), "local 缺 command 不提交");
        assert_eq!(
            d.validation_error.as_deref(),
            Some("Command is required for local transport")
        );
        assert_eq!(d.focus(), McpEditField::Command, "焦点跳到出错字段");
        assert!(d.is_open(), "不关窗");
        // 改正后提交成功。
        d.command_input = revue::widget::Input::new().value("npx srv".to_string());
        assert!(matches!(
            d.handle_key(&Key::Enter),
            Some(McpEditAction::Submit(_))
        ));
    }

    #[test]
    fn remote_without_url_flags_error_and_focuses_url() {
        let mut d = McpEditDialog::new();
        d.open_add();
        d.name_input = revue::widget::Input::new().value("remote-srv".to_string());
        // 切到 remote
        d.handle_key(&Key::Tab); // focus → Transport
        d.handle_key(&Key::Right);
        assert!(d.handle_key(&Key::Enter).is_none(), "remote 缺 url 不提交");
        assert_eq!(
            d.validation_error.as_deref(),
            Some("URL is required for remote transport")
        );
        assert_eq!(d.focus(), McpEditField::Url);
        // ctrl chord 编辑也清错误态。
        d.handle_ctrl_key(&KeyEvent { key: Key::Char('u'), ctrl: true, alt: false, shift: false });
        assert_eq!(d.validation_error, None);
    }

    #[test]
    fn empty_name_flags_error_and_focuses_name() {
        let mut d = McpEditDialog::new();
        d.open_add();
        assert!(d.handle_key(&Key::Enter).is_none());
        assert_eq!(d.validation_error.as_deref(), Some("Name is required"));
        assert_eq!(d.focus(), McpEditField::Name);
    }
}
