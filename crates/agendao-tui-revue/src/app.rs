//! Application entry point and event loop.

use anyhow::Context;
use revue::prelude::*;
use revue::event::Key;

use crate::bridge::api::ApiBridge;
use crate::dialog::{AlertDialog, DialogKind, HelpDialog};
use crate::store::app_store::{AppStore, Route};
use crate::screen::{HomeScreen, SessionScreen};

pub fn run_app() -> anyhow::Result<()> {
    let store = AppStore::new();
    let api = ApiBridge::new("http://127.0.0.1:3000").ok();

    let mut app = App::builder().mouse_capture(true).build();
    let view = RootView::new(store.clone(), api);
    app.run_with_handler(view, move |key_event, view| {
        view.handle_key(&key_event.key)
    })
    .context("agendao TUI runtime exited with error")
}

struct RootView {
    store: AppStore,
    api: Option<ApiBridge>,
    alert: AlertDialog,
    help: HelpDialog,
}

impl RootView {
    fn new(store: AppStore, api: Option<ApiBridge>) -> Self {
        Self {
            store,
            api,
            alert: AlertDialog::new(),
            help: HelpDialog::new(),
        }
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        // ── Dialog layer takes priority ──
        if self.store.dialog_stack.is_open() {
            return self.handle_dialog_key(key);
        }

        match key {
            Key::Char('q') | Key::Escape => { self.store.request_exit(); true }
            Key::Char('h') => { self.store.navigate(Route::Home); true }
            _ => {
                // Route to SessionScreen for session-specific keys
                if let Route::Session { .. } = self.store.route.get() {
                    return false; // session keys handled elsewhere
                }
                false
            }
        }
    }

    fn handle_dialog_key(&mut self, key: &Key) -> bool {
        let top = self.store.dialog_stack.top();
        match top {
            Some(DialogKind::Alert) => self.alert.handle_key(key),
            Some(DialogKind::Help) => self.help.handle_key(key),
            _ => {
                // Close any unrecognized dialog on Esc
                if matches!(key, Key::Escape) {
                    self.store.dialog_stack.pop();
                }
                true
            }
        }
    }

    /// Show an alert dialog.
    #[allow(dead_code)]
    pub fn show_alert(&mut self, title: &str, message: &str) {
        self.alert.show(title, message);
        self.store.dialog_stack.push(DialogKind::Alert);
    }

    /// Toggle help dialog.
    #[allow(dead_code)]
    pub fn toggle_help(&mut self) {
        if self.help.visible {
            self.help.dismiss();
            self.store.dialog_stack.close(&DialogKind::Help);
        } else {
            self.help.toggle();
            self.store.dialog_stack.push(DialogKind::Help);
        }
    }

    fn render_status_bar(&self, ctx: &mut RenderContext) {
        let route_label = self.store.route.get().as_str();
        let dialogs = if self.store.dialog_stack.is_open() {
            format!(" [dialog: {:?}]", self.store.dialog_stack.top().unwrap())
        } else {
            String::new()
        };
        let text = format!(" agendao | [{}]{} | q: quit | h: home | ?: help ", route_label, dialogs);
        let bar_y = ctx.area.height.saturating_sub(1);
        for x in 0..ctx.area.width {
            ctx.draw_text(x, bar_y, " ", Color::rgb(30, 32, 44));
        }
        ctx.draw_text(0, bar_y, &text, Color::rgb(169, 177, 214));
    }

    fn render_dialog_overlay(&self, ctx: &mut RenderContext) {
        let top = self.store.dialog_stack.top();
        match top {
            Some(DialogKind::Alert) => self.alert.render(ctx),
            Some(DialogKind::Help) => self.help.render(ctx),
            _ => {}
        }
    }
}

impl View for RootView {
    fn render(&self, ctx: &mut RenderContext) {
        let route = self.store.route.get();

        match &route {
            Route::Home => {
                HomeScreen { store: &self.store }.render(ctx);
            }
            Route::Session { session_id } => {
                let screen = SessionScreen::new(
                    session_id.clone(),
                    self.api.clone(),
                );
                screen.render(ctx);
            }
        }

        self.render_status_bar(ctx);

        // Dialog overlay — rendered last, on top of everything
        if self.store.dialog_stack.is_open() {
            self.render_dialog_overlay(ctx);
        }
    }
}
