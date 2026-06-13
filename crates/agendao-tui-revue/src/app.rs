//! Application entry point and event loop — 火 (execution authority).

use anyhow::Context;
use revue::prelude::*;
use revue::event::{Event, Key};
use tokio::sync::watch;
use std::cell::RefCell;

use crate::bridge::api::ApiBridge;
use crate::config::AppConfig;
use crate::dialog::{AlertDialog, DialogKind, HelpDialog};
use crate::input::{PromptAction, PromptInput};
use crate::store::app_store::{AppStore, Route};
use crate::screen::{HomeScreen, SessionScreen};
use crate::telemetry::event_bus::EventBus;
use crate::telemetry::event_handler::apply_frontend_event;
use crate::store::session_store::SessionStore;
use crate::store::types::RunStatus;
use crate::transport;

pub fn run_app() -> anyhow::Result<()> { run_app_with_config(AppConfig::default()) }

pub fn run_app_with_config(config: crate::config::AppConfig) -> anyhow::Result<()> {
    let store = AppStore::new();
    if let Some(ref dir) = config.working_dir { store.working_dir.set(dir.display().to_string()); }
    let rt = tokio::runtime::Runtime::new().map_err(|e| anyhow::anyhow!("tokio runtime: {}", e))?;
    let api = if !config.local_direct {
        ApiBridge::new(&config.base_url.clone().unwrap_or_else(|| "http://127.0.0.1:3000".into()), rt.handle().clone()).ok()
    } else { None };
    let (session_filter_tx, session_filter_rx) = watch::channel::<Option<String>>(None);
    if let Some(ref sid) = config.session_id {
        session_filter_tx.send_replace(Some(sid.clone()));
        store.navigate(Route::Session { session_id: sid.clone() });
    }
    let event_bus = EventBus::new();
    let active_session = SessionStore::new();
    let tx = event_bus.sender();
    let wd = config.working_dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let _ = transport::spawn_event_source(tx, wd, &rt.handle(), session_filter_rx, config.unix_socket_path.clone(), config.base_url.clone());
    if let Some(ref a) = config.agent_name { store.selected_agent.set(Some(a.clone())); }
    if let Some(ref m) = config.model { store.selected_model.set(Some(m.clone())); }

    let mut app = App::builder().mouse_capture(true).style("styles/base.css").build();
    let handler = RefCell::new(AppHandler::new(store.clone(), api.clone(), active_session.clone(), event_bus, session_filter_tx));
    let view = RootView { store, api, active_session, handler };

    app.run(view, move |event, view, _app| {
        view.handler.borrow_mut().handle(event)
    }).context("agendao TUI runtime exited with error")
}

struct AppHandler {
    store: AppStore,
    api: Option<ApiBridge>,
    prompt: PromptInput,
    alert: AlertDialog, help: HelpDialog,
    dialog_open: bool, dialog_kind: Option<DialogKind>,
    active_session: SessionStore, event_bus: EventBus,
    #[allow(dead_code)] session_filter_tx: watch::Sender<Option<String>>,
}

impl AppHandler {
    fn new(s: AppStore, a: Option<ApiBridge>, ss: SessionStore, eb: EventBus, sf: watch::Sender<Option<String>>) -> Self {
        Self { store: s, api: a, prompt: PromptInput::new(), alert: AlertDialog::new(), help: HelpDialog::new(), dialog_open: false, dialog_kind: None, active_session: ss, event_bus: eb, session_filter_tx: sf }
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Tick => { let events = self.event_bus.drain(); let mut c = false; for fe in &events { c |= apply_frontend_event(fe, &self.active_session).is_some(); } c }
            Event::Key(key) => self.handle_key(&key.key),
            Event::Mouse(m) => {
                use revue::event::MouseEventKind;
                match m.kind { MouseEventKind::ScrollUp => { self.active_session.scroll_up(); true } MouseEventKind::ScrollDown => { self.active_session.scroll_down(); true } _ => false }
            }
            Event::Resize(..) => true,
            _ => false,
        }
    }

    fn handle_key(&mut self, key: &Key) -> bool {
        if self.dialog_open { return self.handle_dialog_key(key); }
        match self.prompt.handle_key(key) {
            PromptAction::Submit(text) => { self.dispatch(text); return true; }
            PromptAction::SubmitShell(cmd) => { self.dispatch_shell(cmd); return true; }
            PromptAction::None => {}
        }
        match key {
            Key::Char('q') | Key::Escape => { self.store.request_exit(); true }
            Key::Char('h') => { self.store.navigate(Route::Home); true }
            Key::Char('?') => { self.toggle_help(); true }
            _ => false,
        }
    }

    fn dispatch(&mut self, text: String) {
        let route = self.store.route.get();
        let sid = match route {
            Route::Home => {
                if let Some(ref api) = self.api {
                    match api.create_session(None, None) {
                        Ok(info) => {
                            self.active_session.set_session_id(&info.id);
                            self.store.navigate(Route::Session { session_id: info.id.clone() });
                            info.id
                        }
                        Err(e) => { self.active_session.run_status.set(RunStatus::Error(format!("{}", e))); return; }
                    }
                } else { "echo".to_string() }
            }
            Route::Session { session_id } => session_id,
        };
        let mid = format!("user-{}", ts_now());
        self.active_session.push_user_message(&mid, &text);
        self.active_session.run_status.set(RunStatus::Sending);
        if let Some(ref api) = self.api {
            match api.send_prompt(&sid, text) {
                Ok(r) => {
                    let rid = format!("r-{}", ts_now());
                    self.active_session.push_assistant_delta(&rid, &format_status(&r));
                    self.active_session.run_status.set(RunStatus::Idle);
                }
                Err(e) => { self.active_session.run_status.set(RunStatus::Error(format!("{}", e))); }
            }
        } else {
            self.active_session.push_assistant_delta(&format!("echo-{}", ts_now()), &format!("[echo] {}", text));
            self.active_session.run_status.set(RunStatus::Idle);
            self.store.navigate(Route::Session { session_id: "echo".into() });
        }
    }

    fn dispatch_shell(&mut self, _cmd: String) {}
    fn handle_dialog_key(&mut self, key: &Key) -> bool {
        match self.dialog_kind { Some(DialogKind::Alert) => self.alert.handle_key(key), Some(DialogKind::Help) => self.help.handle_key(key), _ => { if matches!(key, Key::Escape) { self.close_dialog(); } true } };
        if !self.alert.visible && !self.help.visible { self.close_dialog(); } true
    }
    fn toggle_help(&mut self) { if self.help.visible { self.close_dialog(); } else { self.help.toggle(); self.dialog_open = true; self.dialog_kind = Some(DialogKind::Help); } }
    fn close_dialog(&mut self) { self.dialog_open = false; self.dialog_kind = None; self.help.dismiss(); self.alert.dismiss(); }
}

struct RootView {
    store: AppStore,
    #[allow(dead_code)] api: Option<ApiBridge>,
    #[allow(dead_code)] active_session: SessionStore,
    handler: RefCell<AppHandler>,
}

impl View for RootView {
    fn render(&self, ctx: &mut RenderContext) {
        let route = self.store.route.get();
        let h = self.handler.borrow();
        let area = ctx.area;

        match &route {
            Route::Home => { HomeScreen { store: &self.store }.render(ctx); }
            Route::Session { session_id } => {
                SessionScreen::new(session_id.clone(), self.api.clone()).render(ctx);
            }
        }

        // Global prompt bar at bottom
        let py = area.height.saturating_sub(2);
        let is_running = matches!(h.active_session.run_status.get(), RunStatus::Sending | RunStatus::Running);
        let hint = h.prompt.status_hint(is_running);
        ctx.draw_text(0, py, &format!(" {}", hint), Color::rgb(86, 95, 137));
        ctx.draw_text(0, py + 1, "> ", Color::rgb(125, 207, 255));

        // Status bar
        let bar_y = area.height.saturating_sub(1);
        let route_label = route.as_str();
        let text = format!(" agendao | [{}] | q:quit ?:help ", route_label);
        for x in 0..area.width { ctx.draw_text(x, bar_y, " ", Color::rgb(30, 32, 44)); }
        ctx.draw_text(0, bar_y, &text, Color::rgb(169, 177, 214));
    }
}

fn format_status(r: &agendao_client::PromptResponse) -> String {
    match r.status.as_str() {
        "awaiting_user" => format!("⏳ Awaiting{}", r.pending_question_id.as_deref().map_or(String::new(), |id| format!(" ({})", id))),
        "queued" => format!("📨 Queued ({} ahead)", r.queued_count.unwrap_or(0)),
        _ => format!("✅ Sent ({})", r.status),
    }
}
fn ts_now() -> String { use std::time::SystemTime; SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| format!("{}", d.as_millis())).unwrap_or_default() }
