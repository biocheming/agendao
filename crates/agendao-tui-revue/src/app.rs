//! Application entry point and event loop — 火 (execution authority).

use anyhow::Context;
use revue::prelude::*;
use revue::event::{Event, Key};
use std::path::PathBuf;

use crate::bridge::api::ApiBridge;
use crate::dialog::{AlertDialog, DialogKind, HelpDialog};
use crate::store::app_store::{AppStore, Route};
use crate::screen::{HomeScreen, SessionScreen};
use crate::telemetry::event_bus::EventBus;
use crate::telemetry::event_handler::apply_frontend_event;
use crate::store::session_store::SessionStore;
use crate::transport;

pub fn run_app() -> anyhow::Result<()> {
    let store = AppStore::new();
    let api = ApiBridge::new("http://127.0.0.1:3000").ok();
    let event_bus = EventBus::new();
    let active_session: SessionStore = SessionStore::new();

    // Spawn transport (no-op when local-server feature disabled)
    let tx = event_bus.sender();
    let wd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let _transport_task = transport::spawn_event_source(tx, wd);

    let mut app = App::builder().mouse_capture(true).build();
    let view = RootView::new(store.clone(), api, active_session.clone());
    let mut handler = AppHandler { store, api: None, alert: AlertDialog::new(), help: HelpDialog::new(), dialog_open: false, dialog_kind: None, active_session, event_bus };

    app.run(view, move |event, view, _app| {
        handler.handle(event, view)
    }).context("agendao TUI runtime exited with error")
}

struct AppHandler {
    store: AppStore,
    #[allow(dead_code)]
    api: Option<ApiBridge>,
    alert: AlertDialog,
    help: HelpDialog,
    dialog_open: bool,
    dialog_kind: Option<DialogKind>,
    active_session: SessionStore,
    event_bus: EventBus,
}

impl AppHandler {
    fn handle(&mut self, event: &Event, _view: &mut RootView) -> bool {
        match event {
            Event::Tick => {
                // 火→土: drain EventBus, apply to active session
                let events = self.event_bus.drain();
                let mut changed = false;
                for fe in &events {
                    let sid = apply_frontend_event(fe, &self.active_session);
                    changed |= sid.is_some();
                }
                changed
            }
            Event::Key(key) => self.handle_key(&key.key),
            Event::Resize(..) => true,
            _ => false,
        }
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

    fn toggle_help(&mut self) {
        if self.help.visible { self.close_dialog(); }
        else { self.help.toggle(); self.dialog_open = true; self.dialog_kind = Some(DialogKind::Help); }
    }

    fn close_dialog(&mut self) { self.dialog_open = false; self.dialog_kind = None; self.help.dismiss(); self.alert.dismiss(); }
}

struct RootView {
    store: AppStore,
    #[allow(dead_code)]
    api: Option<ApiBridge>,
    #[allow(dead_code)]
    active_session: SessionStore,
}

impl RootView {
    fn new(store: AppStore, api: Option<ApiBridge>, active_session: SessionStore) -> Self {
        Self { store, api, active_session }
    }

    fn render_status_bar(&self, ctx: &mut RenderContext) {
        let route_label = self.store.route.get().as_str();
        let wd = self.store.working_dir.get();
        let short_dir = if wd.len() > 30 { format!("...{}", &wd[wd.len().saturating_sub(27)..]) } else { wd };
        let text = format!(" {} | [{}] | q:quit h:home ?:help ", short_dir, route_label);
        let bar_y = ctx.area.height.saturating_sub(1);
        for x in 0..ctx.area.width { ctx.draw_text(x, bar_y, " ", Color::rgb(30, 32, 44)); }
        ctx.draw_text(0, bar_y, &text, Color::rgb(169, 177, 214));
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
    }
}
