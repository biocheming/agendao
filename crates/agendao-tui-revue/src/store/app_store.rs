//! 土 — 全局编排权威 (State Authority)
//!
//! AppStore holds the single Signal truth for all application-level state.
//! Every active semantic domain has exactly one Signal holder.

use revue::prelude::*;

/// Application route
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

/// Global application state
#[derive(Clone)]
pub struct AppStore {
    /// Current route
    pub route: Signal<Route>,
    /// Whether the app is exiting
    pub exiting: Signal<bool>,
    /// Toast message queue
    pub toasts: Signal<Vec<ToastMessage>>,
}

#[derive(Clone)]
pub struct ToastMessage {
    pub text: String,
    pub variant: ToastVariant,
}

#[derive(Clone)]
pub enum ToastVariant {
    Success,
    Error,
    Info,
}

impl AppStore {
    pub fn new() -> Self {
        Self {
            route: signal(Route::Home),
            exiting: signal(false),
            toasts: signal(Vec::new()),
        }
    }

    /// Navigate to a specific route
    pub fn navigate(&self, route: Route) {
        self.route.set(route);
    }

    /// Navigate to home
    pub fn navigate_home(&self) {
        self.navigate(Route::Home);
    }

    /// Request application exit
    pub fn request_exit(&self) {
        self.exiting.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_store_initializes_with_home_route() {
        let store = AppStore::new();
        assert_eq!(store.route.get(), Route::Home);
        assert!(!store.exiting.get());
    }

    #[test]
    fn navigate_updates_route() {
        let store = AppStore::new();
        store.navigate(Route::Session {
            session_id: "ses_1".into(),
        });
        assert!(matches!(
            store.route.get(),
            Route::Session { ref session_id } if session_id == "ses_1"
        ));
    }

    #[test]
    fn request_exit_sets_flag() {
        let store = AppStore::new();
        store.request_exit();
        assert!(store.exiting.get());
    }
}
