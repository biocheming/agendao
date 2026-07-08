//! 木律·土律 — Settings Details pane 的 *in-place 编辑* 状态机。
//!
//! 道纪闭环:
//!   - 木(输入):Add/Edit 的所有字段(name/base_url/protocol/api_key)由 4 个 Input
//!     widget + 1 个 protocol_idx 收口;**唯一字段权威**。
//!   - 火(执行):`handle_key` 返回 `SettingsEditAction`(Submit/Cancel/None),
//!     AppHandler 路由到既有 `submit_provider_edit`(client → server)。
//!   - 土(承载):AppHandler.settings_edit 单点持有编辑态;active=true 时
//!     Providers/Details pane 渲染读它,active=false 时回退正常只读。
//!   - 金(输出):Details pane 在 editing 时 field_block 渲染 Input cursor,
//!     非 editing 时渲染纯 Text — **同一区段同一权威**,不再开 dialog 第二窗口。
//!   - 水(回流):submit 后调 `refresh_providers_into_store` 回灌 store。
//!
//! 配对销毁(道纪·第九条):
//!   - enter_edit / enter_add 配 close;submit / cancel / Tab 切离 Details 三路都走 close
//!   - close 必清 api_key_input(明文不驻留)
//!
//! Add 草稿不入 store.providers:store.providers 是 server 端真相镜像,草稿无 server
//! 落地。Providers pane 渲染时若 is_add() 在列表末尾**虚拟追加** "(new provider)" 行,
//! Details pane 完全从 edit_state 读字段——草稿生命在本结构内部,close 即销毁。

use revue::widget::Input;

/// in-place 编辑两种模式。Add 多 1 个 name 字段(同时充当 id),Edit 不允许改 id/name。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEditMode {
    /// 新建 provider:4 字段(name/base_url/protocol/api_key)。
    Add,
    /// 编辑现有:3 字段(base_url/protocol/api_key)。name/id 不可改(改 id 等于新建+删旧)。
    Edit,
}

/// 字段游标:Add 模式 4 字段循环;Edit 模式 3 字段循环(Name 被跳过)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEditField {
    Name,     // 仅 Add
    BaseUrl,
    Protocol,
    ApiKey,
}

impl SettingsEditField {
    /// 字段循环,跳过不适用于当前 mode 的字段。
    pub fn next(self, mode: SettingsEditMode) -> Self {
        match (mode, self) {
            (SettingsEditMode::Add, SettingsEditField::Name) => SettingsEditField::BaseUrl,
            (SettingsEditMode::Add, SettingsEditField::BaseUrl) => SettingsEditField::Protocol,
            (SettingsEditMode::Add, SettingsEditField::Protocol) => SettingsEditField::ApiKey,
            (SettingsEditMode::Add, SettingsEditField::ApiKey) => SettingsEditField::Name,
            (SettingsEditMode::Edit, SettingsEditField::BaseUrl) => SettingsEditField::Protocol,
            (SettingsEditMode::Edit, SettingsEditField::Protocol) => SettingsEditField::ApiKey,
            (SettingsEditMode::Edit, SettingsEditField::ApiKey) => SettingsEditField::BaseUrl,
            (SettingsEditMode::Edit, SettingsEditField::Name) => SettingsEditField::BaseUrl,
        }
    }

    pub fn prev(self, mode: SettingsEditMode) -> Self {
        // 反向循环 = 正向 3 次(Add 4 字段)/ 2 次(Edit 3 字段);简单清晰避免重复 enum。
        let steps = match mode {
            SettingsEditMode::Add => 3,
            SettingsEditMode::Edit => 2,
        };
        (0..steps).fold(self, |f, _| f.next(mode))
    }
}

/// `handle_key` 返回的动作。AppHandler 拿到 Submit 后从 state 取字段值组装 submission。
pub enum SettingsEditAction {
    /// 当前键已被消费,无后续动作(继续编辑)。
    Consumed,
    /// 用户 Enter:外层从 state 字段值组装 `ProviderEditSubmission` 调 client。
    Submit,
    /// 用户 Esc / Tab 切离 Details:外层调 close()。
    Cancel,
    /// 未识别键:外层可选其他处理。
    Pass,
}

/// 与 server `CONNECT_PROTOCOL_OPTIONS`(provider.rs:576)同源副本。
/// **唯一权威**:dialog/provider_edit.rs 同名 const 在 dialog deprecated 后会消失,
/// 此处变成全局 protocol 列表的 in-process 权威。
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

pub struct SettingsEditState {
    /// editing 是否进行中。Details pane 渲染读它判断 editable / readonly 形态。
    pub active: bool,
    pub mode: SettingsEditMode,
    /// Edit 模式:被编辑的 provider id;Add 模式:空串(直到 submit 时 name_input 充当)。
    pub origin_provider_id: String,
    /// 当前光标所在字段。
    pub focus: SettingsEditField,
    /// 仅 Add 模式有效:provider 显示名 & id(name slugify 后充当 id)。
    pub name_input: Input,
    pub base_url_input: Input,
    /// 当前 protocol 选项在 PROTOCOL_OPTIONS 内的下标。
    pub protocol_idx: usize,
    /// api_key 输入:Input.password(true),buffer 明文,UI 显示 `•`。
    pub api_key_input: Input,
}

impl SettingsEditState {
    pub fn new() -> Self {
        Self {
            active: false,
            mode: SettingsEditMode::Edit,
            origin_provider_id: String::new(),
            focus: SettingsEditField::BaseUrl,
            name_input: Input::new().placeholder("e.g. My OpenAI"),
            base_url_input: Input::new().placeholder("https://api.openai.com/v1"),
            protocol_idx: 0,
            api_key_input: Input::new().password(true).placeholder("sk-..."),
        }
    }

    pub fn is_add(&self) -> bool {
        self.active && matches!(self.mode, SettingsEditMode::Add)
    }

    pub fn is_edit(&self) -> bool {
        self.active && matches!(self.mode, SettingsEditMode::Edit)
    }

    /// 进入 Add 模式:全字段空白,焦点落 Name。
    pub fn enter_add(&mut self) {
        self.active = true;
        self.mode = SettingsEditMode::Add;
        self.origin_provider_id.clear();
        self.focus = SettingsEditField::Name;
        self.name_input = Input::new().placeholder("e.g. my-openai");
        self.base_url_input = Input::new().placeholder("https://api.openai.com/v1");
        self.protocol_idx = 0; // 默认 openai(最常见)
        self.api_key_input = Input::new().password(true).placeholder("sk-...");
    }

    /// 进入 Edit 模式:prefill 当前 provider 字段(api_key 留空 = "不改保留原")。
    pub fn enter_edit(&mut self, info: &agendao_client::ProviderInfo) {
        self.active = true;
        self.mode = SettingsEditMode::Edit;
        self.origin_provider_id = info.id.clone();
        self.focus = SettingsEditField::BaseUrl;
        // Edit 不允许改 name,但保留 buffer 一致(name_input 在 Edit 不渲染)。
        self.name_input = Input::new().value(info.name.clone());
        self.base_url_input = Input::new()
            .placeholder("https://api.openai.com/v1")
            .value(info.base_url.clone().unwrap_or_default());
        self.protocol_idx = info
            .protocol
            .as_deref()
            .and_then(|p| PROTOCOL_OPTIONS.iter().position(|(k, _)| *k == p))
            .unwrap_or(0);
        // api_key Edit prefill 留空 — 与 dialog 方案语义一致:留空 = 不改 server auth,
        // 非空 = 重置 auth.json 条目。永不下发(server `ProviderInfo` 无 api_key 字段)。
        self.api_key_input = Input::new()
            .password(true)
            .placeholder("(leave blank to keep current key)");
    }

    /// 关闭编辑态。**配对销毁**:api_key 明文 buffer 必须清。
    pub fn close(&mut self) {
        self.active = false;
        self.origin_provider_id.clear();
        self.name_input.clear();
        self.base_url_input.clear();
        // 关键:api_key_input.clear() 让明文 buffer 不驻留(道纪·第九条·配对销毁)。
        self.api_key_input.clear();
    }

    /// 当前 protocol key(用于 submit 时填 ProviderEditSubmission.protocol)。
    pub fn protocol_key(&self) -> &'static str {
        PROTOCOL_OPTIONS
            .get(self.protocol_idx)
            .map(|(k, _)| *k)
            .unwrap_or("openai")
    }

    /// 当前 protocol 显示名(field_block 渲染用)。
    pub fn protocol_label(&self) -> &'static str {
        PROTOCOL_OPTIONS
            .get(self.protocol_idx)
            .map(|(_, l)| *l)
            .unwrap_or("OpenAI")
    }

    /// 字段路由:keymap 在 handle_settings_key 编辑态分支调用。
    /// **不接管所有键**——只在 active 时:Tab/Shift-Tab 切字段、Enter Submit、Esc Cancel、
    /// Protocol 字段下 ←/→ 切 protocol_idx、其他字符派发到当前 focus 的 Input。
    /// 返回 Pass 让外层兜底(例如 Ctrl-C 整体退出)。
    pub fn handle_key(&mut self, key: &revue::event::Key) -> SettingsEditAction {
        use revue::event::Key;
        if !self.active {
            return SettingsEditAction::Pass;
        }
        match key {
            Key::Escape => SettingsEditAction::Cancel,
            Key::Enter => SettingsEditAction::Submit,
            Key::Tab => {
                self.focus = self.focus.next(self.mode);
                SettingsEditAction::Consumed
            }
            Key::BackTab => {
                self.focus = self.focus.prev(self.mode);
                SettingsEditAction::Consumed
            }
            // Protocol 字段下 ←/→ 切选项(其他字段下 ←/→ 走 Input.handle_key 移光标)。
            Key::Left | Key::Right if self.focus == SettingsEditField::Protocol => {
                let n = PROTOCOL_OPTIONS.len();
                if n > 0 {
                    self.protocol_idx = match key {
                        Key::Left => (self.protocol_idx + n - 1) % n,
                        _ => (self.protocol_idx + 1) % n,
                    };
                }
                SettingsEditAction::Consumed
            }
            _ => {
                // 派发到当前 focus 的 Input(Protocol 字段不接字符——按上面分支已被吃掉的 ←/→ 之外,
                // 其他键(字符/退格)对 Protocol 无意义,Consumed 让用户感受到"这里不是文字字段")。
                match self.focus {
                    SettingsEditField::Name => {
                        let _ = self.name_input.handle_key(key);
                    }
                    SettingsEditField::BaseUrl => {
                        let _ = self.base_url_input.handle_key(key);
                    }
                    SettingsEditField::Protocol => {
                        // 吞掉,保持 caret 干净——Protocol 不接受文字键。
                    }
                    SettingsEditField::ApiKey => {
                        let _ = self.api_key_input.handle_key(key);
                    }
                }
                SettingsEditAction::Consumed
            }
        }
    }
}

impl Default for SettingsEditState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use revue::event::Key;

    #[test]
    fn add_mode_cycles_four_fields() {
        let mut s = SettingsEditState::new();
        s.enter_add();
        assert_eq!(s.focus, SettingsEditField::Name);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::BaseUrl);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::Protocol);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::ApiKey);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::Name, "Add 4-field cycle returns to Name");
    }

    #[test]
    fn edit_mode_cycles_three_fields_skipping_name() {
        let mut s = SettingsEditState::new();
        let info = agendao_client::ProviderInfo {
            id: "openai".into(),
            name: "OpenAI".into(),
            models: vec![],
            base_url: Some("https://api.openai.com/v1".into()),
            protocol: Some("openai".into()),
        };
        s.enter_edit(&info);
        assert_eq!(s.focus, SettingsEditField::BaseUrl);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::Protocol);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::ApiKey);
        s.handle_key(&Key::Tab);
        assert_eq!(s.focus, SettingsEditField::BaseUrl, "Edit 3-field cycle skips Name");
    }

    #[test]
    fn protocol_field_left_right_cycles_options() {
        let mut s = SettingsEditState::new();
        s.enter_add();
        s.focus = SettingsEditField::Protocol;
        let initial = s.protocol_idx;
        s.handle_key(&Key::Right);
        assert_eq!(s.protocol_idx, (initial + 1) % PROTOCOL_OPTIONS.len());
        s.handle_key(&Key::Left);
        assert_eq!(s.protocol_idx, initial, "Left undoes Right");
    }

    #[test]
    fn close_clears_api_key_buffer() {
        let mut s = SettingsEditState::new();
        s.enter_add();
        s.api_key_input.set_value("sk-secret");
        assert_eq!(s.api_key_input.text(), "sk-secret");
        s.close();
        assert!(s.api_key_input.text().is_empty(), "api_key 明文必须在 close 时抹除");
    }

    #[test]
    fn esc_returns_cancel() {
        let mut s = SettingsEditState::new();
        s.enter_add();
        let action = s.handle_key(&Key::Escape);
        assert!(matches!(action, SettingsEditAction::Cancel));
    }

    #[test]
    fn enter_returns_submit() {
        let mut s = SettingsEditState::new();
        s.enter_add();
        let action = s.handle_key(&Key::Enter);
        assert!(matches!(action, SettingsEditAction::Submit));
    }

    #[test]
    fn enter_edit_prefills_protocol_idx_from_provider_protocol() {
        let mut s = SettingsEditState::new();
        let info = agendao_client::ProviderInfo {
            id: "anth".into(),
            name: "Anthropic".into(),
            models: vec![],
            base_url: None,
            protocol: Some("anthropic".into()),
        };
        s.enter_edit(&info);
        assert_eq!(s.protocol_key(), "anthropic");
    }
}
