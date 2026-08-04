//! 土 — Global orchestration authority.
//!
//! AppStore holds cross-session state: routing, model/agent/mode selection,
//! session list, UI toggles, and Settings page state.

use std::collections::HashSet;

use revue::prelude::*;
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

    // 土：当前选中 model/agent/mode（dispatch 发 prompt 时带上；
    // 可选项列表的活真相在各 Select dialog 内部，不在此处重复持有）
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
    /// 当前主题（阴面记账）。启动时由持久化 config.theme / OSC11 兜底决定初值；
    /// 切换经 `ds::theme::apply_theme` 单点收口（色板 + revue 信号同步）。
    pub theme_id: Signal<crate::ds::theme::ThemeId>,

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
    /// 正在测连接的 provider id（U6 异步化）：`Some` = 后台探测进行中——
    /// Providers 栏行内显示 ◌ pending 标记，且重复触发被防抖吞掉。
    pub settings_testing_provider: Signal<Option<String>>,
    /// open_session 后台拉取进行中（U6③）：transcript 末尾渲染
    /// "⏳ Loading session..." 内联块；回执 drain 清零。
    pub session_loading: Signal<bool>,
    /// Details 栏内当前选中 model_key;`None` = 当前 provider 无 models 或未进入 Details 焦点。
    /// 由 `handle_settings_key` 在 Details focused 时 ↑/↓ 切换,m/e/d 操作以此为目标。
    pub settings_selected_model: Signal<Option<String>>,
    /// 左栏选中分类;已落地 General / ModelSettings / About。
    pub settings_category: Signal<SettingsCategory>,
    /// Tab 切换当前焦点栏;影响 ↑/↓ 行为(选 category / 选 provider / 滚 Details)。
    pub settings_focus_pane: Signal<SettingsFocusPane>,
    /// General 分类 body 内当前选中行下标(`GeneralRow::ALL` 索引)。
    /// keymap 写(↑/↓ 移动、Enter/Space 触发对应 toggle),screen 读(高亮当前行)。
    /// 与 toggle 值本身无关——值真相在各 `show_*`/`theme_id` signal(单点权威)。
    pub settings_general_selected: Signal<usize>,
    /// Keybindings 分类 body 的滚动偏移(首个可见 entry 下标)。
    /// keymap 写(↑/↓/PgUp/PgDn),screen 读(视窗起点)。只读参考,无选中态。
    pub settings_keybindings_scroll: Signal<usize>,
    /// Settings→MCP 分类:server 状态列表(来自 `/mcp`,单点权威)。
    pub settings_mcp: Signal<Vec<SettingsMcpRow>>,
    /// MCP 列表当前选中下标。
    pub settings_mcp_selected: Signal<usize>,
    /// Settings→Skills 分类:catalog + pending proposals 合并列表。
    pub settings_skills: Signal<Vec<SettingsSkillRow>>,
    /// Skills 列表当前选中下标（`flatten_settings_skill_rows` 展开后的可见行下标，
    /// 含类目头行——渲染/键盘/鼠标三方同源）。
    pub settings_skills_selected: Signal<usize>,
    /// Skills 树状分组的折叠类目集合（key = 小写类目名，与 flatten 匹配口径一致；
    /// 空集 = 全部展开）。与 session tree 折叠同范式：折叠态独立持有，不改源数据。
    pub settings_skills_collapsed: Signal<HashSet<String>>,
    /// Settings→Tools 分类：全量 tool 列表（含 disabled/protected 打标）。
    pub settings_tools: Signal<Vec<SettingsToolRow>>,
    /// Tools 列表当前选中下标（`flatten_settings_tool_rows` 展开后的可见行下标）。
    pub settings_tools_selected: Signal<usize>,
    /// Tools 树状分组的折叠类目集合（key = 小写 family 名，口径同 skills）。
    pub settings_tools_collapsed: Signal<HashSet<String>>,
    /// Settings→Plugins 分类：已安装插件列表（managed + discovered，打标）。
    pub settings_plugins: Signal<Vec<SettingsPluginRow>>,
    /// Plugins 列表当前选中下标。
    pub settings_plugins_selected: Signal<usize>,
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
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
            theme_id: signal(crate::ds::theme::ThemeId::TokyoNight),
            providers: signal(Vec::new()),
            providers_connected: signal(HashSet::new()),
            settings_selected_provider: signal(None),
            settings_testing_provider: signal(None),
            session_loading: signal(false),
            settings_selected_model: signal(None),
            settings_category: signal(SettingsCategory::General),
            settings_focus_pane: signal(SettingsFocusPane::Providers),
            settings_general_selected: signal(0),
            settings_keybindings_scroll: signal(0),
            settings_mcp: signal(Vec::new()),
            settings_mcp_selected: signal(0),
            settings_skills: signal(Vec::new()),
            settings_skills_selected: signal(0),
            settings_skills_collapsed: signal(HashSet::new()),
            settings_tools: signal(Vec::new()),
            settings_tools_selected: signal(0),
            settings_tools_collapsed: signal(HashSet::new()),
            settings_plugins: signal(Vec::new()),
            settings_plugins_selected: signal(0),
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
        assert!(s.session_list.get().is_empty());
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
        assert_eq!(s.settings_category.get(), SettingsCategory::General);
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

    /// 七项分类均已落地(General / ModelSettings / Skills / Tools / MCP / Keybindings / About)。
    #[test]
    fn settings_category_implementation_flags() {
        for cat in SettingsCategory::ALL {
            assert!(cat.is_implemented(), "{:?} should be implemented", cat);
        }
        assert_eq!(SettingsCategory::ALL.len(), 8);
        assert_eq!(SettingsCategory::Skills.label(), "Skills");
        assert_eq!(SettingsCategory::Tools.label(), "Tools");
        assert_eq!(SettingsCategory::Plugins.label(), "Plugins");
        assert_eq!(SettingsCategory::McpServers.label(), "MCP Servers");
    }
}
