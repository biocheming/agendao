//! 金 — Provider Add/Edit Dialog（Settings→Providers 的 a/e 入口）。
//!
//! 字段：Name（Add 可填兼作 id slug / Edit 只读）/ Protocol（‹ › ←/→ 循环，
//! 选项读 `settings_edit_state::PROTOCOL_OPTIONS` 唯一权威）/ Base URL（文本）/
//! API key（password 掩码；Add 必填，Edit 留空 = 不重置 server auth）。
//! 与 McpEditDialog 同范式：4 字段 × 4 行块，渲染经 `backdrop::render_dialog`
//! （实色底不透字），submit 载荷直接是 `ProviderEditSubmission`——
//! 写入走既有 `submit_provider_edit` 单点链路（火/土，不另开第二通路）。
//!
//! 验证（Enter 时，与 in-place `submit_settings_edit` 同口径）：
//! name/base_url 两模式必填；Add 模式 api_key 必填。不满足静默不提交
//! （同 model_edit/mcp_edit 口径），用户继续填。
//!
//! 上游（Settings keymap / panel_dispatch）调：
//! - `dialog.handle_key(&key)` → `Some(Action::Submit(...))` 时，AppHandler
//!   调 `submit_provider_edit`（client → server → refresh_providers_into_store）；
//! - close 在 submit / Cancel / Esc 三路对称（道纪·第九条）；
//!   close 必清 api_key 明文 buffer（配对销毁，明文不驻留）。

use revue::event::Key;
use revue::prelude::*;
use revue::widget::Border;

use crate::app::provider_actions::{ProviderEditMode, ProviderEditSubmission};
use crate::app::settings_edit_state::PROTOCOL_OPTIONS;
use crate::dialog::backdrop;
use crate::theme::colors;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderEditField {
    Name,
    Protocol,
    BaseUrl,
    ApiKey,
}

impl ProviderEditField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Protocol,
            Self::Protocol => Self::BaseUrl,
            Self::BaseUrl => Self::ApiKey,
            Self::ApiKey => Self::Name,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Name => Self::ApiKey,
            Self::Protocol => Self::Name,
            Self::BaseUrl => Self::Protocol,
            Self::ApiKey => Self::BaseUrl,
        }
    }
}

pub enum ProviderEditAction {
    Submit(Box<ProviderEditSubmission>),
    Cancel,
}

pub struct ProviderEditDialog {
    pub visible: bool,
    pub mode: ProviderEditMode,
    /// Edit 模式原 provider id（id 不可改——改 id = 删旧加新，不走 Edit）。
    origin_id: String,
    name_input: revue::widget::Input,
    /// 当前 protocol 选项在 `PROTOCOL_OPTIONS` 内的下标。
    protocol_idx: usize,
    base_url_input: revue::widget::Input,
    /// api_key 输入：Input.password(true)，buffer 明文，UI 显示 `•`。
    api_key_input: revue::widget::Input,
    focus: ProviderEditField,
}

impl ProviderEditDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: ProviderEditMode::Add,
            origin_id: String::new(),
            name_input: revue::widget::Input::new().placeholder("e.g. My OpenAI"),
            protocol_idx: 0,
            base_url_input: revue::widget::Input::new()
                .placeholder("https://api.openai.com/v1"),
            api_key_input: revue::widget::Input::new()
                .password(true)
                .placeholder("sk-..."),
            focus: ProviderEditField::Name,
        }
    }

    pub fn open_add(&mut self) {
        self.mode = ProviderEditMode::Add;
        self.origin_id.clear();
        self.name_input = revue::widget::Input::new().placeholder("e.g. My OpenAI");
        self.protocol_idx = 0; // 默认 openai（最常见）
        self.base_url_input = revue::widget::Input::new()
            .placeholder("https://api.openai.com/v1");
        self.api_key_input = revue::widget::Input::new()
            .password(true)
            .placeholder("sk-...");
        self.focus = ProviderEditField::Name;
        self.visible = true;
    }

    /// Edit 预填：从 `ProviderInfo` 取 name/base_url/protocol；api_key 留空
    /// （留空 = 不重置 server auth，非空 = 重置 auth.json 条目）。
    pub fn open_edit(&mut self, info: &agendao_client::ProviderInfo) {
        self.mode = ProviderEditMode::Edit;
        self.origin_id = info.id.clone();
        self.name_input = revue::widget::Input::new()
            .placeholder("Provider name")
            .value(info.name.clone());
        self.protocol_idx = info
            .protocol
            .as_deref()
            .and_then(|p| PROTOCOL_OPTIONS.iter().position(|(k, _)| *k == p))
            .unwrap_or(0);
        self.base_url_input = revue::widget::Input::new()
            .placeholder("https://api.openai.com/v1")
            .value(info.base_url.clone().unwrap_or_default());
        self.api_key_input = revue::widget::Input::new()
            .password(true)
            .placeholder("(leave blank to keep current key)");
        // Name 只读，焦点直接落 Protocol（第一个可编辑字段）。
        self.focus = ProviderEditField::Protocol;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.origin_id.clear();
        self.name_input.clear();
        self.base_url_input.clear();
        // 关键：api_key 明文 buffer 不驻留（道纪·第九条·配对销毁）。
        self.api_key_input.clear();
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// 当前 protocol key（submission 用，与 server `CONNECT_PROTOCOL_OPTIONS` 同源）。
    fn protocol_key(&self) -> &'static str {
        PROTOCOL_OPTIONS
            .get(self.protocol_idx)
            .map(|(k, _)| *k)
            .unwrap_or("openai")
    }

    fn protocol_label(&self) -> &'static str {
        PROTOCOL_OPTIONS
            .get(self.protocol_idx)
            .map(|(_, l)| *l)
            .unwrap_or("OpenAI")
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<ProviderEditAction> {
        if !self.visible {
            return None;
        }
        match key {
            Key::Escape => {
                self.close();
                Some(ProviderEditAction::Cancel)
            }
            Key::Enter => {
                let name = self.name_input.text().trim().to_string();
                let base_url = self.base_url_input.text().trim().to_string();
                let api_key = self.api_key_input.text().to_string();
                // 验证：name/base_url 必填；Add 模式 api_key 必填
                // （Edit 留空 = 保留原 key）。不满足静默不提交。
                if name.is_empty()
                    || base_url.is_empty()
                    || (self.mode == ProviderEditMode::Add && api_key.is_empty())
                {
                    return None;
                }
                // Add 模式 id slug：lowercase + 空格→`-`（与 in-place 同口径）；
                // Edit 模式直接用 origin_id。
                let id = match self.mode {
                    ProviderEditMode::Add => name
                        .to_lowercase()
                        .chars()
                        .map(|c| if c.is_whitespace() { '-' } else { c })
                        .collect(),
                    ProviderEditMode::Edit => self.origin_id.clone(),
                };
                let submission = ProviderEditSubmission {
                    mode: self.mode,
                    id,
                    name,
                    base_url,
                    protocol: self.protocol_key().to_string(),
                    api_key,
                };
                self.close();
                Some(ProviderEditAction::Submit(Box::new(submission)))
            }
            Key::Tab => {
                self.focus = self.focus.next();
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev();
                None
            }
            // Protocol 字段下 ←/→ 切选项（其他字段走 Input 光标移动）。
            Key::Left | Key::Right if self.focus == ProviderEditField::Protocol => {
                let n = PROTOCOL_OPTIONS.len();
                if n > 0 {
                    self.protocol_idx = match key {
                        Key::Left => (self.protocol_idx + n - 1) % n,
                        _ => (self.protocol_idx + 1) % n,
                    };
                }
                None
            }
            _ => {
                match self.focus {
                    ProviderEditField::Name => {
                        // Edit 模式 name 只读（改 id = 删旧加新，不走 Edit）。
                        if self.mode == ProviderEditMode::Add {
                            let _ = self.name_input.handle_key(key);
                        }
                    }
                    ProviderEditField::Protocol => {
                        // 吞掉文字键——Protocol 只接 ←/→。
                    }
                    ProviderEditField::BaseUrl => {
                        let _ = self.base_url_input.handle_key(key);
                    }
                    ProviderEditField::ApiKey => {
                        let _ = self.api_key_input.handle_key(key);
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
        let title = match self.mode {
            ProviderEditMode::Add => " Add Provider ",
            ProviderEditMode::Edit => " Edit Provider ",
        };

        let name_field = field_input(
            "Name (Add: also used as ID)",
            self.name_input.clone(),
            self.focus == ProviderEditField::Name,
            self.mode == ProviderEditMode::Edit, // Edit 时只读 hint
        );
        let protocol_field = field_choice(
            "Protocol",
            self.protocol_label(),
            self.focus == ProviderEditField::Protocol,
        );
        let base_field = field_input(
            "Base URL",
            self.base_url_input.clone(),
            self.focus == ProviderEditField::BaseUrl,
            false,
        );
        let key_label = match self.mode {
            ProviderEditMode::Add => "API key",
            ProviderEditMode::Edit => "API key (blank = keep current)",
        };
        let key_field = field_input(
            key_label,
            self.api_key_input.clone(),
            self.focus == ProviderEditField::ApiKey,
            false,
        );

        let content = vstack()
            .gap(0)
            .child_sized(name_field, 4)
            .child_sized(protocol_field, 4)
            .child_sized(base_field, 4)
            .child_sized(key_field, 4);

        // 返回外框 Rect（绝对坐标）：发布给 keymap 做鼠标字段命中（金律·几何同源）。
        Some(backdrop::render_dialog(
            title,
            colors::ACCENT_CYAN(),
            content,
            "Tab: next   ←/→: protocol   Enter: save   Esc: cancel",
            ctx,
            76,
            24,
        ))
    }
}

impl ProviderEditDialog {
    /// 鼠标点击设置当前字段（与 Tab 切换同一 `focus` 权威）。
    pub(crate) fn set_focus(&mut self, field: ProviderEditField) {
        self.focus = field;
    }

    /// 当前焦点字段（测试/命中校验用）。
    #[cfg(test)]
    pub(crate) fn focus(&self) -> ProviderEditField {
        self.focus
    }

    /// 全部字段（渲染顺序）：鼠标按行块反查字段用。
    pub(crate) const FIELDS: [ProviderEditField; 4] = [
        ProviderEditField::Name,
        ProviderEditField::Protocol,
        ProviderEditField::BaseUrl,
        ProviderEditField::ApiKey,
    ];
}

impl Default for ProviderEditDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn field_input(
    label: &str,
    mut input: revue::widget::Input,
    focused: bool,
    readonly: bool,
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
    input = input.focused(focused && !readonly);
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

/// protocol 横向选择器：`‹ OpenAI ›` 形态，focused 时高亮（与 mcp_edit
/// field_choice 同语义，dialog 几何内复刻）。
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

    fn sample_provider() -> agendao_client::ProviderInfo {
        agendao_client::ProviderInfo {
            id: "openai".into(),
            name: "OpenAI".into(),
            models: vec![],
            base_url: Some("https://api.openai.com/v1".into()),
            protocol: Some("openai".into()),
            disabled: false,
        }
    }

    #[test]
    fn open_add_defaults_focus_on_name() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        assert!(d.is_open());
        assert_eq!(d.mode, ProviderEditMode::Add);
        assert_eq!(d.focus, ProviderEditField::Name);
        assert_eq!(d.protocol_idx, 0);
    }

    #[test]
    fn open_edit_prefills_and_focuses_protocol() {
        let mut d = ProviderEditDialog::new();
        d.open_edit(&sample_provider());
        assert_eq!(d.mode, ProviderEditMode::Edit);
        assert_eq!(d.name_input.text(), "OpenAI");
        assert_eq!(d.base_url_input.text(), "https://api.openai.com/v1");
        assert_eq!(d.protocol_key(), "openai");
        assert!(d.api_key_input.text().is_empty(), "api_key Edit 预填必须留空");
        assert_eq!(d.focus, ProviderEditField::Protocol);
    }

    #[test]
    fn edit_mode_name_is_readonly() {
        let mut d = ProviderEditDialog::new();
        d.open_edit(&sample_provider());
        d.focus = ProviderEditField::Name;
        d.handle_key(&Key::Char('x'));
        assert_eq!(d.name_input.text(), "OpenAI");
    }

    #[test]
    fn protocol_left_right_cycles() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        d.focus = ProviderEditField::Protocol;
        d.handle_key(&Key::Right);
        assert_eq!(d.protocol_idx, 1);
        d.handle_key(&Key::Left);
        assert_eq!(d.protocol_idx, 0);
        // 左越界回绕到末项。
        d.handle_key(&Key::Left);
        assert_eq!(d.protocol_idx, PROTOCOL_OPTIONS.len() - 1);
    }

    #[test]
    fn add_requires_name_base_url_and_api_key() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        assert!(d.handle_key(&Key::Enter).is_none(), "空 name 不提交");
        for c in "My Provider".chars() {
            d.handle_key(&Key::Char(c));
        }
        assert!(d.handle_key(&Key::Enter).is_none(), "空 base_url 不提交");
        d.focus = ProviderEditField::BaseUrl;
        for c in "https://x".chars() {
            d.handle_key(&Key::Char(c));
        }
        assert!(d.handle_key(&Key::Enter).is_none(), "Add 空 api_key 不提交");
        assert!(d.is_open());
    }

    #[test]
    fn add_submit_slugifies_name_into_id() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        for c in "My OpenAI".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = ProviderEditField::BaseUrl;
        for c in "https://api.example.com/v1".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = ProviderEditField::ApiKey;
        for c in "sk-secret".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(ProviderEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.mode, ProviderEditMode::Add);
        assert_eq!(s.id, "my-openai");
        assert_eq!(s.name, "My OpenAI");
        assert_eq!(s.base_url, "https://api.example.com/v1");
        assert_eq!(s.protocol, "openai");
        assert_eq!(s.api_key, "sk-secret");
        assert!(!d.is_open());
    }

    #[test]
    fn edit_submit_keeps_origin_id_and_allows_blank_api_key() {
        let mut d = ProviderEditDialog::new();
        d.open_edit(&sample_provider());
        let Some(ProviderEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit (blank api_key = keep current)");
        };
        assert_eq!(s.mode, ProviderEditMode::Edit);
        assert_eq!(s.id, "openai");
        assert_eq!(s.name, "OpenAI");
        assert!(s.api_key.is_empty(), "留空 = 不重置 auth");
    }

    #[test]
    fn esc_returns_cancel_and_clears_api_key() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        d.focus = ProviderEditField::ApiKey;
        for c in "sk-x".chars() {
            d.handle_key(&Key::Char(c));
        }
        let action = d.handle_key(&Key::Escape);
        assert!(matches!(action, Some(ProviderEditAction::Cancel)));
        assert!(!d.is_open());
        assert!(d.api_key_input.text().is_empty(), "close 必须抹除 api_key 明文");
    }
}
