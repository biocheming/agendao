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
    }).context("agendao TUI runtime exited with error")
}

struct RootView {
    store: AppStore,
    api: Option<ApiBridge>,
    alert: AlertDialog,
    help: HelpDialog,
    dialog_open: bool,
    dialog_kind: Option<DialogKind>,
}

impl RootView {
    fn new(store: AppStore, api: Option<ApiBridge>) -> Self {
        Self { store, api, alert: AlertDialog::new(), help: HelpDialog::new(), dialog_open: false, dialog_kind: None }
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        if self.dialog_open {
            return self.handle_dialog_key(key);
        }
        match key {
            Key::Char('q') | Key::Escape => { self.store.request_exit(); true }
            Key::Char('h') => { self.store.navigate(Route::Home); true }
            Key::Char('?') => { self.toggle_help(); true }
            _ => false,
        }
    }

    fn handle_dialog_key(&mut self, key: &Key) -> bool {
        match self.dialog_kind {
            Some(DialogKind::Alert) => self.alert.handle_key(key),
            Some(DialogKind::Help) => self.help.handle_key(key),
            _ => { if matches!(key, Key::Escape) { self.close_dialog(); } true }
        };
        if !self.alert.visible && !self.help.visible { self.close_dialog(); }
        true
    }

    #[allow(dead_code)]
    pub fn show_alert(&mut self, title: &str, msg: &str) {
        self.alert.show(title, msg);
        self.dialog_open = true; self.dialog_kind = Some(DialogKind::Alert);
    }

    pub fn toggle_help(&mut self) {
        if self.help.visible { self.close_dialog(); }
        else { self.help.toggle(); self.dialog_open = true; self.dialog_kind = Some(DialogKind::Help); }
    }

    fn close_dialog(&mut self) { self.dialog_open = false; self.dialog_kind = None; self.help.dismiss(); self.alert.dismiss(); }

    fn render_status_bar(&self, ctx: &mut RenderContext) {
        let route_label = self.store.route.get().as_str();
        let wd = self.store.working_dir.get();
        let short_dir = if wd.len() > 30 { format!("...{}", &wd[wd.len().saturating_sub(27)..]) } else { wd };
        let text = format!(" {} | [{}] | q:quit h:home ?:help ", short_dir, route_label);
        let bar_y = ctx.area.height.saturating_sub(1);
        for x in 0..ctx.area.width { ctx.draw_text(x, bar_y, " ", Color::rgb(30, 32, 44)); }
        ctx.draw_text(0, bar_y, &text, Color::rgb(169, 177, 214));
    }

    fn render_dialog_overlay(&self, ctx: &mut RenderContext) {
        if !self.dialog_open { return; }
        match self.dialog_kind {
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
            Route::Home => { HomeScreen { store: &self.store }.render(ctx); }
            Route::Session { session_id } => {
                SessionScreen::new(session_id.clone(), self.api.clone()).render(ctx);
            }
        }
        self.render_status_bar(ctx);
        self.render_dialog_overlay(ctx);
    }
}
