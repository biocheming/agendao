use std::any::Any;
use std::collections::VecDeque;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

use anyhow::Context;
use crossterm::event::{Event as CrosstermEvent, EventStream, MouseEventKind};
use parking_lot::Mutex;
use parking_lot::RwLock;
use reratui::element::Element;
use reratui::fiber_tree::{clear_fiber_tree, set_fiber_tree};
use reratui::hooks::{use_context, use_context_provider};
use reratui::scheduler::{batch, effect_queue};
use reratui::{
    clear_current_event, clear_global_handlers, clear_render_context, init_render_context,
    reset_component_position_counter, set_current_event, Buffer, Component, FiberTree, Rect,
};
use tokio::sync::Notify;
use tokio_stream::Stream;

use crate::app::{
    App, BridgeIterationOutcome, BridgeWaitStrategy, ReactiveDialogLayerSnapshot, RunOutcome,
};
use crate::components::HomeView;
use crate::core::{is_primary_key_event, AppContext, CustomEvent, Event, Route};
use crate::ui::{BufferSurface, RenderSurface};

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
    let (session_id, id, payload) = match custom.as_ref() {
        CustomEvent::FrontendEvent(event) => match event.as_ref() {
            agendao_server_core::frontend_events::FrontendEvent::OutputBlockAppended {
                session_id,
                id,
                block,
                ..
            } => (session_id.as_str(), id.as_deref(), block),
            _ => return None,
        },
        _ => return None,
    };
    let kind = payload.get("kind").and_then(|value| value.as_str())?;
    if kind != "scheduler_stage" {
        return None;
    }
    Some((session_id, id, kind))
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

fn draw_app_frame_blocking(
    terminal: &mut crate::app::terminal::Tui,
    snapshot: ReactiveRootSnapshot,
    errors: &Arc<RuntimeErrorSink>,
) -> anyhow::Result<Arc<Mutex<Option<(u16, u16)>>>> {
    let reactive_cursor = Arc::new(Mutex::new(None));
    terminal.draw(|frame| {
        reset_component_position_counter();
        let root = Element::component(ReactiveRootComponent {
            snapshot: snapshot.clone(),
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
struct ReactiveRootComponent {
    snapshot: ReactiveRootSnapshot,
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
pub(crate) struct ReactiveRootSnapshot {
    pub(crate) app_context: Arc<AppContext>,
    pub(crate) theme: crate::theme::Theme,
    pub(crate) keybinds: crate::context::KeybindRegistry,
    pub(crate) route: Route,
    pub(crate) animations_enabled: bool,
    pub(crate) prompt_input_blocked: bool,
    pub(crate) slash_popup_open: bool,
    pub(crate) prompt: crate::components::Prompt,
    pub(crate) selection: crate::ui::Selection,
    pub(crate) toast: crate::components::Toast,
    pub(crate) dialog_layer: ReactiveDialogLayerSnapshot,
    pub(crate) screen_lines: Arc<std::sync::Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct ReactiveRouteComponent {
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
    screen_lines: Arc<std::sync::Mutex<Vec<String>>>,
    dialog_layer: ReactiveDialogLayerSnapshot,
}

#[derive(Clone)]
struct ReactiveSessionRouteComponent {
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
    session_id: String,
}

#[derive(Clone)]
struct ReactiveSessionViewComponent {
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
struct ReactiveHomeRouteComponent {
    cursor: Arc<Mutex<Option<(u16, u16)>>>,
    errors: Arc<RuntimeErrorSink>,
}

#[derive(Clone)]
pub(crate) struct ReactiveAppContextHandle(pub(crate) Arc<AppContext>);

#[derive(Clone)]
pub(crate) struct ReactiveRouteSnapshot(pub(crate) Route);

#[derive(Clone, Copy)]
pub(crate) struct ReactiveAnimationsEnabled(pub(crate) bool);

#[derive(Clone, Copy)]
pub(crate) struct ReactivePromptInputBlocked(pub(crate) bool);

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct ReactiveSlashPopupOpen(pub(crate) bool);

#[derive(Clone)]
pub(crate) struct ReactiveUiEventEmitter(pub(crate) Arc<AppContext>);

#[derive(Clone)]
pub(crate) struct ReactiveSessionViewHandle(pub(crate) Option<crate::components::SessionView>);

#[derive(Clone)]
pub(crate) struct ReactivePromptHandle(pub(crate) crate::components::Prompt);

#[derive(Clone)]
struct ReactiveSelection(pub(crate) crate::ui::Selection);

#[derive(Clone)]
struct ReactiveToast(pub(crate) crate::components::Toast);

impl Component for ReactiveRootComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let _app_context =
            use_context_provider(|| ReactiveAppContextHandle(self.snapshot.app_context.clone()));
        let _event_emitter =
            use_context_provider(|| ReactiveUiEventEmitter(self.snapshot.app_context.clone()));
        let _theme = use_context_provider(|| self.snapshot.theme.clone());
        let _keybinds = use_context_provider(|| self.snapshot.keybinds.clone());
        let _route = use_context_provider(|| ReactiveRouteSnapshot(self.snapshot.route.clone()));
        let _animations =
            use_context_provider(|| ReactiveAnimationsEnabled(self.snapshot.animations_enabled));
        let _prompt_input_blocked = use_context_provider(|| {
            ReactivePromptInputBlocked(self.snapshot.prompt_input_blocked)
        });
        let _slash_popup_open =
            use_context_provider(|| ReactiveSlashPopupOpen(self.snapshot.slash_popup_open));
        let _prompt = use_context_provider(|| ReactivePromptHandle(self.snapshot.prompt.clone()));
        let _selection =
            use_context_provider(|| ReactiveSelection(self.snapshot.selection.clone()));
        let _toast_clone = use_context_provider(|| ReactiveToast(self.snapshot.toast.clone()));
        let root = Element::component(ReactiveRouteComponent {
            cursor: self.cursor.clone(),
            errors: self.errors.clone(),
            screen_lines: self.snapshot.screen_lines.clone(),
            dialog_layer: self.snapshot.dialog_layer.clone(),
        });
        root.render(area, buffer);
    }
}

impl Component for ReactiveRouteComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let route = use_context::<ReactiveRouteSnapshot>().0.clone();
        let selection = use_context::<ReactiveSelection>().0;
        let toast = use_context::<ReactiveToast>().0;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            *self.cursor.lock() = None;

            match &route {
                Route::Session { session_id } => {
                    let session_route = Element::component(ReactiveSessionRouteComponent {
                        cursor: self.cursor.clone(),
                        errors: self.errors.clone(),
                        session_id: session_id.clone(),
                    })
                    .with_key(session_id.clone());
                    session_route.render(area, buffer);
                }
                _ => {
                    let home_route = Element::component(ReactiveHomeRouteComponent {
                        cursor: self.cursor.clone(),
                        errors: self.errors.clone(),
                    })
                        .with_key("reactive-home-route");
                    home_route.render(area, buffer);
                }
            }

            {
                let mut surface = BufferSurface::new(buffer);
                render_reactive_dialog_layer_snapshot(&self.dialog_layer, &mut surface, area);
                if toast.is_visible() {
                    let toast_width = 60u16.min(area.width.saturating_sub(4));
                    let toast_height = toast.desired_height(toast_width);
                    let base_x =
                        area.x + area.width.saturating_sub(toast_width.saturating_add(2));
                    let max_x = area.x + area.width.saturating_sub(toast_width);
                    let toast_x = base_x.saturating_add(toast.slide_offset()).min(max_x);
                    let toast_area = Rect {
                        x: toast_x,
                        y: 2.min(area.height.saturating_sub(1)),
                        width: toast_width,
                        height: toast_height.min(area.height.saturating_sub(2)),
                    };
                    Element::component(toast.clone())
                        .with_key("toast")
                        .render(toast_area, surface.buffer_mut());
                }
            }

            let should_capture_screen_lines =
                selection.is_active() || selection.is_selecting();
            if should_capture_screen_lines {
                let lines = crate::ui::capture_screen_lines(buffer, area);
                self.screen_lines
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone_from(&lines);
            }
            crate::ui::apply_selection_highlight(buffer, area, &selection);
        }));

        if let Err(payload) = result {
            self.errors.store(anyhow::anyhow!(
                "reactive route render panicked: {}",
                panic_payload_message(payload.as_ref())
            ));
        }
    }
}

fn render_reactive_dialog_layer_snapshot(
    dialog_layer: &ReactiveDialogLayerSnapshot,
    surface: &mut BufferSurface<'_>,
    area: Rect,
) {
    let theme = use_context::<crate::theme::Theme>();
    if dialog_layer.show_backdrop {
        let modal_backdrop =
            ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(
                theme.background_menu,
            ));
        surface.render_widget(modal_backdrop, area);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.permission_prompt.clone())
            .with_key("dialog-permission-prompt")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.question_prompt.clone())
            .with_key("dialog-question-prompt")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.help_dialog.clone())
            .with_key("dialog-help")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.alert_dialog.clone())
            .with_key("dialog-alert")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.slash_popup.clone())
            .with_key("dialog-slash")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.command_palette.clone())
            .with_key("dialog-command-palette")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.model_select.clone())
            .with_key("dialog-model-select")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.agent_select.clone())
            .with_key("dialog-agent-select")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.status_dialog.clone())
            .with_key("dialog-status")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.session_list_dialog.clone())
            .with_key("dialog-session-list")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.session_export_dialog.clone())
            .with_key("dialog-session-export")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.session_rename_dialog.clone())
            .with_key("dialog-session-rename")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.skill_list_dialog.clone())
            .with_key("dialog-skill-list")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.mcp_dialog.clone())
            .with_key("dialog-mcp")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.timeline_dialog.clone())
            .with_key("dialog-timeline")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.fork_dialog.clone())
            .with_key("dialog-fork")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.provider_dialog.clone())
            .with_key("dialog-provider")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.recovery_action_dialog.clone())
            .with_key("dialog-recovery-action")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.skill_proposal_review_dialog.clone())
            .with_key("dialog-skill-proposal-review")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.subagent_dialog.clone())
            .with_key("dialog-subagent")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.tag_dialog.clone())
            .with_key("dialog-tag")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.theme_list_dialog.clone())
            .with_key("dialog-theme-list")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.prompt_stash_dialog.clone())
            .with_key("dialog-prompt-stash")
            .render(area, buffer);
    }
    {
        let buffer = surface.buffer_mut();
        Element::component(dialog_layer.tool_call_cancel_dialog.clone())
            .with_key("dialog-tool-call-cancel")
            .render(area, buffer);
    }
}

impl Component for ReactiveHomeRouteComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = use_context::<ReactiveAppContextHandle>().0;
        let prompt = use_context::<ReactivePromptHandle>().0;
        let home = HomeView::new(app_context);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            home.render_reactive_with_cursor(buffer, area, &prompt)
        }));

        match result {
            Ok(cursor) => {
                *self.cursor.lock() = cursor;
            }
            Err(payload) => {
                self.errors.store(anyhow::anyhow!(
                    "reactive home render panicked: {}",
                    panic_payload_message(payload.as_ref())
                ));
            }
        }
    }
}

impl Component for ReactiveSessionRouteComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = use_context::<ReactiveAppContextHandle>().0;
        let session_view = {
            Some(app_context.ensure_session_view_handle(&self.session_id))
        };
        let _session_view_handle =
            use_context_provider(|| ReactiveSessionViewHandle(session_view));

        let child = Element::component(ReactiveSessionViewComponent {
            cursor: self.cursor.clone(),
            errors: self.errors.clone(),
        });
        child.render(area, buffer);
    }
}

impl Component for ReactiveSessionViewComponent {
    fn render(&self, area: Rect, buffer: &mut Buffer) {
        let app_context = use_context::<ReactiveAppContextHandle>().0;
        let prompt = use_context::<ReactivePromptHandle>().0;
        let view = use_context::<ReactiveSessionViewHandle>().0;
        let Some(view) = view else {
            *self.cursor.lock() = None;
            return;
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            view.render_reactive_with_prompt(&app_context, buffer, area, &prompt)
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

    set_fiber_tree(FiberTree::new());
    init_render_context();
    batch::init_main_thread();
    let startup = {
        let mut app = app.lock();
        app.prepare_bridge_runtime_start(terminal.size().ok().map(Rect::from))
    };
    let app_context = startup.app_context;
    let server_event_task = startup.server_event_task;

    let result = async {
        loop {
            let now = Instant::now();
            let loop_snapshot = app.lock().bridge_loop_snapshot(now, first_frame);
            if loop_snapshot.is_exiting {
                break;
            }
            let polled_event = match loop_snapshot.wait_strategy {
                BridgeWaitStrategy::PollReady => poll_ready_relevant_event(&mut events),
                BridgeWaitStrategy::Wait { deadline } => {
                    let bridge_notified = app_context.ui_bridge_notified();
                    tokio::pin!(bridge_notified);
                    if let Some(deadline) = deadline {
                        let timeout =
                            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
                        tokio::pin!(timeout);
                        tokio::select! {
                            event = next_relevant_crossterm_event(&mut events) => event,
                            _ = &mut bridge_notified => None,
                            _ = &mut timeout => None,
                        }
                    } else {
                        tokio::select! {
                            event = next_relevant_crossterm_event(&mut events) => event,
                            _ = &mut bridge_notified => None,
                        }
                    }
                }
            };

            let resized_area = if matches!(polled_event, Some(Event::Resize(_, _))) {
                terminal.autoresize()?;
                terminal.size().ok().map(Rect::from)
            } else {
                None
            };

            let mut should_draw = first_frame;

            let max_events_per_frame = loop_snapshot.max_events_per_frame;
            let iteration = {
                let mut app = app.lock();
                app.process_bridge_iteration(
                    resized_area,
                    loop_snapshot.tick_due,
                    max_events_per_frame,
                    polled_event.as_ref(),
                    first_frame,
                )?
            };
            should_draw |= iteration.should_draw;

            // Codex-style event loop principle: ignored terminal noise must not
            // force reratui bookkeeping/render passes. If no meaningful event,
            // tick, or bridge update changed state, go back to sleep directly.
            if !should_draw {
                clear_current_event();
                continue;
            }

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.prepare_for_render();
            });
            reset_component_position_counter();
            clear_global_handlers();

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.mark_unseen_for_unmount();
            });
            let BridgeIterationOutcome {
                should_draw: _,
                reratui_event,
                reactive_root_snapshot,
            } = iteration;
            set_current_event(reratui_event.map(Arc::new));

            if let Some(error) = errors.take() {
                return Err(error);
            }

            if should_draw {
                let snapshot = reactive_root_snapshot.unwrap_or_else(|| unreachable!(
                    "reactive root snapshot must exist whenever a frame draw is requested"
                ));
                let _ = draw_app_frame_blocking(&mut terminal, snapshot, &errors)?;
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

async fn next_relevant_crossterm_event(events: &mut EventStream) -> Option<Event> {
    next_relevant_event_from_stream(events).await
}

fn poll_ready_relevant_event<S>(events: &mut S) -> Option<Event>
where
    S: Stream<Item = std::io::Result<CrosstermEvent>> + Unpin,
{
    futures::pin_mut!(events);
    let waker = futures::task::noop_waker_ref();
    let mut cx = std::task::Context::from_waker(waker);
    loop {
        match events.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(Ok(event))) => {
                if let Some(mapped) = map_crossterm_event(event) {
                    return Some(mapped);
                }
            }
            Poll::Ready(Some(Err(_))) | Poll::Ready(None) | Poll::Pending => return None,
        }
    }
}

async fn next_relevant_event_from_stream<S>(events: &mut S) -> Option<Event>
where
    S: Stream<Item = std::io::Result<CrosstermEvent>> + Unpin,
{
    poll_fn(|cx| loop {
        match Pin::new(&mut *events).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                if let Some(mapped) = map_crossterm_event(event) {
                    return Poll::Ready(Some(mapped));
                }
            }
            Poll::Ready(Some(Err(_))) | Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        }
    })
    .await
}

fn map_crossterm_event(event: CrosstermEvent) -> Option<Event> {
    match event {
        CrosstermEvent::Key(key) if is_primary_key_event(key) => Some(Event::Key(key)),
        CrosstermEvent::Key(_) => None,
        CrosstermEvent::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Moved) => None,
        CrosstermEvent::Mouse(mouse) => Some(Event::Mouse(mouse)),
        CrosstermEvent::Resize(width, height) => Some(Event::Resize(width, height)),
        CrosstermEvent::FocusGained | CrosstermEvent::FocusLost => None,
        CrosstermEvent::Paste(text) => Some(Event::Paste(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppLaunchConfig};
    use crate::event::CustomEvent;
    use crossterm::event::{
        Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
    };
    use futures::{Future, task::noop_waker_ref};
    use ratatui::{buffer::Buffer as TerminalBuffer, layout::Rect};
    use reratui::{
        clear_current_event, clear_global_handlers, clear_render_context, init_render_context,
        reset_component_position_counter, set_current_event, with_render_context_mut, FiberTree,
    };
    use std::collections::VecDeque;
    use std::io;
    use std::task::Context as TaskContext;

    #[derive(Default)]
    struct FakeEventStream {
        events: VecDeque<io::Result<CrosstermEvent>>,
    }

    impl FakeEventStream {
        fn with_events(events: Vec<io::Result<CrosstermEvent>>) -> Self {
            Self {
                events: events.into(),
            }
        }
    }

    impl Stream for FakeEventStream {
        type Item = io::Result<CrosstermEvent>;

        fn poll_next(
            mut self: Pin<&mut Self>,
            _cx: &mut TaskContext<'_>,
        ) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.events.pop_front())
        }
    }

    struct ReactiveRenderHarness {
        app: Arc<Mutex<App>>,
        errors: Arc<RuntimeErrorSink>,
        area: Rect,
        cursor: Arc<Mutex<Option<(u16, u16)>>>,
    }

    impl ReactiveRenderHarness {
        fn new(app: Arc<Mutex<App>>, area: Rect) -> Self {
            clear_fiber_tree();
            clear_render_context();
            set_fiber_tree(FiberTree::new());
            init_render_context();

            Self {
                app,
                errors: Arc::new(RuntimeErrorSink::default()),
                area,
                cursor: Arc::new(Mutex::new(None)),
            }
        }

        fn errors(&self) -> Arc<RuntimeErrorSink> {
            self.errors.clone()
        }

        fn cursor(&self) -> Arc<Mutex<Option<(u16, u16)>>> {
            self.cursor.clone()
        }

        fn render(&self, event: Option<CrosstermEvent>) {
            self.render_frame(event, false, |_| {});
        }

        fn render_with_forward_gate(&self, event: Option<CrosstermEvent>) {
            self.render_frame(event, true, |_| {});
        }

        fn render_to_string(&self, event: Option<CrosstermEvent>) -> String {
            let mut frame_text = String::new();
            self.render_frame(event, false, |buffer| {
                let width = buffer.area.width as usize;
                frame_text = buffer
                    .content
                    .chunks(width)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
            });
            frame_text
        }

        fn render_frame<F>(
            &self,
            event: Option<CrosstermEvent>,
            gate_with_forwarding: bool,
            on_buffer: F,
        ) where
            F: FnOnce(&TerminalBuffer),
        {
            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.prepare_for_render();
                tree.mark_unseen_for_unmount();
            });
            reset_component_position_counter();
            clear_global_handlers();

            let reratui_event = if gate_with_forwarding {
                let app = self.app.lock();
                if app.should_forward_current_terminal_event_to_reratui() {
                    event.map(Arc::new)
                } else {
                    None
                }
            } else {
                event.map(Arc::new)
            };
            set_current_event(reratui_event);

            let snapshot = {
                let mut app = self.app.lock();
                app.prepare_reactive_root_snapshot(self.area)
            };
            let root = Element::component(ReactiveRootComponent {
                snapshot,
                cursor: self.cursor.clone(),
                errors: self.errors.clone(),
            });
            let mut buffer = TerminalBuffer::empty(self.area);
            root.render(self.area, &mut buffer);
            on_buffer(&buffer);

            reratui::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.process_unmounts();
            });
            with_render_context_mut(|ctx| {
                ctx.begin_batch();
                let _ = ctx.end_batch();
            });
            clear_current_event();
        }
    }

    impl Drop for ReactiveRenderHarness {
        fn drop(&mut self) {
            clear_current_event();
            clear_fiber_tree();
            clear_render_context();
        }
    }

    fn scheduler_stage_event(session_id: &str, id: &str, text: &str) -> Event {
        Event::Custom(Box::new(CustomEvent::FrontendEvent(Box::new(
            agendao_server_core::frontend_events::FrontendEvent::OutputBlockAppended {
                session_id: session_id.to_string(),
                id: Some(id.to_string()),
                block: serde_json::json!({
                    "kind": "scheduler_stage",
                    "text": text,
                }),
                live_identity: None,
            },
        ))))
    }

    fn message_delta_event(session_id: &str, id: &str, text: &str) -> Event {
        Event::Custom(Box::new(CustomEvent::FrontendEvent(Box::new(
            agendao_server_core::frontend_events::FrontendEvent::OutputBlockAppended {
                session_id: session_id.to_string(),
                id: Some(id.to_string()),
                block: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": text,
                }),
                live_identity: None,
            },
        ))))
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
        let CustomEvent::FrontendEvent(event) = custom.as_ref()
        else {
            panic!("expected output block");
        };
        let agendao_server_core::frontend_events::FrontendEvent::OutputBlockAppended { block, .. } =
            event.as_ref()
        else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "new");
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

    #[test]
    fn map_crossterm_event_ignores_mouse_move() {
        let event = CrosstermEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Moved,
            column: 7,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::empty(),
        });

        assert!(map_crossterm_event(event).is_none());
    }

    #[test]
    fn map_crossterm_event_keeps_mouse_clicks() {
        let event = CrosstermEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 7,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::empty(),
        });

        assert!(matches!(map_crossterm_event(event), Some(Event::Mouse(_))));
    }

    #[test]
    fn map_terminal_event_for_reratui_keeps_mouse_scroll() {
        let app = App::new_with_config(AppLaunchConfig::default())
            .expect("app should initialize");
        let event = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 9,
            row: 4,
            modifiers: crossterm::event::KeyModifiers::empty(),
        });

        let mapped = app.current_reratui_event(Some(&event));
        assert!(matches!(
            mapped,
            Some(CrosstermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 9,
                row: 4,
                ..
            }))
        ));
    }

    #[test]
    fn map_crossterm_event_ignores_focus_noise() {
        assert!(map_crossterm_event(CrosstermEvent::FocusGained).is_none());
        assert!(map_crossterm_event(CrosstermEvent::FocusLost).is_none());
    }

    #[test]
    fn next_relevant_crossterm_event_skips_mouse_move_noise() {
        let mut events = FakeEventStream::with_events(vec![
            Ok(CrosstermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            })),
            Ok(CrosstermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 2,
                row: 3,
                modifiers: crossterm::event::KeyModifiers::empty(),
            })),
        ]);
        let waker = noop_waker_ref();
        let mut cx = TaskContext::from_waker(waker);
        let mut future = Box::pin(next_relevant_event_from_stream(&mut events));

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Some(Event::Mouse(mouse))) => {
                assert!(matches!(mouse.kind, MouseEventKind::Down(_)));
                assert_eq!(mouse.column, 2);
                assert_eq!(mouse.row, 3);
            }
            other => panic!("expected mouse click after filtering noise, got {other:?}"),
        }
    }

    #[test]
    fn poll_ready_relevant_event_returns_immediate_key_without_waiting() {
        let mut events = FakeEventStream::with_events(vec![Ok(CrosstermEvent::Key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty()),
        ))]);

        let ready = poll_ready_relevant_event(&mut events);
        assert!(matches!(
            ready,
            Some(Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                ..
            }))
        ));
    }

    #[test]
    fn poll_ready_relevant_event_skips_mouse_move_noise() {
        let mut events = FakeEventStream::with_events(vec![
            Ok(CrosstermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            })),
            Ok(CrosstermEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::empty(),
            ))),
        ]);

        let ready = poll_ready_relevant_event(&mut events);
        assert!(matches!(
            ready,
            Some(Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            }))
        ));
    }

    #[test]
    fn reactive_home_prompt_submit_exposes_cursor() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        harness.render(None);
        assert!(
            harness.cursor().lock().is_some(),
            "home route should expose cursor"
        );

        let submit_prompt = |event: CrosstermEvent| {
            let mapped_event = map_crossterm_event(event.clone());
            if let Some(mapped) = mapped_event {
                let mut app = app.lock();
                app.process_event(&mapped).expect("event should process");
            }
            harness.render(Some(event.clone()));
            {
                let mut app = app.lock();
                app.drain_pending_events(32)
                    .expect("pending reactive events should drain");
            }
        };

        submit_prompt(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('H'),
            KeyModifiers::empty(),
        )));
        submit_prompt(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::empty(),
        )));
        submit_prompt(CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::empty(),
        )));
        assert!(
            harness.errors().take().is_none(),
            "reactive home render should not panic"
        );
    }

    #[test]
    fn reactive_home_char_input_updates_app_prompt_after_drain() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        harness.render(None);
        let event = CrosstermEvent::Key(KeyEvent::new(
            KeyCode::Char('H'),
            KeyModifiers::empty(),
        ));
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("event should process");
        }
        let rendered = harness.render_to_string(Some(event));
        {
            let mut app = app.lock();
            app.drain_pending_events(32)
                .expect("pending reactive events should drain");
            assert_eq!(app.prompt_handle().get_input(), "H");
        }
        assert!(
            rendered.contains('H'),
            "rendered home frame should contain typed text"
        );

        assert!(
            harness.errors().take().is_none(),
            "reactive home render should not panic"
        );
    }

    #[test]
    fn globally_consumed_selection_escape_is_not_forwarded_to_reratui() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        {
            let mut app = app.lock();
            app.process_event(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            }))
            .expect("seed selection down");
            app.process_event(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            }))
            .expect("seed selection up");
        }

        let event = CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("event should process");
            assert!(
                !app.selection_snapshot().is_active(),
                "global handler should clear selection"
            );
            assert!(
                !app.should_forward_current_terminal_event_to_reratui(),
                "consumed selection escape must not reach reratui prompt"
            );
        }
        harness.render_with_forward_gate(Some(event));

        {
            let app = app.lock();
            let drained = app.context_handle().drain_ui_events(8);
            assert!(
                drained.is_empty(),
                "reratui prompt must not emit follow-up interrupt for consumed escape"
            );
        }

        assert!(
            harness.errors().take().is_none(),
            "reactive render should not panic"
        );
    }

    #[test]
    fn globally_consumed_selection_ctrl_c_is_not_forwarded_to_reratui() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        {
            let mut app = app.lock();
            app.process_event(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            }))
            .expect("seed selection down");
            app.process_event(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: crossterm::event::KeyModifiers::empty(),
            }))
            .expect("seed selection up");
        }

        let event =
            CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("event should process");
            assert!(
                !app.selection_snapshot().is_active(),
                "global handler should finalize copy and clear selection"
            );
            assert!(
                !app.should_forward_current_terminal_event_to_reratui(),
                "consumed selection copy must not reach reratui prompt"
            );
        }
        harness.render_with_forward_gate(Some(event));

        {
            let app = app.lock();
            let drained = app.context_handle().drain_ui_events(8);
            assert!(
                drained.is_empty(),
                "reratui prompt must not emit exit for consumed selection ctrl-c"
            );
        }

        assert!(
            harness.errors().take().is_none(),
            "reactive render should not panic"
        );
    }

    #[test]
    fn help_dialog_escape_is_not_forwarded_to_reratui_prompt() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        {
            let mut app = app.lock();
            app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
                action: agendao_command::UiActionId::ShowHelp,
            })))
            .expect("open help dialog through public action");
        }

        let event = CrosstermEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("event should process");
            assert!(
                !app.context_handle().is_dialog_open(crate::state::DialogSlot::Help),
                "global dialog handler should close help dialog"
            );
            assert!(
                !app.should_forward_current_terminal_event_to_reratui(),
                "consumed dialog escape must not reach reratui prompt"
            );
        }
        harness.render_with_forward_gate(Some(event));

        {
            let app = app.lock();
            let drained = app.context_handle().drain_ui_events(8);
            assert!(
                drained.is_empty(),
                "reratui prompt must not emit follow-up interrupt for consumed dialog escape"
            );
        }

        assert!(
            harness.errors().take().is_none(),
            "reactive render should not panic"
        );
    }

    #[test]
    fn reactive_slash_popup_text_key_is_forwarded_to_reratui_prompt() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        {
            let mut app = app.lock();
            let mut prompt = app.prompt_handle();
            prompt.set_input("/he".to_string());
            app.process_event(&Event::Custom(Box::new(CustomEvent::PromptEdited {
                prompt: Box::new(prompt),
            })))
            .expect("seed slash popup through prompt authority");
            assert!(
                app.context_handle()
                    .is_dialog_open(crate::state::DialogSlot::SlashPopup),
                "slash popup should be open before forwarding text key"
            );
        }

        let event = CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("event should process");
            assert!(
                app.should_forward_current_terminal_event_to_reratui(),
                "reactive slash popup text input must remain visible to reratui prompt"
            );
        }
        harness.render_with_forward_gate(Some(event));

        {
            let app = app.lock();
            let drained = app.context_handle().drain_ui_events(8);
            assert!(
                drained.iter().any(|event| matches!(
                    event,
                    Event::Custom(custom)
                        if matches!(
                            custom.as_ref(),
                            CustomEvent::PromptEdited { prompt }
                                if prompt.get_input() == "/hel"
                        )
                )),
                "reratui prompt should emit PromptEdited with the continued slash input"
            );
        }

        assert!(
            harness.errors().take().is_none(),
            "reactive render should not panic"
        );
    }

    #[test]
    fn dialog_mouse_down_is_not_forwarded_to_reratui() {
        let app = Arc::new(Mutex::new(
            App::new_with_config(AppLaunchConfig::default()).expect("app should initialize"),
        ));
        let area = Rect::new(0, 0, 100, 30);
        let harness = ReactiveRenderHarness::new(app.clone(), area);

        {
            let mut app = app.lock();
            app.process_event(&Event::Custom(Box::new(CustomEvent::UiActionRequested {
                action: agendao_command::UiActionId::OpenModelList,
            })))
            .expect("open model dialog through public action");
        }

        let event = CrosstermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::empty(),
        });
        if let Some(mapped) = map_crossterm_event(event.clone()) {
            let mut app = app.lock();
            app.process_event(&mapped).expect("mouse event should process");
            assert!(
                !app.should_forward_current_terminal_event_to_reratui(),
                "consumed dialog mouse down must not reach reratui"
            );
        }
        harness.render_with_forward_gate(Some(event));

        {
            let app = app.lock();
            let drained = app.context_handle().drain_ui_events(8);
            assert!(
                drained.is_empty(),
                "reratui tree must not emit follow-up events for consumed dialog click"
            );
        }

        assert!(
            harness.errors().take().is_none(),
            "reactive render should not panic"
        );
    }
}
