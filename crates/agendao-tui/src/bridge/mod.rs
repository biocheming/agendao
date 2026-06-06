use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use crossterm::event::{Event as CrosstermEvent, EventStream, MouseEventKind};
use parking_lot::Mutex;
use parking_lot::RwLock;
use reratui::element::Element;
use reratui::fiber_tree::{clear_fiber_tree, set_fiber_tree};
use reratui::hooks::{use_context, use_context_provider, use_event};
use reratui::scheduler::{batch, effect_queue};
use reratui::{
    clear_current_event, clear_global_handlers, clear_render_context, init_render_context,
    reset_component_position_counter, set_current_event, Buffer, Component, FiberTree, Rect,
};
use tokio::sync::Notify;
use tokio_stream::StreamExt;

use crate::app::{App, RunOutcome};
use crate::core::{is_primary_key_event, AppContext, CustomEvent, Event, Route, StateChange};
use crate::ui::BufferSurface;

#[derive(Clone, Debug)]
pub struct UiBridgeSnapshot {
    pub revision: u64,
    pub last_event: Option<Event>,
    pub pending_events: usize,
    pub high_water_mark: usize,
    pub coalesced_events: u64,
    pub dropped_events: u64,
    pub capacity: usize,
}

#[derive(Clone, Default)]
pub struct UiBridge {
    queue: Arc<Mutex<VecDeque<Event>>>,
    last_event: Arc<RwLock<Option<Event>>>,
    revision: Arc<AtomicU64>,
    high_water_mark: Arc<AtomicUsize>,
    coalesced_events: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
    capacity: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

#[cfg(test)]
const DEFAULT_UI_BRIDGE_QUEUE: usize = 64;
#[cfg(not(test))]
const DEFAULT_UI_BRIDGE_QUEUE: usize = 4_096;

impl UiBridge {
    pub fn new() -> Self {
        Self {
            capacity: Arc::new(AtomicUsize::new(DEFAULT_UI_BRIDGE_QUEUE)),
            ..Self::default()
        }
    }

    pub fn set_capacity(&self, capacity: usize) {
        self.capacity.store(capacity.max(1), Ordering::SeqCst);
    }

    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::SeqCst)
    }

    pub fn emit(&self, event: Event) -> bool {
        self.record(&event);
        let mut queue = self.queue.lock();
        if let Some(index) = queue
            .iter()
            .rposition(|queued| queued_event_is_superseded_by(queued, &event))
        {
            queue[index] = event;
            self.coalesced_events.fetch_add(1, Ordering::SeqCst);
        } else {
            if queue.len() >= self.capacity() {
                queue.pop_front();
                self.dropped_events.fetch_add(1, Ordering::SeqCst);
            }
            queue.push_back(event);
            self.high_water_mark
                .fetch_max(queue.len(), Ordering::SeqCst);
        }
        self.notify.notify_one();
        true
    }

    pub fn emit_custom(&self, event: crate::core::CustomEvent) -> bool {
        self.emit(Event::Custom(Box::new(event)))
    }

    pub fn record(&self, event: &Event) {
        *self.last_event.write() = Some(event.clone());
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> UiBridgeSnapshot {
        let pending_events = self.queue.lock().len();
        UiBridgeSnapshot {
            revision: self.revision.load(Ordering::SeqCst),
            last_event: self.last_event.read().clone(),
            pending_events,
            high_water_mark: self.high_water_mark.load(Ordering::SeqCst),
            coalesced_events: self.coalesced_events.load(Ordering::SeqCst),
            dropped_events: self.dropped_events.load(Ordering::SeqCst),
            capacity: self.capacity(),
        }
    }

    pub fn drain(&self, limit: usize) -> Vec<Event> {
        let mut queue = self.queue.lock();
        let mut drained = Vec::with_capacity(limit.min(queue.len()));
        for _ in 0..limit {
            let Some(event) = queue.pop_front() else {
                break;
            };
            drained.push(event);
        }
        drained
    }

    pub fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }
}

fn queued_event_is_superseded_by(queued: &Event, incoming: &Event) -> bool {
    if let (Event::Mouse(queued_mouse), Event::Mouse(incoming_mouse)) = (queued, incoming) {
        return matches!(queued_mouse.kind, MouseEventKind::Moved)
            && matches!(incoming_mouse.kind, MouseEventKind::Moved);
    }

    let (
        Some((queued_session_id, queued_id, queued_kind)),
        Some((incoming_session_id, incoming_id, incoming_kind)),
    ) = (
        scheduler_stage_output_block_identity(queued),
        scheduler_stage_output_block_identity(incoming),
    )
    else {
        return false;
    };

    queued_session_id == incoming_session_id
        && queued_id == incoming_id
        && queued_kind == incoming_kind
}

fn scheduler_stage_output_block_identity(event: &Event) -> Option<(&str, Option<&str>, &str)> {
    let Event::Custom(custom) = event else {
        return None;
    };
    let CustomEvent::StateChanged(StateChange::OutputBlock {
        session_id,
        id,
        payload,
        ..
    }) = custom.as_ref()
    else {
        return None;
    };
    let kind = payload.get("kind").and_then(|value| value.as_str())?;
    if kind != "scheduler_stage" {
        return None;
    }
    Some((session_id.as_str(), id.as_deref(), kind))
}

#[derive(Default)]
struct RuntimeErrorSink {
    error: Mutex<Option<anyhow::Error>>,
}

impl RuntimeErrorSink {
    fn store(&self, error: anyhow::Error) {
        let mut slot = self.error.lock();
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn take(&self) -> Option<anyhow::Error> {
        self.error.lock().take()
    }
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn process_app_event_blocking(app: &Arc<Mutex<App>>, event: &Event) -> anyhow::Result<bool> {
    app.lock().process_event(event)
}

fn drain_app_pending_events_blocking(app: &Arc<Mutex<App>>, limit: usize) -> anyhow::Result<bool> {
    app.lock().drain_pending_events(limit)
}

fn draw_app_frame_blocking(
    terminal: &mut crate::app::terminal::Tui,
    app: &Arc<Mutex<App>>,
    errors: &Arc<RuntimeErrorSink>,
) -> anyhow::Result<Arc<Mutex<Option<(u16, u16)>>>> {
    let reactive_cursor = Arc::new(Mutex::new(None));
    debug_assert!(
        app.lock().can_render_reactive_route(),
        "legacy frame fallback should be unreachable after reratui migration"
    );
    terminal.draw(|frame| {
        reset_component_position_counter();
        let root = Element::component(ReactiveRootComponent {
            app: app.clone(),
            cursor: reactive_cursor.clone(),
            errors: errors.clone(),
        });
        root.render(frame.area(), frame.buffer_mut());
        if let Some((x, y)) = *reactive_cursor.lock() {
            frame.set_cursor_position((x, y));
        }
    })?;
    Ok(reactive_cursor)
}

#[derive(Clone)]
struct TerminalEventBridge {
    app: Arc<Mutex<App>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
struct ReactiveRootComponent {
    app: Arc<Mutex<App>>,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
struct ReactiveRouteComponent {
    app: Arc<Mutex<App>>,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
struct ReactiveSessionRouteComponent {
    app: Arc<Mutex<App>>,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
    session_id: String,
}

#[derive(Clone)]
struct ReactiveSessionViewComponent {
    app: Arc<Mutex<App>>,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
pub(crate) struct ReactiveAppContextHandle(pub(crate) Arc<AppContext>);

#[derive(Clone)]
pub(crate) struct ReactiveSessionContext {
    pub(crate) session_id: String,
}

impl Component for TerminalEventBridge {
    fn render(&self, _area: Rect, _buffer: &mut Buffer) {
        let Some(raw_event) = use_event() else {
            return;
        };

        let Some(event) = map_crossterm_event(raw_event) else {
            return;
        };

        if let Err(error) = process_app_event_blocking(&self.app, &event) {
            self.errors.store(error);
        }
    }
}

impl Component for ReactiveRootComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = {
            let app = self.app.lock();
            if !app.can_render_reactive_route() {
                *self.cursor.lock() = None;
                return;
            }

            app.context_handle()
        };

        let _app_context = use_context_provider(|| ReactiveAppContextHandle(app_context));
        let root = Element::component(ReactiveRouteComponent {
            app: self.app.clone(),
            cursor: self.cursor.clone(),
            errors: self.errors.clone(),
        });
        root.render(area, buffer);
    }
}

impl Component for ReactiveRouteComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = use_context::<ReactiveAppContextHandle>().0;
        let route = app_context.current_route();

        {
            self.app.lock().begin_reactive_render(area);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            *self.cursor.lock() = None;

            match &route {
                Route::Session { session_id } => {
                    let session_route = Element::component(ReactiveSessionRouteComponent {
                        app: self.app.clone(),
                        cursor: self.cursor.clone(),
                        errors: self.errors.clone(),
                        session_id: session_id.clone(),
                    })
                    .with_key(session_id.clone());
                    session_route.render(area, buffer);
                }
                _ => {
                    let mut surface = BufferSurface::new(buffer);
                    let app = self.app.lock();
                    app.render_home_view(&mut surface, area);
                    *self.cursor.lock() = surface.cursor_position();
                }
            }

            let theme = app_context.theme.read().clone();
            {
                let mut surface = BufferSurface::new(buffer);
                let mut app = self.app.lock();
                app.render_reactive_dialog_layer(&mut surface, area, &theme);
                app.render_reactive_toast(&mut surface, area, &theme);
            }

            let mut app = self.app.lock();
            app.capture_reactive_screen_lines(buffer, area);
            app.apply_reactive_selection(buffer, area);
        }));

        if let Err(payload) = result {
            self.errors.store(anyhow::anyhow!(
                "reactive route render panicked: {}",
                panic_payload_message(payload.as_ref())
            ));
        }
    }
}

impl Component for ReactiveSessionRouteComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let _session_context = use_context_provider(|| ReactiveSessionContext {
            session_id: self.session_id.clone(),
        });

        let child = Element::component(ReactiveSessionViewComponent {
            app: self.app.clone(),
            cursor: self.cursor.clone(),
            errors: self.errors.clone(),
        });
        child.render(area, buffer);
    }
}

impl Component for ReactiveSessionViewComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = use_context::<ReactiveAppContextHandle>().0;
        let session = use_context::<ReactiveSessionContext>();

        let view = {
            let mut app = self.app.lock();
            app.ensure_session_view(&session.session_id);
            app_context.session_view_handle()
        };
        let Some(view) = view else {
            *self.cursor.lock() = None;
            return;
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let app = self.app.lock();
            app.render_session_view(&view, &app_context, buffer, area)
        }));

        match result {
            Ok(cursor) => {
                *self.cursor.lock() = cursor;
            }
            Err(payload) => {
                self.errors.store(anyhow::anyhow!(
                    "reactive session render panicked: {}",
                    panic_payload_message(payload.as_ref())
                ));
            }
        }
    }
}

pub fn run_app(app: App) -> anyhow::Result<RunOutcome> {
    let shared = Arc::new(Mutex::new(app));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create tokio runtime for agendao-tui")?;

    let result = runtime.block_on(run_app_async(shared.clone()));
    let app = Arc::try_unwrap(shared)
        .map_err(|_| anyhow::anyhow!("agendao-tui runtime still holds shared app state"))?
        .into_inner();
    let exit_summary = if result.is_ok() {
        app.exit_summary()
    } else {
        None
    };

    drop(app);
    result?;
    Ok(RunOutcome { exit_summary })
}

async fn run_app_async(app: Arc<Mutex<App>>) -> anyhow::Result<()> {
    let errors = Arc::new(RuntimeErrorSink::default());
    let mut events = EventStream::new();
    let mut first_frame = true;
    let mut terminal = crate::app::terminal::init()
        .context("failed to initialize ratatui terminal for reratui bridge")?;
    let ui_bridge = app.lock().context_handle().ui_bridge.clone();

    set_fiber_tree(FiberTree::new());
    init_render_context();
    batch::init_main_thread();
    let server_event_task = app.lock().spawn_server_event_listener_task();

    if let Ok(area) = terminal.size() {
        app.lock().set_viewport_area(area.into());
    }

    let result = async {
        loop {
            if app.lock().is_exiting() {
                break;
            }

            let now = Instant::now();
            let tick_deadline = app.lock().next_tick_deadline(now);
            let tick_due = tick_deadline.is_some_and(|deadline| deadline <= now);
            let bridge_pending = ui_bridge.snapshot().pending_events > 0;
            let should_wait = !first_frame && !tick_due && !bridge_pending;

            let polled_event = if should_wait {
                let bridge_notified = ui_bridge.notified();
                tokio::pin!(bridge_notified);
                if let Some(deadline) = tick_deadline {
                    let timeout =
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                    tokio::pin!(timeout);
                    tokio::select! {
                        Some(Ok(event)) = events.next() => Some(event),
                        _ = &mut bridge_notified => None,
                        _ = &mut timeout => None,
                    }
                } else {
                    tokio::select! {
                        Some(Ok(event)) = events.next() => Some(event),
                        _ = &mut bridge_notified => None,
                    }
                }
            } else {
                None
            };

            if matches!(polled_event, Some(CrosstermEvent::Resize(_, _))) {
                terminal.autoresize()?;
                if let Ok(area) = terminal.size() {
                    app.lock().set_viewport_area(area.into());
                }
            }

            let mut should_draw = first_frame;

            let tick_due_now = app
                .lock()
                .next_tick_deadline(Instant::now())
                .is_some_and(|deadline| deadline <= Instant::now());
            if tick_due_now {
                should_draw |= process_app_event_blocking(&app, &Event::Tick)?;
            }

            let max_events_per_frame = app
                .lock()
                .context_handle()
                .runtime_budget()
                .max_events_per_frame
                .max(1);
            should_draw |= drain_app_pending_events_blocking(&app, max_events_per_frame)?;

            let bridge_event = polled_event.as_ref().and_then(|event| {
                if map_crossterm_event(event.clone()).is_some() {
                    Some(event.clone())
                } else {
                    None
                }
            });

            if let Some(event) = bridge_event.as_ref() {
                set_current_event(Some(Arc::new(event.clone())));
            } else {
                clear_current_event();
            }

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.prepare_for_render();
            });
            reset_component_position_counter();
            clear_global_handlers();

            if bridge_event.is_some() {
                let bridge = Element::component(TerminalEventBridge {
                    app: app.clone(),
                    errors: errors.clone(),
                });
                let area = Rect::new(0, 0, 1, 1);
                let mut buffer = Buffer::empty(area);
                bridge.render(area, &mut buffer);
                should_draw = true;
            }

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.mark_unseen_for_unmount();
            });

            if let Some(error) = errors.take() {
                return Err(error);
            }

            if should_draw {
                let _ = draw_app_frame_blocking(&mut terminal, &app, &errors)?;
                first_frame = false;
            }

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.process_unmounts();
            });

            batch::begin_batch();
            batch::drain_cross_thread_updates();
            let _ = batch::end_batch();
            clear_current_event();

            if let Some(error) = errors.take() {
                return Err(error);
            }

            effect_queue::flush_effects();
            effect_queue::flush_async_effects().await;

            if let Some(error) = errors.take() {
                return Err(error);
            }
        }

        Ok(())
    }
    .await;

    clear_current_event();
    clear_fiber_tree();
    clear_render_context();
    if let Some(task) = server_event_task {
        task.abort();
    }
    let _ = crate::app::terminal::restore();

    result
}

fn map_crossterm_event(event: CrosstermEvent) -> Option<Event> {
    match event {
        CrosstermEvent::Key(key) if is_primary_key_event(key) => Some(Event::Key(key)),
        CrosstermEvent::Key(_) => None,
        CrosstermEvent::Mouse(mouse) => Some(Event::Mouse(mouse)),
        CrosstermEvent::Resize(width, height) => Some(Event::Resize(width, height)),
        CrosstermEvent::FocusGained => Some(Event::FocusGained),
        CrosstermEvent::FocusLost => Some(Event::FocusLost),
        CrosstermEvent::Paste(text) => Some(Event::Paste(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CustomEvent, StateChange};

    fn scheduler_stage_event(session_id: &str, id: &str, text: &str) -> Event {
        Event::Custom(Box::new(CustomEvent::StateChanged(
            StateChange::OutputBlock {
                session_id: session_id.to_string(),
                id: Some(id.to_string()),
                payload: serde_json::json!({
                    "kind": "scheduler_stage",
                    "text": text,
                }),
                live_identity: None,
            },
        )))
    }

    fn message_delta_event(session_id: &str, id: &str, text: &str) -> Event {
        Event::Custom(Box::new(CustomEvent::StateChanged(
            StateChange::OutputBlock {
                session_id: session_id.to_string(),
                id: Some(id.to_string()),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": text,
                }),
                live_identity: None,
            },
        )))
    }

    #[test]
    fn ui_bridge_coalesces_pending_scheduler_stage_snapshots() {
        let bridge = UiBridge::new();

        bridge.emit(scheduler_stage_event("session-1", "msg-1", "old"));
        bridge.emit(scheduler_stage_event("session-1", "msg-1", "new"));

        let snapshot = bridge.snapshot();
        assert_eq!(snapshot.coalesced_events, 1);
        assert_eq!(snapshot.pending_events, 1);

        let drained = bridge.drain(10);
        assert_eq!(drained.len(), 1);
        let Event::Custom(custom) = &drained[0] else {
            panic!("expected custom event");
        };
        let CustomEvent::StateChanged(StateChange::OutputBlock { payload, .. }) = custom.as_ref()
        else {
            panic!("expected output block");
        };
        assert_eq!(payload["text"], "new");
    }

    #[test]
    fn ui_bridge_keeps_message_deltas_distinct() {
        let bridge = UiBridge::new();

        bridge.emit(message_delta_event("session-1", "msg-1", "a"));
        bridge.emit(message_delta_event("session-1", "msg-1", "b"));

        let drained = bridge.drain(10);
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn ui_bridge_caps_pending_queue_length() {
        let bridge = UiBridge::new();

        for index in 0..(DEFAULT_UI_BRIDGE_QUEUE + 5) {
            bridge.emit(message_delta_event(
                "session-1",
                &format!("msg-{index}"),
                "payload",
            ));
        }

        let snapshot = bridge.snapshot();
        assert_eq!(snapshot.pending_events, DEFAULT_UI_BRIDGE_QUEUE);
        assert_eq!(snapshot.dropped_events, 5);
        assert_eq!(snapshot.high_water_mark, DEFAULT_UI_BRIDGE_QUEUE);
    }
}
