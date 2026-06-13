//! 土 — 全局编排权威 (State Authority)

use revue::prelude::*;
use crate::dialog::DialogStack;

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
    pub toasts: Signal<Vec<ToastMessage>>,
    pub dialog_stack: DialogStack,
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
            dialog_stack: DialogStack::new(),
        }
    }

    pub fn navigate(&self, route: Route) {
        self.route.set(route);
    }

    pub fn navigate_home(&self) {
        self.navigate(Route::Home);
    }

    pub fn request_exit(&self) {
        self.exiting.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_store_has_dialog_stack() {
        let store = AppStore::new();
        assert!(!store.dialog_stack.is_open());
    }

    #[test]
    fn navigate_updates_route() {
        let store = AppStore::new();
        store.navigate(Route::Session { session_id: "ses_1".into() });
        assert!(matches!(
            store.route.get(),
            Route::Session { ref session_id } if session_id == "ses_1"
        ));
    }

    #[test]
    fn request_exit_sets_flag() {
        let store = AppStore::new();
        assert!(!store.exiting.get());
        store.request_exit();
        assert!(store.exiting.get());
    }
}
