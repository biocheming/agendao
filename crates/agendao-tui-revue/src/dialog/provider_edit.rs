//! 金 — Provider Add/Edit Dialog.(**deprecated:Part 7 起被 in-place 编辑取代**)
//!
//! **deprecated** — `app/settings_edit_state.rs` 的 `SettingsEditState` 把
//! Provider 字段编辑搬进了 Settings Details pane(同一区段既是只读 view 又是
//! 编辑 form),实现金律·唯一成形权威。本 dialog 不再有 keymap 触发面
//! (`a`/`e` 已改为 `self.settings_edit.enter_add/edit`),保留文件仅作
//! 历史参考。后续清理:连同 `app::Panel::ProviderEdit`、
//! `AppHandler.provider_edit_dialog` 字段、panel_dispatch 的 dispatch 分支
//! 一并移除。
//!
//! 历史设计(下文保留作为参考)————————————————————————————————————————————
//!
//! 四字段表单(name / base_url / protocol / api_key)弹居中 dialog。
//! 道纪:
//! - **木律**:Settings 内 a/e 唯一入口(keymap 触发 open_add/open_edit);
//! - **金律**:api_key 字段用 `Input.password(true)` 渲染 `•`,api_key 明文
//!   只在 submit 瞬间从 `Input.text()` 取出交给 client → server,
//!   dialog close 自动 `clear()`(土律·第九条 lifecycle 对称,不驻留);
//! - **第十条·可观测性**:protocol selector 来源即 server `/provider/connect/schema`
//!   下发或硬编码 fallback,选项与 server `CONNECT_PROTOCOL_OPTIONS` 同源。
//!
//! 上游(Part 4)调:
//! - `dialog.handle_key(&key)` → 返回 `Some(Action::Submit(...))` 时,AppHandler
//!   读 `ProviderEditSubmission` 调 client 写入并 refresh。
//! - close 在 submit / Cancel / Esc 三路对称(道纪·第九条)。
#![allow(dead_code)]

use revue::event::Key;
use revue::prelude::*;
use revue::widget::Border;

use crate::dialog::backdrop;
use crate::theme::colors;

/// Add(新建)或 Edit(改既有)。两路 submit 字段相同,
/// 路由侧据 mode 决定 register_custom_provider vs update_provider+connect_provider。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderEditMode {
    Add,
    Edit,
}

/// dialog 当前聚焦字段;Tab/Shift-Tab 循环切换。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderEditField {
    Name,
    BaseUrl,
    Protocol,
    ApiKey,
}

impl ProviderEditField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::BaseUrl,
            Self::BaseUrl => Self::Protocol,
            Self::Protocol => Self::ApiKey,
            Self::ApiKey => Self::Name,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Name => Self::ApiKey,
            Self::BaseUrl => Self::Name,
            Self::Protocol => Self::BaseUrl,
            Self::ApiKey => Self::Protocol,
        }
    }
}

/// dialog → AppHandler 单向事件(handle_key 返回值)。
pub enum ProviderEditAction {
    Submit(ProviderEditSubmission),
    Cancel,
}

/// 提交载荷:AppHandler 据 mode 调 client 写入。
///
/// `api_key` 为空 = "保留原 key"(Edit 模式下不发 connect_provider);
/// Add 模式必须非空(register_custom_provider 强制要 api_key)。
pub struct ProviderEditSubmission {
    pub mode: ProviderEditMode,
    /// Edit 模式 = 原 provider.id(server endpoint key);
    /// Add 模式 = 用户填的 name 做 slug(由 AppHandler 转 lowercase + 去空格)。
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    pub api_key: String,
}

/// Provider 添加 / 编辑 dialog。
pub struct ProviderEditDialog {
    pub visible: bool,
    pub mode: ProviderEditMode,
    /// 原 provider.id(Edit);Add 模式留空字符串,submit 时由 AppHandler 据 name 派生。
    pub origin_id: String,
    name_input: revue::widget::Input,
    base_url_input: revue::widget::Input,
    api_key_input: revue::widget::Input,
    /// protocol 选项(static):与 server `CONNECT_PROTOCOL_OPTIONS` 同源。
    /// 用 `&'static str` 引用静态表项的第一字段(`id`),展示用第二字段。
    protocol_options: &'static [(&'static str, &'static str)],
    protocol_idx: usize,
    focus: ProviderEditField,
}

/// Server `CONNECT_PROTOCOL_OPTIONS` 同源副本(provider.rs:576)。
/// 改新 protocol 时两边同步;TUI 不联网拉 schema 用这份兜底
/// (土律·第四条单点权威——动态拉 schema 是优化,本表是保底真相)。
pub const PROTOCOL_OPTIONS: &[(&str, &str)] = &[
    ("openai", "OpenAI"),
    ("openrouter", "OpenRouter"),
    ("perplexity", "Perplexity"),
    ("anthropic", "Anthropic"),
    ("google", "Google"),
    ("bedrock", "Amazon Bedrock"),
    ("vertex", "Google Vertex"),
    ("github-copilot", "GitHub Copilot"),
    ("gitlab", "GitLab"),
];

impl ProviderEditDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: ProviderEditMode::Add,
            origin_id: String::new(),
            name_input: revue::widget::Input::new().placeholder("e.g. My OpenAI"),
            base_url_input: revue::widget::Input::new()
                .placeholder("https://api.openai.com/v1"),
            api_key_input: revue::widget::Input::new()
                .placeholder("sk-...")
                .password(true),
            protocol_options: PROTOCOL_OPTIONS,
            protocol_idx: 0,
            focus: ProviderEditField::Name,
        }
    }

    /// 打开 Add 模式(空表单),focus Name。
    pub fn open_add(&mut self) {
        self.mode = ProviderEditMode::Add;
        self.origin_id.clear();
        self.name_input =
            revue::widget::Input::new().placeholder("e.g. My OpenAI");
        self.base_url_input = revue::widget::Input::new()
            .placeholder("https://api.openai.com/v1");
        self.api_key_input = revue::widget::Input::new()
            .placeholder("sk-...")
            .password(true);
        self.protocol_idx = 0;
        self.focus = ProviderEditField::Name;
        self.visible = true;
    }

    /// 打开 Edit 模式,prefill name/base_url/protocol;api_key 留空(留空=不改)。
    pub fn open_edit(&mut self, info: &agendao_client::ProviderInfo) {
        self.mode = ProviderEditMode::Edit;
        self.origin_id = info.id.clone();
        self.name_input = revue::widget::Input::new()
            .placeholder("Display name")
            .value(info.name.clone());
        self.base_url_input = revue::widget::Input::new()
            .placeholder("https://api.openai.com/v1")
            .value(info.base_url.clone().unwrap_or_default());
        self.api_key_input = revue::widget::Input::new()
            .placeholder("Leave empty to keep current")
            .password(true);
        self.protocol_idx = info
            .protocol
            .as_deref()
            .and_then(|p| {
                self.protocol_options.iter().position(|(id, _)| *id == p)
            })
            .unwrap_or(0);
        self.focus = ProviderEditField::Name;
        self.visible = true;
    }

    /// 关闭并清空(含 api_key Input.clear() 避免明文驻留;道纪·第九条)。
    pub fn close(&mut self) {
        self.visible = false;
        self.name_input.clear();
        self.base_url_input.clear();
        self.api_key_input.clear();
        self.origin_id.clear();
    }

    pub fn is_open(&self) -> bool {
        self.visible
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
                // 校验必填:name/base_url/protocol(Add 时 api_key 也必填)。
                let name = self.name_input.text().trim().to_string();
                let base_url = self.base_url_input.text().trim().to_string();
                let api_key = self.api_key_input.text().to_string();
                let protocol = self
                    .protocol_options
                    .get(self.protocol_idx)
                    .map(|(id, _)| id.to_string())
                    .unwrap_or_default();
                if name.is_empty() || base_url.is_empty() || protocol.is_empty() {
                    // 缺字段时 Enter 不提交,只静默(校验/红框由 AppHandler toast 处理);
                    // 这里 dialog 保持 open 让用户继续编辑。
                    return None;
                }
                let id = if self.mode == ProviderEditMode::Edit {
                    self.origin_id.clone()
                } else {
                    // Add 模式 slug:lowercase + 空格→-。AppHandler 可再覆盖。
                    name.to_lowercase()
                        .chars()
                        .map(|c| if c.is_whitespace() { '-' } else { c })
                        .collect()
                };
                let submission = ProviderEditSubmission {
                    mode: self.mode,
                    id,
                    name,
                    base_url,
                    protocol,
                    api_key,
                };
                self.close();
                Some(ProviderEditAction::Submit(submission))
            }
            Key::Tab => {
                self.focus = self.focus.next();
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev();
                None
            }
            Key::Left if self.focus == ProviderEditField::Protocol => {
                if self.protocol_idx == 0 {
                    self.protocol_idx = self.protocol_options.len() - 1;
                } else {
                    self.protocol_idx -= 1;
                }
                None
            }
            Key::Right if self.focus == ProviderEditField::Protocol => {
                self.protocol_idx =
                    (self.protocol_idx + 1) % self.protocol_options.len();
                None
            }
            _ => {
                // 字符/方向键转发给当前 focus 字段的 Input(返回 bool,这里弃用)。
                match self.focus {
                    ProviderEditField::Name => {
                        let _ = self.name_input.handle_key(key);
                    }
                    ProviderEditField::BaseUrl => {
                        let _ = self.base_url_input.handle_key(key);
                    }
                    ProviderEditField::Protocol => {} // Protocol 不接字符
                    ProviderEditField::ApiKey => {
                        let _ = self.api_key_input.handle_key(key);
                    }
                }
                None
            }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible {
            return;
        }
        let title = match self.mode {
            ProviderEditMode::Add => " Add Provider ",
            ProviderEditMode::Edit => " Edit Provider ",
        };

        // 每字段:label 1 行(focused 时 ACCENT_AMBER,否则 FG_SECONDARY)+
        // bordered Input 3 行(╭/│input│/╰)。Protocol 行用横向选择器形态。
        let name_block = field_input(
            "Name",
            self.name_input.clone(),
            self.focus == ProviderEditField::Name,
        );
        let base_block = field_input(
            "Base URL",
            self.base_url_input.clone(),
            self.focus == ProviderEditField::BaseUrl,
        );
        let proto_block = protocol_selector(
            self.protocol_options,
            self.protocol_idx,
            self.focus == ProviderEditField::Protocol,
        );
        let api_label = match self.mode {
            ProviderEditMode::Add => "API Key",
            ProviderEditMode::Edit => "API Key (leave empty = keep current)",
        };
        let api_block = field_input(
            api_label,
            self.api_key_input.clone(),
            self.focus == ProviderEditField::ApiKey,
        );

        let content = vstack()
            .gap(0)
            .child_sized(name_block, 4)
            .child_sized(base_block, 4)
            .child_sized(proto_block, 4)
            .child_sized(api_block, 4);

        backdrop::render_dialog(
            title,
            colors::ACCENT_CYAN,
            content,
            "Tab: next field   ←/→: Protocol   Enter: save   Esc: cancel",
            ctx,
            72, // max_w
            22, // max_h(4 字段块 × 4 + footer 2 + padding ≈ 22)
        );
    }
}

impl Default for ProviderEditDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// 单字段块:label 1 行 + bordered Input 3 行(╭│╰),总高 4。
/// `focused` 时 label 用 ACCENT_AMBER 提示当前 caret 在此,Input 自身画 cursor。
fn field_input(
    label: &str,
    mut input: revue::widget::Input,
    focused: bool,
) -> revue::widget::Stack {
    let label_color = if focused {
        colors::E_AMBER
    } else {
        colors::FG_SECONDARY
    };
    let border_color = if focused {
        colors::E_AMBER
    } else {
        colors::BORDER
    };
    input = input.focused(focused);
    vstack()
        .gap(0)
        .child_sized(
            Text::new(format!(" {}", label)).fg(label_color),
            1,
        )
        .child_sized(Border::rounded().fg(border_color).child(input), 3)
}

/// Protocol 横向选择器:label 1 行 + 框内 `< openai >` 形态(focused 时 amber)。
fn protocol_selector(
    options: &[(&str, &str)],
    idx: usize,
    focused: bool,
) -> revue::widget::Stack {
    let label_color = if focused {
        colors::E_AMBER
    } else {
        colors::FG_SECONDARY
    };
    let border_color = if focused {
        colors::E_AMBER
    } else {
        colors::BORDER
    };
    let (id, label) = options.get(idx).copied().unwrap_or(("openai", "OpenAI"));
    let display = format!(" ‹ {} ({}) ›  [{}/{}]", label, id, idx + 1, options.len());
    let body = hstack()
        .gap(0)
        .child_sized(Text::new(" ").fg(colors::FG_PRIMARY), 1)
        .child_flex(
            Text::new(display).fg(if focused {
                colors::E_AMBER
            } else {
                colors::FG_PRIMARY
            }),
            1.0,
        );
    vstack()
        .gap(0)
        .child_sized(Text::new(" Protocol").fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(body), 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_add_sets_state() {
        let mut d = ProviderEditDialog::new();
        assert!(!d.is_open());
        d.open_add();
        assert!(d.is_open());
        assert_eq!(d.mode, ProviderEditMode::Add);
        assert!(d.origin_id.is_empty());
        assert_eq!(d.focus, ProviderEditField::Name);
    }

    #[test]
    fn open_edit_prefills_from_info() {
        let info = agendao_client::ProviderInfo {
            id: "openai".into(),
            name: "OpenAI".into(),
            models: vec![],
            base_url: Some("https://api.openai.com/v1".into()),
            protocol: Some("anthropic".into()),
        };
        let mut d = ProviderEditDialog::new();
        d.open_edit(&info);
        assert!(d.is_open());
        assert_eq!(d.mode, ProviderEditMode::Edit);
        assert_eq!(d.origin_id, "openai");
        assert_eq!(d.name_input.text(), "OpenAI");
        assert_eq!(d.base_url_input.text(), "https://api.openai.com/v1");
        // protocol "anthropic" 在 PROTOCOL_OPTIONS 第 4 项(idx 3)。
        assert_eq!(d.protocol_idx, 3);
    }

    #[test]
    fn enter_with_empty_required_does_not_submit() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        // 全空 enter → 静默,dialog 仍 open。
        assert!(matches!(d.handle_key(&Key::Enter), None));
        assert!(d.is_open());
    }

    #[test]
    fn esc_returns_cancel_and_closes() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        let action = d.handle_key(&Key::Escape);
        assert!(matches!(action, Some(ProviderEditAction::Cancel)));
        assert!(!d.is_open());
    }

    #[test]
    fn tab_cycles_focus_forward() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        assert_eq!(d.focus, ProviderEditField::Name);
        d.handle_key(&Key::Tab);
        assert_eq!(d.focus, ProviderEditField::BaseUrl);
        d.handle_key(&Key::Tab);
        assert_eq!(d.focus, ProviderEditField::Protocol);
        d.handle_key(&Key::Tab);
        assert_eq!(d.focus, ProviderEditField::ApiKey);
        d.handle_key(&Key::Tab);
        assert_eq!(d.focus, ProviderEditField::Name); // wraps
    }

    #[test]
    fn protocol_left_right_cycles() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        d.focus = ProviderEditField::Protocol;
        let start = d.protocol_idx;
        d.handle_key(&Key::Right);
        assert_eq!(d.protocol_idx, (start + 1) % PROTOCOL_OPTIONS.len());
        d.handle_key(&Key::Left);
        assert_eq!(d.protocol_idx, start);
    }

    #[test]
    fn full_submit_returns_submission_and_closes() {
        let mut d = ProviderEditDialog::new();
        d.open_add();
        // 用 handle_key 填字段(经过 focus 路径)。
        for c in "MyProv".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → BaseUrl
        for c in "https://x".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → Protocol(默认 idx=0 openai)
        d.handle_key(&Key::Tab); // → ApiKey
        for c in "sk-xyz".chars() {
            d.handle_key(&Key::Char(c));
        }
        let action = d.handle_key(&Key::Enter);
        let Some(ProviderEditAction::Submit(s)) = action else {
            panic!("expected Submit");
        };
        assert_eq!(s.mode, ProviderEditMode::Add);
        assert_eq!(s.name, "MyProv");
        assert_eq!(s.base_url, "https://x");
        assert_eq!(s.protocol, "openai");
        assert_eq!(s.api_key, "sk-xyz");
        assert_eq!(s.id, "myprov"); // slug
        assert!(!d.is_open()); // 道纪·第九条:submit 后 close
    }
}
