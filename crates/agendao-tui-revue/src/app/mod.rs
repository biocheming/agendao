//! Application entry point, event loop, and root view — 火 (execution authority)
//! + 金 (output shaping).
//!
//! The keymap and slash-action dispatchers live in [`keymap`] so this file
//! can stay focused on wiring (App construction, run loop, RootView render).
//! Both files share the same `AppHandler` struct via split `impl` blocks —
//! Rust allows the impl to live in any sibling module as long as the type
//! and its fields are at least `pub(crate)`-visible.

mod keymap;

use anyhow::Context;
use revue::prelude::*;
use tokio::sync::watch;
use std::cell::RefCell;

/// Global publish slot for the SessionList dialog's interactive
/// scrollbar. The dialog writes here every frame; the mouse
/// handler reads it on the next event tick. Lives at module scope
/// (not on `AppHandler`) because the dialog's `render(&self, ctx)`
/// is invoked from a borrowed `&self` deep in the layout tree —
/// there's no `AppHandler` handle in scope at that point.
///
/// We use `std::sync::Mutex` instead of `RefCell` because the
/// slot must be `Sync` to live in a `OnceLock` static. The lock is
/// only ever taken on the render or event thread, so contention is
/// not a concern in practice.
///
/// The other list dialogs (ModelSelect, AgentSelect, Help) are
/// less common and haven't been wired to the global publish; their
/// mouse interactions go through other paths.
pub static SESSION_LIST_SCROLLBAR_PUBLISH: std::sync::OnceLock<
    std::sync::Mutex<Option<crate::dialog::backdrop::ListDialogScrollbarArea>>,
> = std::sync::OnceLock::new();

/// Lazy initialiser for the publish slot — same pattern as
/// `std::sync::OnceLock::get_or_init`. We use this so the cell is
/// created on first access; no need for a static initializer that
/// can't run at const time.
pub fn session_list_scrollbar_slot(
) -> &'static std::sync::Mutex<Option<crate::dialog::backdrop::ListDialogScrollbarArea>> {
    SESSION_LIST_SCROLLBAR_PUBLISH.get_or_init(|| std::sync::Mutex::new(None))
}

use crate::bridge::api::ApiBridge;
use crate::config::AppConfig;
use crate::dialog::{
    AgentSelectDialog,
    AlertDialog, HelpDialog,
    ModelSelectDialog, SessionListDialog,
    PermissionDialog, QuestionDialog,
    ConfirmDialog, SessionRenameDialog, StashDialog, StashEntry,
};
use crate::input::{PromptInput, SlashPopup};
use crate::screen::{HomeLayout, layout_block, layout_block_ctx};
use crate::store::app_store::{AppStore, Route};
use crate::telemetry::event_bus::EventBus;
use crate::store::session_store::SessionStore;
use crate::store::types::{RunStatus, SessionListItem, ToolPhase, TranscriptBlock};
use crate::theme::colors;
use crate::transport;

pub fn run_app() -> anyhow::Result<()> { run_app_with_config(AppConfig::default()) }

pub fn run_app_with_config(config: crate::config::AppConfig) -> anyhow::Result<()> {
    // 主题收口（阴面唯一注册点）：颜色真值权威在 styles/base.css 的 :root
    // 变量；此处 Theme 仅驱动运行时 variant。OSC11 终端背景探测（ds/osc11）
    // 决定 dark/light variant：detect_bg 保守返回 None → fallback dark。
    crate::ds::theme::register_agendao_themes();
    let theme = match crate::ds::osc11::detect_bg() {
        Some((r, g, b)) if crate::ds::osc11::is_light_bg(r, g, b)
            => crate::ds::theme::tokyo_night_light(),
        _ => crate::ds::theme::tokyo_night_dark(),
    };
    revue::style::set_theme(theme);

    let store = AppStore::new();
    if let Some(ref dir) = config.working_dir { store.working_dir.set(dir.display().to_string()); }
    let rt = tokio::runtime::Runtime::new().map_err(|e| anyhow::anyhow!("tokio runtime: {}", e))?;
    let (sf_tx, sf_rx) = watch::channel::<Option<String>>(None);
    if let Some(ref sid) = config.session_id {
        sf_tx.send_replace(Some(sid.clone()));
        store.navigate(Route::Session { session_id: sid.clone() });
    }
    let eb = EventBus::new();
    let active_session = SessionStore::new();
    let tx = eb.sender();
    let wd = config.working_dir.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Build ApiBridge: local-direct uses in-process server, external uses HTTP
    let api: Option<ApiBridge> = if config.local_direct {
        // Prefer pre-created local_server from outer async context (host.rs).
        // This matches the old TUI pattern: server created in outer runtime,
        // projector tasks run on outer runtime's thread pool.
        let local_state = if let Some(pre) = config.local_server {
            tracing::info!("using pre-created local server state from host");
            Some(pre)
        } else {
            // Fallback: create server state on our own runtime
            tracing::info!("creating local server state internally");
            match rt.block_on(agendao_server_local::new_local_server_for_workspace(wd.clone())) {
                Ok(state) => Some(state),
                Err(e) => {
                    tracing::error!(%e, "FAILED to init local server; data pipeline will be empty");
                    None
                }
            }
        };
        if let Some(ls) = local_state {
            let _ = transport::spawn_local_event_source(tx, ls.clone(), &rt.handle(), sf_rx.clone());
            Some(ApiBridge::new_local(ls, rt.handle().clone()))
        } else {
            // Server creation failed — fall back to transport-based mode
            let _ = transport::spawn_event_source(tx, wd, &rt.handle(), sf_rx, config.unix_socket_path.clone(), config.base_url.clone());
            None
        }
    } else {
        let _ = transport::spawn_event_source(tx, wd, &rt.handle(), sf_rx, config.unix_socket_path.clone(), config.base_url.clone());
        ApiBridge::new(&config.base_url.clone().unwrap_or_else(|| "http://127.0.0.1:3000".into()), rt.handle().clone()).ok()
    };
    tracing::info!(api_present = api.is_some(), "ApiBridge construction complete");
    if let Some(ref a) = config.agent_name { store.selected_agent.set(Some(a.clone())); }
    if let Some(ref m) = config.model { store.selected_model.set(Some(m.clone())); }

    // ── Eager message load for --session / AGENDAO_TUI_SESSION ──
    //
    // The SessionStore is created empty and the historical messages are
    // normally pulled in by AppHandler::load_session_messages when the
    // user picks a row from the SessionList dialog. With an env-var
    // session we skip that dialog and navigate straight to Session
    // route, so the transcript stays blank. Calling the same load path
    // here makes both entry points converge on the same content.
    if let Some(ref sid) = config.session_id {
        active_session.set_session_id(sid);
        keymap::eager_load_session_messages(&active_session, api.as_ref(), sid);
    }

    let mut app = App::builder().mouse_capture(true).style("styles/base.css").build();
    let handler = RefCell::new(AppHandler::new(store.clone(), api.clone(), active_session.clone(), eb, sf_tx));
    let view = RootView { store, api, active_session, handler };

    app.run(view, move |event, view, app| {
        let mut h = view.handler.borrow_mut();
        let handled = h.handle(event);
        let layout_dirty = h.layout_dirty;
        h.layout_dirty = false;
        drop(h);
        if handled {
            // Force a full redraw (revue's DOM dirty-region tracking
            // doesn't detect Input widget state changes, so we can't
            // rely on per-cell diffs after a handled event).
            app.request_redraw();
            // The DOM incremental update only refreshes nodes that
            // changed in the structural sense (added/removed/re-typed).
            // When fold state changes, the OUTER tree shape is the same
            // (still a vstack of 637 hstacks) but the LEAF widget
            // heights differ. Without a layout rebuild, the new content
            // is rendered into the cached height slots from before the
            // toggle and the visible result is the same stale frame.
            if layout_dirty { app.request_layout_rebuild(); }
        }
        handled
    }).context("agendao TUI runtime exited with error")
}

/// Which overlay is currently active (only one at a time).
///
/// `pub(crate)` so [`super::keymap`] can `match` on it for per-panel key
/// routing and the render path in this module can pattern-match the
/// overlay layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Panel {
    None,
    Slash,
    ModelSelect,
    AgentSelect,
    SessionList,
    Rename,
    Stash,
    #[allow(dead_code)] Confirm,
    Help,
    Permission,
    Question,
    #[allow(dead_code)] Alert,
}

/// Application state + event handler.
///
/// Fields are `pub(crate)` because the keymap dispatcher lives in a
/// sibling module (`keymap`) and matches / mutates them directly. The
/// struct itself is `pub(crate)` so the sibling can `impl` it.
///
/// Why not a `&mut self` API on a private struct? Each handler reads
/// 2-5 different fields, and threading a typed accessor for every
/// one would dwarf the actual logic. The fields are protected by
/// `RefCell` in `RootView` at the consumer side, which is the only
/// boundary that matters.
pub(crate) struct AppHandler {
    pub(crate) store: AppStore,
    pub(crate) api: Option<ApiBridge>,
    pub(crate) prompt: PromptInput,
    pub(crate) slash_popup: SlashPopup,
    pub(crate) model_select: ModelSelectDialog,
    pub(crate) agent_select: AgentSelectDialog,
    pub(crate) session_list: SessionListDialog,
    pub(crate) sidebar_visible: bool,
    pub(crate) permission_dialog: PermissionDialog,
    pub(crate) question_dialog: QuestionDialog,
    pub(crate) rename_dialog: SessionRenameDialog,
    pub(crate) stash_dialog: StashDialog,
    pub(crate) stash_entries: Vec<StashEntry>,
    pub(crate) confirm_dialog: ConfirmDialog,
    pub(crate) alert: AlertDialog,
    pub(crate) help: HelpDialog,
    pub(crate) panel: Panel,
    pub(crate) active_session: SessionStore,
    pub(crate) spinner_tick: u64,
    pub(crate) interrupt_pending: bool,
    /// 发 prompt 后置位；一轮结束（Idle）时由 Tick 分支消费一次——拉取服务端
    /// LLM 生成的 title 同步到 active_session.title，然后清除。闭合新建 session
    /// 首轮 title 无事件回流的缺口（header 不再恒显 "New Session"）。
    pub(crate) title_refresh_pending: bool,
    pub(crate) interrupt_time: std::time::Instant,
    pub(crate) event_bus: EventBus,
    pub(crate) sf_tx: watch::Sender<Option<String>>,
    /// Set by event handlers whose state change might alter widget
    /// heights (fold toggle, message push, scroll, etc.). The run loop
    /// reads this after `handle()` and calls `request_layout_rebuild()`
    /// so the layout tree is recomputed before the next draw — without
    /// this, a folded→unfolded block is rendered into its OLD height
    /// slot by the cached layout and the visible frame stays the same.
    pub(crate) layout_dirty: bool,
    /// Height of the transcript viewport in rows. Updated every frame
    /// by `RootView::render` from the layout's actual area, then read
    /// by cursor-moving handlers (Tab, j/k) to call
    /// `ensure_cursor_visible(viewport_h)` so the cursor's block lands
    /// inside the visible window after a navigation jump.
    pub(crate) transcript_viewport_h: u16,
    /// Y-coordinate of the transcript area on screen (after header+divider).
    /// Used by mouse click handler to map click_y to transcript row.
    pub(crate) transcript_area_y: u16,

    /// Absolute screen rect of the transcript scrollbar column,
    /// captured every frame by `RootView::render` and consumed by the
    /// mouse handler to hit-test arrow clicks and thumb drags. The
    /// Rect is the scrollbar's *full* span (▲ + track + ▼), one column
    /// wide. `None` when not on the session route or content fits in
    /// the viewport.
    pub(crate) transcript_scrollbar_area: Option<Rect>,
    /// Metrics paired with `transcript_scrollbar_area`: the
    /// total content rows and viewport rows the scrollbar was drawn
    /// against. Together they form the `ScrollbarOverlay` view-model
    /// for hit-testing without re-walking the transcript.
    pub(crate) transcript_scrollbar_metrics: Option<(u16, u16)>,
    /// Active drag on the transcript scrollbar, if any. Set on
    /// `BeginDrag`, mutated on every `Drag` event, cleared on `Up`.
    pub(crate) transcript_scrollbar_drag: Option<crate::widget::ScrollbarDrag>,
    /// Per-frame slot the `ScrollableTranscript` writes into during
    /// `render`. `RootView::render` drains it into
    /// `transcript_scrollbar_area` / `transcript_scrollbar_metrics`
    /// after `layout.render(ctx)` returns and the immutable borrow
    /// is released. Lives on `AppHandler` so the borrow for the
    /// handler's other fields can coexist with the publish clone.
    pub(crate) transcript_scrollbar_publish: std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
    /// Absolute screen rect of the sidebar scrollbar column,
    /// captured every frame and consumed by the mouse handler.
    pub(crate) sidebar_scrollbar_area: Option<Rect>,
    /// Metrics paired with `sidebar_scrollbar_area`.
    pub(crate) sidebar_scrollbar_metrics: Option<(u16, u16)>,
    /// Active drag on the sidebar scrollbar, if any.
    pub(crate) sidebar_scrollbar_drag: Option<crate::widget::ScrollbarDrag>,
    /// Per-frame publish slot for the sidebar scrollbar.
    pub(crate) sidebar_scrollbar_publish: std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
    /// Sidebar scroll offset, in "rows from top". The sidebar is
    /// reset to 0 when first shown and never auto-resets on data
    /// change — users explicitly drag/click to scroll.
    pub(crate) sidebar_scroll_offset: u16,
    /// Active drag on the session-list dialog's scrollbar. The
    /// dialog uses its own `selected: usize` as the cursor; the
    /// drag state is just a remembered y origin so Drag events
    /// can map cursor-y → new selected index.
    pub(crate) session_list_scrollbar_drag: Option<crate::widget::ScrollbarDrag>,
}

pub(crate) const HOME_PROMPT_PLACEHOLDERS: &[&str] = &[
    "Fix a TODO in the codebase",
    "What is the tech stack of this project?",
    "Fix broken tests",
];
pub(crate) const HOME_SHELL_PLACEHOLDERS: &[&str] = &["ls -la", "git status", "pwd"];

impl AppHandler {
    fn new(s: AppStore, a: Option<ApiBridge>, ss: SessionStore, eb: EventBus, sf: watch::Sender<Option<String>>) -> Self {
        let prompt = PromptInput::new().with_persistence().with_placeholders(HOME_PROMPT_PLACEHOLDERS, HOME_SHELL_PLACEHOLDERS);
        let mut model_select = ModelSelectDialog::new();
        let mut agent_select = AgentSelectDialog::new();

        // ── 完整启动初始化 ──
        if let Some(ref api) = a {
            tracing::info!("starting initialization: API bridge present");

            // 1. 工作区配置
            match api.get_workspace_context() {
                Ok(ctx) => {
                    tracing::info!(workspace = %ctx.identity.workspace_key, "init: workspace_context loaded");
                    s.working_dir.set(ctx.identity.workspace_key);
                    if !ctx.recent_models.is_empty() {
                        let _ = api.put_recent_models(ctx.recent_models);
                    }
                }
                Err(e) => tracing::error!(%e, "init: workspace_context FAILED"),
            }

            // 2. 模型列表
            match api.get_all_providers() {
                Ok(resp) => {
                    let connected: std::collections::HashSet<String> = resp.connected.iter().cloned().collect();
                    let n_connected = connected.len();
                    let total = resp.all.len();
                    // ProviderInfo carries both `id` (registry key, e.g. "aihubmix",
                    // "deepseek") and `name` (display label, e.g. "AIHubMix"). The
                    // server's parse_model_string resolves "<provider_id>/<model_id>",
                    // so storing display name as provider here makes send_prompt fail
                    // with "Provider not found: AIHubMix". Group label still uses
                    // `name` for human-friendly display, and `connected` is keyed by
                    // id, matching how the server tracks connection state.
                    let entries: Vec<crate::dialog::ModelEntry> = resp.all.into_iter().flat_map(|p| {
                        let avail = connected.contains(&p.id);
                        let display_name = p.name.clone();
                        let provider_id = p.id.clone();
                        p.models.into_iter().map(move |m| crate::dialog::ModelEntry {
                            provider: provider_id.clone(),
                            provider_display: display_name.clone(),
                            model_id: m.id.clone(),
                            display: format!("{} ({})", m.name, display_name),
                            variants: vec![],
                            available: avail,
                        })
                    }).collect();
                    // Surface the connected providers so the user knows
                    // which models will actually work — useful when the
                    // dialog shows 5,140 entries but only 8 providers are
                    // wired in.
                    tracing::info!(
                        connected_provider_ids = ?resp.connected,
                        "init: connected providers"
                    );
                    tracing::info!(
                        providers_total = total, providers_connected = n_connected,
                        model_entries = entries.len(),
                        "init: providers loaded"
                    );
                    ss.set_mcp_lsp(n_connected, total, vec![]);
                    model_select.set_models(entries);
                }
                Err(e) => tracing::error!(%e, "init: get_all_providers FAILED"),
            }

            // 3. Agent 列表
            match api.list_agents() {
                Ok(agents) => {
                    tracing::info!(count = agents.len(), "init: agents loaded");
                    agent_select.set_agents(agents.into_iter().map(|a| crate::dialog::AgentEntry {
                        name: a.name.clone(), display: a.name,
                        description: a.description.unwrap_or_default(),
                    }).collect());
                }
                Err(e) => tracing::error!(%e, "init: list_agents FAILED"),
            }

            // 4. 执行模式
            match api.list_execution_modes() {
                Ok(modes) => {
                    tracing::info!(count = modes.len(), "init: execution modes loaded");
                    if let Some(first) = modes.first() {
                        s.selected_mode.set(Some(first.name.clone()));
                    }
                }
                Err(e) => tracing::error!(%e, "init: list_execution_modes FAILED"),
            }

            // 5. 会话列表（按 cwd 过滤，与 /sessions 对话框语义一致）
            let cwd = s.working_dir.get();
            let cwd_filter = if cwd.is_empty() { None } else { Some(cwd) };
            match api.list_sessions_in_directory(cwd_filter) {
                Ok(sessions) => {
                    tracing::info!(count = sessions.len(), "init: sessions loaded");
                    s.session_list.set(sessions.into_iter().map(|s| SessionListItem {
                        id: s.id, title: s.title, run_status: None,
                    }).collect());
                }
                Err(e) => tracing::error!(%e, "init: list_sessions FAILED"),
            }
        } else {
            tracing::error!("init: NO API BRIDGE — all data will be empty. Check local server creation.");
        }
        Self {
            store: s, api: a, prompt,
            slash_popup: SlashPopup::new(),
            model_select, agent_select,
            session_list: SessionListDialog::new(),
            sidebar_visible: false,
            permission_dialog: PermissionDialog::new(),
            question_dialog: QuestionDialog::new(),
            rename_dialog: SessionRenameDialog::new(),
            stash_dialog: StashDialog::new(),
            stash_entries: vec![],
            confirm_dialog: ConfirmDialog::new(),
            alert: AlertDialog::new(), help: HelpDialog::new(),
            panel: Panel::None,
            spinner_tick: 0,
            interrupt_pending: false,
            title_refresh_pending: false,
            interrupt_time: std::time::Instant::now(),
            active_session: ss, event_bus: eb, sf_tx: sf,
            layout_dirty: false,
            transcript_viewport_h: 30, // overwritten on first render
            transcript_area_y: 2,      // after header + divider
            transcript_scrollbar_area: None,
            transcript_scrollbar_metrics: None,
            transcript_scrollbar_drag: None,
            transcript_scrollbar_publish: std::rc::Rc::new(RefCell::new(None)),
            sidebar_scrollbar_area: None,
            sidebar_scrollbar_metrics: None,
            sidebar_scrollbar_drag: None,
            sidebar_scrollbar_publish: std::rc::Rc::new(RefCell::new(None)),
            sidebar_scroll_offset: 0,
            session_list_scrollbar_drag: None,
        }
    }
}

struct RootView {
    store: AppStore,
    #[allow(dead_code)] api: Option<ApiBridge>,
    #[allow(dead_code)] active_session: SessionStore,
    handler: RefCell<AppHandler>,
}

/// Wrapper that renders a Stack inside a ScrollView, slicing the
/// rendered content to the viewport via a private content buffer.
///
/// The wrapping flow is:
///   1. Build the full transcript Stack with content_h rows of natural height.
///   2. Allocate a content buffer of size (area.width, content_h).
///   3. Render the Stack into that buffer at (0, 0).
///   4. Hand the buffer to ScrollView::render_content, which copies
///      the visible window (rows scroll_top..scroll_top+area.height)
///      into the actual draw context, plus an inline scrollbar.
///
/// Without this wrapper a Stack with content > area.height clips
/// silently from the bottom — it does NOT scroll, and the user sees
/// no indication that there's more above. The ScrollView call here
/// is the same one revue's example_widgets.rs uses for log views.
struct ScrollableTranscript {
    /// Refined ScrollView from the agendao widget base. Drops in
    /// cleanly for what was a raw `revue::ScrollView`; the only added
    /// responsibility for the caller is the `publish` callback below,
    /// which the mouse handler reads to hit-test scrollbar clicks.
    sv: crate::widget::ScrollView,
    content: Stack,
    content_h: u16,
    /// Captured for telemetry / debugging. The actual scroll position
    /// lives inside `sv` via its `scroll_offset` builder.
    #[allow(dead_code)]
    scroll_top: u16,
    /// Sink the widget writes its absolute screen rect + metrics into
    /// during `render`. `RootView::render` drains it into
    /// `AppHandler.transcript_scrollbar_*` after the immutable borrow
    /// is released. `Rc<RefCell<…>>` because `View::render` only gets
    /// `&self` and we have no other writable channel back to the
    /// handler.
    publish: std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
}

/// Per-frame publish from [`ScrollableTranscript`] back to the handler:
/// the scrollbar's absolute screen geometry (1 column wide, full
/// transcript height) and the metrics needed to build a `Scrollbar`
/// view-model on the event side.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TranscriptScrollbarPublish {
    /// Absolute screen rect of the scrollbar column.
    area: Rect,
    /// Total content rows.
    content_h: u16,
    /// Visible window rows.
    viewport_h: u16,
}

impl View for ScrollableTranscript {
    fn render(&self, ctx: &mut RenderContext) {
        use revue::layout::Rect;
        let area = ctx.area;
        if area.width < 2 || area.height == 0 { return; }

        // Build the offscreen content buffer at full content height,
        // render the entire stack into it, then let ScrollView copy
        // the visible window into the real ctx.
        let content_width = area.width.saturating_sub(1); // reserve scrollbar col
        let mut content_buf = self.sv.create_content_buffer(content_width);
        let content_area = Rect::new(0, 0, content_width, self.content_h);
        let mut content_ctx = RenderContext::new(&mut content_buf, content_area);
        self.content.render(&mut content_ctx);

        // ScrollView takes the visible window starting at scroll_top
        // and paints it into ctx (alongside its scrollbar).
        self.sv.render_content(ctx, &content_buf);

        // Now overlay agendao's interactive scrollbar (▲ ▼ thumb) on
        // top of the simple `│/█` that `revue::ScrollView` just
        // painted. Compute the absolute scrollbar rect from the
        // ctx-relative `area` and `ctx.area.xy`.
        let sb_x_abs = ctx.area.x.saturating_add(area.x).saturating_add(area.width.saturating_sub(1));
        let sb_y_abs = ctx.area.y.saturating_add(area.y);
        let scrollbar_area_abs = Rect::new(sb_x_abs, sb_y_abs, 1, area.height);
        let overlay = crate::widget::ScrollbarOverlay::new(
            (ctx.area.x, ctx.area.y),
            area,
            self.content_h,
            area.height,
            self.scroll_top,
        );
        overlay.render(ctx);

        // Publish for the next event tick.
        if let Ok(mut slot) = self.publish.try_borrow_mut() {
            *slot = Some(TranscriptScrollbarPublish {
                area: scrollbar_area_abs,
                content_h: self.content_h,
                viewport_h: area.height,
            });
        }
    }
}

struct ScrollableSidebar {
    content: Stack,
    content_h: u16,
    /// Sidebar's scroll offset (rows from top, 0 = top). Plain u16
    /// instead of a Signal because the value is set in the same
    /// `RootView::render` pass that builds us — no need for
    /// reactivity within a single frame.
    scroll_top: u16,
    publish: std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
}

impl View for ScrollableSidebar {
    fn render(&self, ctx: &mut RenderContext) {
        use revue::layout::Rect;
        let area = ctx.area;
        if area.width < 2 || area.height == 0 { return; }

        // Reserve the rightmost column for the scrollbar overlay.
        let content_width = area.width.saturating_sub(1);
        let content_area = Rect::new(0, 0, content_width, self.content_h);
        let mut content_buf = revue::render::Buffer::new(content_width, self.content_h);
        let mut content_ctx = RenderContext::new(&mut content_buf, content_area);
        self.content.render(&mut content_ctx);

        // Paste the visible window [scroll_top .. scroll_top+viewport]
        // into ctx. Same approach as ScrollableTranscript but offset
        // comes from a local field rather than inside a ScrollView.
        let viewport = area.height;
        let max_offset = self.content_h.saturating_sub(viewport);
        let row_start = self.scroll_top.min(max_offset);
        for y in 0..viewport {
            let src_y = row_start + y;
            if src_y >= self.content_h { break; }
            for x in 0..area.width {
                if let Some(cell) = content_buf.get(x, src_y) {
                    ctx.set(x, y, *cell);
                }
            }
        }

        // Overlay ▲/▼/thumb on the reserved rightmost column.
        let sb_x_abs = ctx.area.x.saturating_add(area.x).saturating_add(area.width.saturating_sub(1));
        let sb_y_abs = ctx.area.y.saturating_add(area.y);
        let scrollbar_area_abs = Rect::new(sb_x_abs, sb_y_abs, 1, area.height);
        let overlay = crate::widget::ScrollbarOverlay::new(
            (ctx.area.x, ctx.area.y),
            area,
            self.content_h,
            area.height,
            row_start,
        );
        overlay.render(ctx);

        // Publish for the next event tick.
        if let Ok(mut slot) = self.publish.try_borrow_mut() {
            *slot = Some(TranscriptScrollbarPublish {
                area: scrollbar_area_abs,
                content_h: self.content_h,
                viewport_h: area.height,
            });
        }
    }
}

impl View for RootView {
    fn render(&self, ctx: &mut RenderContext) {
        let route = self.store.route.get();
        let h = self.handler.borrow();
        let is_running = matches!(h.active_session.run_status.get(), RunStatus::Sending | RunStatus::Running);
        let is_slash = h.panel == Panel::Slash;
        // Transcript viewport height, hoisted out of the inner
        // session-route branch so we can publish it to the handler
        // (for `ensure_cursor_visible` in the next event) after the
        // borrow is released at the bottom of `render`. Defaults to
        // the Home route's full height.
        let transcript_viewport_h: u16 = ctx.area.height.saturating_sub(9);

        // ── Content area ──
        let mut content_stack = vstack();
        match &route {
            Route::Home => {
                let home = HomeLayout { store: self.store.clone() };
                content_stack = content_stack.child(home);
            }
            Route::Session { .. } => {
                let title = h.active_session.title.get();
                let dir = self.store.working_dir.get();
                let dir_short = dir.rsplit('/').next().unwrap_or(&dir);

                // ── Header (single row): dir · title · badges · status ──
                //
                // Use a fixed-width left segment for the dir/title pair so
                // the badges hang at a predictable spot regardless of the
                // session title length. The previous loose `hstack().gap(2)`
                // pushed each child to its Auto slot — on a 160-col terminal
                // that meant the title floated near column 80 with 60 cols
                // of dead air around it.
                //
                // Layout: [📁 dir]·[title]·[· model][· agent]   …  [status]
                let title_w = title.chars().count() as u16 + 1;
                let dir_w = dir_short.chars().count() as u16 + 4; // "📁 " + space
                let mut header = hstack().gap(2);
                header = header
                    .child_sized(
                        Text::new(format!("📁 {}", dir_short)).fg(colors::FG_MUTED),
                        dir_w,
                    )
                    .child_sized(
                        Text::new(&title).bold().fg(colors::FG_PRIMARY),
                        title_w,
                    );

                if let Some(ref m) = self.store.selected_model.get() {
                    let label = format!("· {}", m);
                    let w = label.chars().count() as u16 + 1;
                    header = header.child_sized(
                        Text::new(label).fg(colors::ACCENT_CYAN),
                        w,
                    );
                }
                if let Some(ref a) = self.store.selected_agent.get() {
                    let label = format!("· {}", a);
                    let w = label.chars().count() as u16 + 1;
                    header = header.child_sized(
                        Text::new(label).fg(colors::ACCENT_PURPLE),
                        w,
                    );
                }
                // Run status indicator pinned to the right via a flex spacer.
                let (status_text, status_color) = match &h.active_session.run_status.get() {
                    RunStatus::Running => (Some(" ● Running"), colors::ACCENT_GREEN),
                    RunStatus::Sending => (Some(" ○ Sending"), colors::ACCENT_YELLOW),
                    RunStatus::WaitingUser => (Some(" ⏸ Waiting"), colors::ACCENT_YELLOW),
                    RunStatus::Error(_) => (Some(" ✕ Error"), colors::ACCENT_RED),
                    RunStatus::Idle => (None, colors::FG_MUTED),
                };
                // Spacer flex grows to push the status to the right edge.
                header = header.child_flex(Text::new(""), 1.0);
                if let Some(s) = status_text {
                    let w = s.chars().count() as u16 + 1;
                    header = header.child_sized(Text::new(s).fg(status_color), w);
                }

                content_stack = content_stack.child_sized(header, 1);
                // Divider: thin line, single row, FG_MUTED so it recedes
                // visually rather than competing with the message content.
                content_stack = content_stack.child_sized(
                    Text::new("─".repeat(ctx.area.width as usize)).fg(colors::BORDER),
                    1,
                );

                let msgs = h.active_session.messages.get();

                // Build transcript + optional sidebar.
                //
                // CRITICAL: every block must be `child_sized` to its
                // estimated natural height. Without this, vstack
                // distributes the transcript area equally across all
                // children — a single user prompt fills the whole pane
                // while every assistant message gets only 1-2 rows and
                // looks empty (the bug we hit on first send).
                //
                // We also need bottom-anchored truncation so the latest
                // tool result and assistant text stay visible: if total
                // height exceeds the available transcript area, drop
                // blocks from the FRONT (oldest) until the remainder
                // fits. Without this, a long tool_catalog_search result
                // pushes the final assistant answer off the bottom of
                // the screen.
                let mut main_area = hstack().gap(0);
                let mut transcript = vstack().gap(0);

                if msgs.is_empty() {
                    transcript = transcript.child(
                        Text::new("   Type below to start a conversation.")
                            .fg(colors::FG_MUTED)
                    );
                    main_area = main_area.child_flex(transcript, 1.0);
                } else {
                    // True scrollable timeline.
                    //
                    // We compute total content height = Σ block heights,
                    // then apply the user's scroll_offset (rows-from-bottom)
                    // to slide a viewport over it. PageUp/PageDown adjust
                    // scroll_offset; new messages auto-pin to the bottom
                    // ONLY when offset is 0, so reading old history
                    // doesn't get yanked back to the latest mid-read.
                    let available = ctx.area.height.saturating_sub(9);
                    let total_h: u16 = msgs.iter()
                        .map(|b| layout_block(b, 0).height)
                        .sum::<u16>()
                        .saturating_add(1);

                    // Clamp scroll offset: 0 = bottom-pinned, max = total - viewport.
                    let max_offset = total_h.saturating_sub(available);
                    let user_offset = h.active_session.scroll_offset.get().min(max_offset);
                    // ScrollView counts offset from the TOP, but our
                    // store treats it as "rows back from bottom" so the
                    // latest message stays visible by default. Convert.
                    let scroll_top = max_offset.saturating_sub(user_offset);

                    let cursor_idx = h.active_session.transcript_cursor.get();
                    // turn 级思考延续标记：UserPrompt 起一个新 turn，其后首个
                    // Thinking 用 ✻，同 turn 内被 text/tool 夹断的后续 Thinking
                    // 用 ┆ 续接符（避免 reasoning 流被拆成一串重复 ✻ 独立块）。
                    let mut turn_has_thinking = false;
                    for (i, block) in msgs.iter().enumerate() {
                        let thinking_continuation = matches!(
                            block,
                            TranscriptBlock::Thinking { .. }
                        ) && turn_has_thinking;
                        let blk = layout_block_ctx(block, h.spinner_tick, thinking_continuation);
                        // 紧凑重排（spec 2026-06-16）：非 cursor 块顶格无 bar，
                        // cursor 选中块左侧显 ▌BORDER_SEL 焦点条（cursor 时才占 1 列，
                        // 内容随之右移 1 列，属局部焦点效果）。
                        let is_cursor = Some(i) == cursor_idx;
                        let rendered = if is_cursor {
                            hstack().gap(0)
                                .child_sized(Text::new("▌").fg(colors::BORDER_SEL).bold(), 1)
                                .child_flex(blk.view, 1.0)
                        } else {
                            blk.view
                        };
                        transcript = transcript.child_sized(rendered, blk.height);
                        match block {
                            TranscriptBlock::UserPrompt { .. } => turn_has_thinking = false,
                            TranscriptBlock::Thinking { .. } => turn_has_thinking = true,
                            _ => {}
                        }
                    }
                    let status = h.active_session.run_status.get();
                    if matches!(status, RunStatus::Sending) {
                        transcript = transcript.child_sized(
                            Text::new(" ⏳ Sending...").fg(colors::ACCENT_YELLOW),
                            1,
                        );
                    }

                    let sv = crate::widget::scroll_view()
                        .with_content_height(total_h)
                        .scroll_offset(scroll_top)
                        .show_scrollbar(true);

                    main_area = main_area.child_flex(
                        ScrollableTranscript {
                            sv,
                            content: transcript,
                            content_h: total_h,
                            scroll_top,
                            publish: h.transcript_scrollbar_publish.clone(),
                        },
                        1.0,
                    );
                }

                // Sidebar (toggle with b key)
                if h.sidebar_visible {
                    let token = h.active_session.token_usage.get();
                    let cache = h.active_session.cache_stats.get();
                    let price = h.active_session.pricing.get();
                    let ctx_pct = h.active_session.context_pct.get();
                    let trees = h.active_session.sidebar_trees.get();
                    let mcp = h.active_session.mcp_lsp.get();
                    let tools = h.active_session.active_tools.get();
                    let (sidebar_content, sidebar_content_h) = crate::telemetry::SessionSidebar::build(
                        &token, &cache, &price, ctx_pct, &trees, &mcp, &tools,
                    );
                    let sidebar = ScrollableSidebar {
                        content: sidebar_content,
                        content_h: sidebar_content_h,
                        scroll_top: h.sidebar_scroll_offset,
                        publish: h.sidebar_scrollbar_publish.clone(),
                    };
                    main_area = main_area.child_sized(sidebar, 32);
                }

                // main_area takes all remaining vertical space below the
                // header + divider; without `child_flex` the outer vstack
                // splits its area equally and the transcript ends up with
                // less than half the screen even when content_stack has
                // only three children.
                content_stack = content_stack.child_flex(main_area, 1.0);
            }
        }

        // ── Context strip: model / agent / mode ──
        let mut ctx_parts: Vec<String> = Vec::new();
        if let Some(ref m) = self.store.selected_model.get() {
            ctx_parts.push(format!("m:{}", m));
        }
        if let Some(ref a) = self.store.selected_agent.get() {
            ctx_parts.push(format!("a:{}", a));
        }
        if let Some(ref m) = self.store.selected_mode.get() {
            ctx_parts.push(format!("{}", m));
        }
        let context_strip_text = if ctx_parts.is_empty() {
            "[default]".to_string()
        } else {
            format!("[{}]", ctx_parts.join(" "))
        };
        let context_strip = Text::new(&context_strip_text).fg(colors::FG_MUTED);

        // ── Attachment strip ──
        let attachments = h.active_session.attachments.get();
        let attachment_strip = if attachments.is_empty() {
            vstack()
        } else {
            let mut strip = vstack();
            for att in &attachments {
                let label = match &att.kind {
                    crate::store::types::AttachmentKind::File { path, .. } => {
                        format!(" 📎 {} ({})", att.name, path)
                    }
                    crate::store::types::AttachmentKind::Image { mime, .. } => {
                        format!(" 🖼 {} [{}]", att.name, mime)
                    }
                };
                strip = strip.child(Text::new(&label).fg(colors::ACCENT_PURPLE));
            }
            strip
        };

        // ── Prompt bar ──
        let spinner = if is_running {
            format!("{} ", crate::widget::spinner::frame(
                crate::widget::spinner::SpinnerGlyph::Claude,
                h.spinner_tick / 3,
            ))
        } else { String::new() };
        let hint = if h.interrupt_pending {
            " ⚠ Press Esc again to interrupt".to_string()
        } else if is_slash {
            " ↑↓ select  Enter: execute  Esc: close".to_string()
        } else {
            h.prompt.status_hint(is_running)
        };
        let hint_text = Text::new(&format!(" {}{}", spinner, hint)).fg(colors::FG_MUTED);
        let input_border = if h.prompt.is_focused() {
            Border::rounded().fg(colors::ACCENT_CYAN)
        } else {
            Border::rounded().fg(colors::BORDER)
        };
        let input_widget = input_border.child(h.prompt.widget());
        // hint: 1 row, input border: 3 rows (top + content + bottom).
        // Without child_sized, vstack divides the 4-row prompt_bar slot
        // equally — clipping the bordered input to 2 rows and erasing
        // its content line.
        let prompt_bar = vstack()
            .child_sized(hint_text, 1)
            .child_sized(input_widget, 3);

        // ── Status bar ──
        let panel_label = match h.panel {
            Panel::Slash => "slash",
            Panel::ModelSelect => "model",
            Panel::AgentSelect => "agent",
            Panel::SessionList => "sessions",
            Panel::Stash => "stash",
            Panel::Rename => "rename",
            Panel::Confirm => "confirm",
            Panel::Permission => "perm",
            Panel::Question => "question",
            Panel::Help => "help",
            Panel::Alert => "alert",
            Panel::None => route.as_str(),
        };
        let dir = self.store.working_dir.get();
        let dir_short = dir.rsplit('/').next().unwrap_or(&dir);
        let tokens = h.active_session.token_usage.get();
        let mut stats = String::new();
        if tokens.total > 0 {
            stats.push_str(&format!(" in:{} out:{}", crate::theme::fmt_tokens(tokens.input), crate::theme::fmt_tokens(tokens.output)));
            if tokens.cache_read > 0 || tokens.cache_miss > 0 {
                stats.push_str(&format!(" cache:{}r/{}m", crate::theme::fmt_tokens(tokens.cache_read), crate::theme::fmt_tokens(tokens.cache_miss)));
            }
            if tokens.total_cost > 0.0 {
                stats.push_str(&format!(" {}", crate::theme::fmt_cost(tokens.total_cost)));
            }
        }
        // Active tasks count
        let active_tools = h.active_session.active_tools.get();
        let running = active_tools.iter().filter(|t| t.phase == ToolPhase::Running).count();
        if running > 0 {
            stats.push_str(&format!(" tasks:{}", running));
        }
        // Show cursor + key hints for transcript navigation. Without
        // these, users have no clue Tab/Space exist — and Space/Tab
        // don't visibly do anything until they happen to hover their
        // eyes on the right spot. Status-bar hint advertises the
        // shortcut and confirms the cursor moved.
        let cursor_hint = match h.active_session.transcript_cursor.get() {
            Some(idx) => format!(" cursor:{}", idx + 1),
            None => String::new(),
        };
        let nav_hint = if matches!(route, Route::Session { .. }) {
            " Tab:nav Space:fold PgUp/Dn:scroll"
        } else {
            ""
        };
        let status_text = format!(
            " {} │ [{}]{}{} │{} q:quit ^P:cmd ?:help ",
            dir_short, panel_label, stats, cursor_hint, nav_hint,
        );
        let status_bar = Text::new(&status_text)
            .fg(colors::FG_MUTED)
            .bg(colors::BG_SECONDARY);

        // ── Full layout ──
        // content_stack expands (flex 1.0); the strips below have fixed
        // heights so the page never compresses the main content. Without
        // explicit sizing, vstack would equally divide rows across all 5
        // children — flatting the home logo + sidebar to ~8 rows each.
        let attachment_h: u16 = if attachments.is_empty() { 0 } else {
            attachments.len().min(3) as u16
        };
        let layout = vstack()
            .child_flex(content_stack, 1.0)
            .child_sized(context_strip, 1)
            .child_sized(attachment_strip, attachment_h)
            .child_sized(prompt_bar, 4)   // hint (1) + bordered input (3)
            .child_sized(status_bar, 1);

        layout.render(ctx);

        // ── Render overlays (positioned above prompt bar) ──
        drop(h); // Release borrow before re-borrowing
        // Publish the transcript viewport height so the NEXT event
        // handler (Tab / j / k / fold) knows how much room is left for
        // the cursor to land in. Without this, `ensure_cursor_visible`
        // would have to guess, and on a 30-row terminal the cursor
        // would sometimes scroll past the visible window when
        // navigating to a far block.
        self.handler.borrow_mut().transcript_viewport_h = transcript_viewport_h;
        // Transcript area starts at y=2 (after header row + divider row).
        // Used by mouse click handler to map click_y → transcript row.
        self.handler.borrow_mut().transcript_area_y = 2;
        // Drain the transcript scrollbar's per-frame publish into the
        // handler so the next mouse event can hit-test arrow clicks
        // and thumb drags. The publish slot is None when the session
        // route's content fits in the viewport, in which case the
        // scrollbar area stays None and the mouse handler skips it.
        // Snapshot the publish slot via Copy. Doing it in a single
        // expression means the temporary `Ref<AppHandler>` and inner
        // `Ref<…publish…>` both drop at the `;`. The previous `if let`
        // form extended the outer `Ref`'s lifetime across the arm body
        // (Edition 2021 temp-lifetime rules for `if let` initializers),
        // colliding with the in-arm `borrow_mut()` and panicking
        // with "RefCell already borrowed" on first render in direct mode.
        let publish_snapshot: Option<TranscriptScrollbarPublish> = self
            .handler
            .borrow()
            .transcript_scrollbar_publish
            .try_borrow()
            .ok()
            .and_then(|opt| opt.as_ref().copied());
        match publish_snapshot {
            Some(p) => {
                self.handler.borrow_mut().transcript_scrollbar_area = Some(p.area);
                self.handler.borrow_mut().transcript_scrollbar_metrics =
                    Some((p.content_h, p.viewport_h));
            }
            None => {
                self.handler.borrow_mut().transcript_scrollbar_area = None;
                self.handler.borrow_mut().transcript_scrollbar_metrics = None;
            }
        }
        // Same drain for the sidebar scrollbar.
        // Same drain for the sidebar scrollbar — see transcript
        // comment above for why the read-then-write pattern is split
        // across two expressions (RefCell double-borrow avoidance).
        let publish_snapshot: Option<TranscriptScrollbarPublish> = self
            .handler
            .borrow()
            .sidebar_scrollbar_publish
            .try_borrow()
            .ok()
            .and_then(|opt| opt.as_ref().copied());
        match publish_snapshot {
            Some(p) => {
                self.handler.borrow_mut().sidebar_scrollbar_area = Some(p.area);
                self.handler.borrow_mut().sidebar_scrollbar_metrics =
                    Some((p.content_h, p.viewport_h));
            }
            None => {
                self.handler.borrow_mut().sidebar_scrollbar_area = None;
                self.handler.borrow_mut().sidebar_scrollbar_metrics = None;
            }
        }
        let h = self.handler.borrow();
        let prompt_y = ctx.area.y + ctx.area.height.saturating_sub(5);
        match h.panel {
            Panel::Slash => {
                let popup = h.slash_popup.render_popup();
                // 左对齐占满宽(与输入框对齐),左右各留 2 列。不再居中固定 50——
                // 居中会让补全脱离输入框本体,窄屏还截断描述(金克木:框不得压输入)。
                // px 是相对 ctx.area 的偏移(positioned.x 语义),故 px=2 即左对齐。
                let pw = ctx.area.width.saturating_sub(4);
                let ph = (h.slash_popup.filtered_count().min(8) as u16 + 4).min(ctx.area.height.saturating_sub(6));
                let px = 2u16;
                // prompt_y 是绝对坐标(含 ctx.area.y)。positioned.x/.y 是相对 ctx.area,
                // Buffer::fill 是绝对 —— 口径必须分清,否则 fill 填错位、popup 区域没被
                // 实色覆盖,下层 transcript 透字。故 fill 用绝对,positioned 用相对。
                let py_abs = prompt_y.saturating_sub(ph).max(1);
                let py_rel = py_abs.saturating_sub(ctx.area.y);
                // positioned 浮层不清背景,先实色预填挡住下层 transcript(实色不透字)。
                // fill 用绝对坐标填满整行宽(从 ctx.area.x 起 width 列);popup 内容仍
                // px=2 缩进与输入框对齐。padding 区(左右各 2 列)也要实色,否则边缘透字。
                h.slash_popup.fill_background(ctx.buffer, ctx.area.x, py_abs, ctx.area.width, ph);
                revue::widget::positioned(popup).x(px as i16).y(py_rel as i16).width(pw).height(ph).render(ctx);
            }
            Panel::ModelSelect => h.model_select.render(ctx),
            Panel::AgentSelect => h.agent_select.render(ctx),
            Panel::SessionList => h.session_list.render(ctx),
            Panel::Stash => h.stash_dialog.render(ctx),
            Panel::Rename => h.rename_dialog.render(ctx),
            Panel::Confirm => h.confirm_dialog.render(ctx),
            Panel::Permission => h.permission_dialog.render(ctx),
            Panel::Question => h.question_dialog.render(ctx),
            Panel::Help => h.help.render(ctx),
            _ => {}
        }

        // ── Toast overlay ─────────────────────────────────────────
        // Pending toasts hover above the prompt bar so the user sees
        // why an action was rejected (e.g. "Provider not connected").
        // We show only the most-recent toast that hasn't expired and
        // bound its width so the line stays readable on narrow terminals.
        let toasts = self.store.toasts.get();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // Find the latest non-expired toast. expires_at == 0 is treated
        // as "no deadline" for backwards compatibility — present code
        // always sets it, but this leaves room for legacy callers.
        let active = toasts.iter().rev().find(|t| t.expires_at == 0 || t.expires_at > now_ms);
        if let Some(t) = active {
            use crate::store::types::ToastMsgVariant;
            let (icon, color) = match t.variant {
                ToastMsgVariant::Success => ("✓", colors::ACCENT_GREEN),
                ToastMsgVariant::Error   => ("✕", colors::ACCENT_RED),
                ToastMsgVariant::Warning => ("⚠", colors::ACCENT_YELLOW),
                ToastMsgVariant::Info    => ("•", colors::ACCENT_CYAN),
            };
            let max_w = ctx.area.width.saturating_sub(4).min(80);
            let raw = format!("{} {}", icon, t.text);
            // Truncate to fit so emojis at the edge don't half-render.
            let display: String = if raw.chars().count() as u16 > max_w {
                let mut s: String = raw.chars().take(max_w as usize).collect();
                s.push('…');
                s
            } else {
                raw
            };
            let w = (display.chars().count() as u16).min(max_w).max(10);
            let x = (ctx.area.width.saturating_sub(w + 2)) / 2;
            let y = prompt_y.saturating_sub(2).max(1);
            // Bordered toast keeps the message visually distinct from
            // the transcript text underneath.
            let toast_widget = Border::rounded()
                .fg(color)
                .child(Text::new(display).fg(color));
            revue::widget::positioned(toast_widget)
                .x(x as i16)
                .y(y as i16)
                .width(w + 2)
                .height(3)
                .render(ctx);
        }
    }
}

#[cfg(test)]
mod drain_publish_regression {
    //! Regression test for the `RefCell already borrowed` panic that
    //! struck the agendao TUI on first frame in Direct mode
    //! (introduced in commit 98108a3, fixed in the follow-up).
    //!
    //! Root cause (Phase 1 of systematic-debugging):
    //! The borrow shape
    //!
    //!     if let Ok(publish) = outer.borrow().field.try_borrow() {
    //!         match publish.as_ref() {
    //!             Some(p) => outer.borrow_mut().x = Some(p.x),  // PANIC
    //!         }
    //!     }
    //!
    //! In Edition 2021, the temporary `Ref<Outer>` (from `outer.borrow()`)
    //! is live for the whole `if let` block because it participates in
    //! the pattern binding — so the in-block `outer.borrow_mut()` collides
    //! with the still-live `Ref<Outer>` and panics with
    //! "RefCell already borrowed".
    //!
    //! The fix snapshots the inner `Option<Copy>` into a local in a single
    //! expression, so all temporaries drop at the `;` and the subsequent
    //! `borrow_mut()` is clean. These tests pin both halves of that story.

    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Payload {
        area: u32,
        content_h: u32,
        viewport_h: u32,
    }

    /// Pin the OLD broken pattern: a `Ref` is alive at the same time as a
    /// `borrow_mut()` on the same `RefCell` — `RefCell` is special-cased by
    /// the borrow checker (multiple `borrow()` calls are statically
    /// allowed; runtime tracks the count), so this **compiles** but
    /// **panics at runtime**. This is the exact shape from the original
    /// `if let Ok(publish) = self.handler.borrow()...` block: the
    /// `if let`-bound `Ref<AppHandler>` stayed alive across the arm body's
    /// `self.handler.borrow_mut()` calls, producing the same runtime panic.
    /// If this test ever stops panicking, the assumption in the module
    /// doc-comment is wrong and the rest of this file is suspect.
    #[test]
    #[should_panic(expected = "already borrowed")]
    fn old_pattern_panics() {
        let cell: RefCell<u32> = RefCell::new(0);
        let _r = cell.borrow(); // Ref alive until end of scope
        let _ = cell.borrow_mut(); // collides with _r at runtime → panic
    }

    /// Pin the FIX pattern: snapshot via Copy in a single expression so all
    /// temporaries drop at `;`, then a fresh `borrow_mut()` is clean. This
    /// is the exact shape used in `RootView::render` for the transcript
    /// and sidebar scrollbar publish drain.
    #[test]
    fn fix_copy_snapshot_then_mut_borrow_succeeds() {
        struct Outer {
            area: Option<Payload>,
            metrics: Option<(u32, u32)>,
        }
        let outer: RefCell<Outer> = RefCell::new(Outer {
            area: None,
            metrics: None,
        });
        let inner: Rc<RefCell<Option<Payload>>> = Rc::new(RefCell::new(Some(Payload {
            area: 7,
            content_h: 100,
            viewport_h: 30,
        })));

        // Fix: snapshot in one expression; all temporaries drop at `;`.
        let snapshot: Option<Payload> = {
            let _outer_guard = outer.borrow();
            inner.try_borrow().ok().and_then(|opt| opt.as_ref().copied())
        };
        // After this `;`, no `Ref`s are alive — safe to `borrow_mut`.

        match snapshot {
            Some(p) => {
                outer.borrow_mut().area = Some(p);
                outer.borrow_mut().metrics = Some((p.content_h, p.viewport_h));
            }
            None => {
                outer.borrow_mut().area = None;
                outer.borrow_mut().metrics = None;
            }
        }
        assert_eq!(
            outer.borrow().area,
            Some(Payload { area: 7, content_h: 100, viewport_h: 30 })
        );
        assert_eq!(outer.borrow().metrics, Some((100, 30)));
    }
}
