//! 土 — Global orchestration authority.
//!
//! AppStore holds cross-session state: routing, available models/agents,
//! session list, and a map of active SessionStores.

use revue::prelude::*;
use revue::style::ThemeVariant;
use crate::store::types::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Session { session_id: String },
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Route::Home => "home",
            Route::Session { .. } => "session",
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
        }
    }

    pub fn navigate(&self, route: Route) { self.route.set(route); }
    pub fn navigate_home(&self) { self.navigate(Route::Home); }
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
}
