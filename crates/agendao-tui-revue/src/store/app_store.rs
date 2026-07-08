//! 土 — Global orchestration authority.
//!
//! AppStore holds cross-session state: routing, available models/agents,
//! session list, and a map of active SessionStores.

use std::collections::HashSet;

use revue::prelude::*;
use revue::style::ThemeVariant;
use crate::store::types::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Session { session_id: String },
    /// Settings 全屏页面(三栏:分类 | Providers | Details)。
    /// 进入由 `UiActionId::OpenSettings`(⚙ click / `/settings`)唯一触发;
    /// Esc → `navigate_home()` 收口。
    Settings,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Route::Home => "home",
            Route::Session { .. } => "session",
            Route::Settings => "settings",
        }
    }
}

#[derive(Clone)]
pub struct AppStore {
    pub route: Signal<Route>,
    pub exiting: Signal<bool>,
    pub working_dir: Signal<String>,

    // 土：可用模型/Agent（ModelSelect/AgentSelect dialog 消费）
    pub available_models: Signal<Vec<ModelInfo>>,
    pub available_agents: Signal<Vec<AgentInfo>>,
    pub selected_model: Signal<Option<String>>,
    pub selected_agent: Signal<Option<String>>,
    pub selected_mode: Signal<Option<String>>,

    // 土：可用会话列表（SessionList dialog 消费）
    pub session_list: Signal<Vec<SessionListItem>>,

    // 土：Toast 队列（ToastLayer 消费）
    pub toasts: Signal<Vec<ToastMsg>>,

    // 金：Session header dir 全路径 tooltip（点击触发，render overlay 消费）
    pub dir_tooltip: Signal<Option<DirTooltip>>,

    // 木→金：UI 偏好 toggle（单一所有权，默认值对齐当前硬编码行为；
    // 渲染端读 signal 决定是否画 header/tips/thinking/scrollbar + 密度）。
    // 注意：timestamps 未在此列——TUI 当前根本不渲染 timestamp（TranscriptBlock
    // 各变体无 timestamp 字段），加一个无消费端的 signal 即「有阴无阳」伪权威
    // （道纪第十条）。待 timestamp 渲染落地后再加。
    pub show_thinking: Signal<bool>,
    pub show_scrollbar: Signal<bool>,
    pub show_header: Signal<bool>,
    pub show_tips: Signal<bool>,
    /// true=紧凑间距（块间 0 行间隔）；false=舒适（当前默认，块间 1 行）。
    pub compact_density: Signal<bool>,
    /// 当前主题 variant（阴面记账）。启动时由 OSC11 检测初值；
    /// ToggleAppearance 经 `ds::theme::toggle_variant` 翻转 + `set_theme` 同步渲染。
    pub theme_variant: Signal<ThemeVariant>,

    // 土：Settings 页面状态(`OpenSettings` 进入时 navigate + load 写入;
    // SettingsScreen 唯一只读消费)。
    /// 完整 provider 列表(含 base_url + models),来自 server `/provider` 端点。
    /// 阴面记账(土律):写一次,读多次;Settings 关闭后**不清空**,下次再开秒显。
    pub providers: Signal<Vec<agendao_client::ProviderInfo>>,
    /// 已连接 provider id 集合(server `/provider` 响应的 `connected` 字段)。
    /// 用于 Providers 栏 ● connected dot 和 Details 栏 "Enabled" pill。
    pub providers_connected: Signal<HashSet<String>>,
    /// 当前 Details 栏展示哪个 provider;`None` = providers 为空。
    pub settings_selected_provider: Signal<Option<String>>,
    /// Details 栏内当前选中 model_key;`None` = 当前 provider 无 models 或未进入 Details 焦点。
    /// 由 `handle_settings_key` 在 Details focused 时 ↑/↓ 切换,m/e/d 操作以此为目标。
    pub settings_selected_model: Signal<Option<String>>,
    /// 左栏选中分类;`Model Settings` 是当前唯一有实现的项。
    pub settings_category: Signal<SettingsCategory>,
    /// Tab 切换当前焦点栏;影响 ↑/↓ 行为(选 category / 选 provider / 滚 Details)。
    pub settings_focus_pane: Signal<SettingsFocusPane>,
}

impl AppStore {
    pub fn new() -> Self {
        Self {
            route: signal(Route::Home),
            exiting: signal(false),
            working_dir: signal(
                std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            available_models: signal(Vec::new()),
            available_agents: signal(Vec::new()),
            selected_model: signal(None),
            selected_agent: signal(None),
            selected_mode: signal(None),
            session_list: signal(Vec::new()),
            toasts: signal(Vec::new()),
            dir_tooltip: signal(None),
            show_thinking: signal(true),
            show_scrollbar: signal(true),
            show_header: signal(true),
            show_tips: signal(true),
            compact_density: signal(false),
            theme_variant: signal(ThemeVariant::Dark),
            providers: signal(Vec::new()),
            providers_connected: signal(HashSet::new()),
            settings_selected_provider: signal(None),
            settings_selected_model: signal(None),
            settings_category: signal(SettingsCategory::ModelSettings),
            settings_focus_pane: signal(SettingsFocusPane::Providers),
        }
    }

    pub fn navigate(&self, route: Route) { self.route.set(route); }
    pub fn navigate_home(&self) { self.navigate(Route::Home); }
    pub fn navigate_settings(&self) { self.navigate(Route::Settings); }
    pub fn request_exit(&self) { self.exiting.set(true); }

    pub fn push_toast(&self, text: &str, variant: ToastMsgVariant) {
        // Auto-expire toasts after 4 seconds of wall clock so the prompt
        // area doesn't stay obscured by a "Switched to model" banner
        // forever. The render loop checks `expires_at <= now()` and
        // skips expired entries; a separate housekeeping pass garbage-
        // collects them so the Vec doesn't grow unbounded.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let expires_at = now_ms.saturating_add(4_000);
        self.toasts.update(|t| t.push(ToastMsg { text: text.into(), variant, expires_at }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_store_defaults() {
        let s = AppStore::new();
        assert_eq!(s.route.get(), Route::Home);
        assert!(!s.exiting.get());
        assert!(s.available_models.get().is_empty());
    }

    #[test]
    fn navigate_and_exit() {
        let s = AppStore::new();
        s.navigate(Route::Session { session_id: "s1".into() });
        assert!(matches!(s.route.get(), Route::Session { .. }));
        s.request_exit();
        assert!(s.exiting.get());
    }

    #[test]
    fn push_toast() {
        let s = AppStore::new();
        s.push_toast("done", ToastMsgVariant::Success);
        assert_eq!(s.toasts.get().len(), 1);
    }

    /// Settings 路由进出 + 默认 signals 初值符合"未拉取 providers"基线。
    #[test]
    fn settings_route_and_defaults() {
        let s = AppStore::new();
        s.navigate_settings();
        assert_eq!(s.route.get(), Route::Settings);
        assert_eq!(s.route.get().as_str(), "settings");
        assert!(s.providers.get().is_empty());
        assert!(s.providers_connected.get().is_empty());
        assert!(s.settings_selected_provider.get().is_none());
        assert_eq!(s.settings_category.get(), SettingsCategory::ModelSettings);
        assert_eq!(s.settings_focus_pane.get(), SettingsFocusPane::Providers);
        s.navigate_home();
        assert_eq!(s.route.get(), Route::Home);
    }

    /// SettingsFocusPane::next 循环正确(Categories→Providers→Details→Categories)。
    #[test]
    fn settings_focus_pane_cycle() {
        let p = SettingsFocusPane::Categories;
        let p = p.next();
        assert_eq!(p, SettingsFocusPane::Providers);
        let p = p.next();
        assert_eq!(p, SettingsFocusPane::Details);
        let p = p.next();
        assert_eq!(p, SettingsFocusPane::Categories);
    }

    /// 只有 ModelSettings 标 implemented;其余五项灰显占位(土律·第十条)。
    #[test]
    fn settings_category_implementation_flags() {
        assert!(SettingsCategory::ModelSettings.is_implemented());
        assert!(!SettingsCategory::General.is_implemented());
        assert!(!SettingsCategory::PromptLibrary.is_implemented());
        assert!(!SettingsCategory::KnowledgeBase.is_implemented());
        assert!(!SettingsCategory::Keybindings.is_implemented());
        assert!(!SettingsCategory::About.is_implemented());
    }
}
