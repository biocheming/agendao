use super::{
    SessionMessageOutputCache, SessionMessageViewportState, SessionMessagesSnapshot,
    SessionMessagesSnapshotSeed,
    SessionReasoningState, SessionRenderModelCache, SessionRenderSnapshot,
    SessionRenderSnapshotSeed, SessionView,
    build_session_render_model_memo_key, build_session_viewport_content_memo_key,
    map_scrollbar_row_to_offset, render_session_messages_child, reset_session_render_perf_counters,
    resolve_session_render_model, snapshot_session_render_perf_counters,
};
use chrono::Utc;
use parking_lot::Mutex;
use ratatui::{buffer::Buffer, layout::Rect};
use reratui::fiber_tree::{clear_fiber_tree, set_fiber_tree};
use reratui::{
    Component, Element, FiberTree, clear_current_event, clear_global_handlers,
    clear_render_context, init_render_context, reset_component_position_counter,
    set_current_event,
    with_render_context_mut,
};

use crate::{
    api::{PendingPermissionSummary, SessionRunStatusKind, SessionRuntimeState},
    bridge::{
        ReactiveAnimationsEnabled, ReactiveAppContextHandle, ReactivePromptHandle,
        ReactiveRouteSnapshot, ReactiveSessionViewHandle, ReactiveUiEventEmitter,
    },
    components::Prompt,
    context::{AppContext, Message, MessagePart, MessageRole, SessionStatus, TokenUsage},
    ui::BufferSurface,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use std::sync::Arc;

struct TestSessionMessagesRender {
    area: Rect,
    snapshot: SessionMessagesSnapshot,
    viewport: SessionMessageViewportState,
    reasoning: SessionReasoningState,
    message_cache: SessionMessageOutputCache,
    render_model_cache: SessionRenderModelCache,
    output: Arc<Mutex<Option<super::SessionMessagesOutput>>>,
}

impl Component for TestSessionMessagesRender {
    fn render(&self, _area: Rect, buffer: &mut reratui::Buffer) {
        let output = render_session_messages_child(
            self.area,
            &self.snapshot,
            &self.viewport,
            &self.reasoning,
            &self.message_cache,
            &self.render_model_cache,
            buffer,
        );
        *self.output.lock() = Some(output);
    }
}

struct TestReactiveSessionMessagesComponent {
    area: Rect,
    context: Arc<AppContext>,
    session_id: String,
    viewport: SessionMessageViewportState,
    reasoning: SessionReasoningState,
    output: Arc<Mutex<Option<super::SessionMessagesOutput>>>,
    sidebar_state: Arc<Mutex<Option<super::SessionSidebarChromeState>>>,
}

struct TestReactiveSessionViewComponent {
    context: Arc<AppContext>,
    view: SessionView,
    prompt: Prompt,
    area: Rect,
}

impl Component for TestReactiveSessionViewComponent {
    fn render(&self, _area: Rect, buffer: &mut reratui::Buffer) {
        let _app_context = reratui::hooks::use_context_provider(|| {
            ReactiveAppContextHandle(self.context.clone())
        });
        let _theme =
            reratui::hooks::use_context_provider(|| self.context.theme.read().clone());
        let _keybinds =
            reratui::hooks::use_context_provider(|| self.context.keybind.read().clone());
        let _route =
            reratui::hooks::use_context_provider(|| ReactiveRouteSnapshot(self.context.current_route()));
        let _animations = reratui::hooks::use_context_provider(|| {
            ReactiveAnimationsEnabled(*self.context.animations_enabled.read())
        });
        let _prompt_input_blocked = reratui::hooks::use_context_provider(|| {
            crate::bridge::ReactivePromptInputBlocked(self.context.has_blocking_dialogs())
        });
        let _slash_popup_open = reratui::hooks::use_context_provider(|| {
            crate::bridge::ReactiveSlashPopupOpen(
                self.context
                    .is_dialog_open(crate::context::DialogSlot::SlashPopup),
            )
        });
        let _event_emitter =
            reratui::hooks::use_context_provider(|| ReactiveUiEventEmitter(self.context.clone()));
        let _prompt =
            reratui::hooks::use_context_provider(|| ReactivePromptHandle(self.prompt.clone()));
        let _session_view = reratui::hooks::use_context_provider(|| {
            ReactiveSessionViewHandle(self.context.session_view_handle())
        });
        self.view
            .render_reactive_with_prompt(&self.context, buffer, self.area, &self.prompt);
    }
}

impl Component for TestReactiveSessionMessagesComponent {
    fn render(&self, _area: Rect, buffer: &mut reratui::Buffer) {
        let _app_context = reratui::hooks::use_context_provider(|| {
            ReactiveAppContextHandle(self.context.clone())
        });
        let _theme =
            reratui::hooks::use_context_provider(|| self.context.theme.read().clone());
        let _keybinds =
            reratui::hooks::use_context_provider(|| self.context.keybind.read().clone());
        let _route =
            reratui::hooks::use_context_provider(|| ReactiveRouteSnapshot(self.context.current_route()));
        let _animations = reratui::hooks::use_context_provider(|| {
            ReactiveAnimationsEnabled(*self.context.animations_enabled.read())
        });
        let _prompt_input_blocked = reratui::hooks::use_context_provider(|| {
            crate::bridge::ReactivePromptInputBlocked(self.context.has_blocking_dialogs())
        });
        let _slash_popup_open = reratui::hooks::use_context_provider(|| {
            crate::bridge::ReactiveSlashPopupOpen(
                self.context
                    .is_dialog_open(crate::context::DialogSlot::SlashPopup),
            )
        });
        let _event_emitter =
            reratui::hooks::use_context_provider(|| ReactiveUiEventEmitter(self.context.clone()));
        let _prompt = reratui::hooks::use_context_provider(|| {
            ReactivePromptHandle(Prompt::new(self.context.clone()))
        });
        let _session_view = reratui::hooks::use_context_provider(|| {
            ReactiveSessionViewHandle(self.context.session_view_handle())
        });

        let child = Element::component(super::SessionMessagesComponent {
            area: self.area,
            snapshot: SessionMessagesSnapshot::from_seed(
                &SessionMessagesSnapshotSeed::capture(&self.context, &self.session_id),
            ),
            viewport: self.viewport.clone(),
            reasoning: self.reasoning.clone(),
            output: self.output.clone(),
        });
        child.render(self.area, buffer);

        if let Some(view) = self.context.session_view_handle() {
            let sidebar_snapshot = view.state.lock().sidebar.clone();
            let sidebar_area = sidebar_snapshot
                .render_state
                .sidebar_area()
                .or_else(|| {
                    (super::session_sidebar_should_render_overlay(
                        &sidebar_snapshot.lifecycle,
                        sidebar_snapshot.last_terminal_width,
                    ))
                        .then_some(self.area)
                });

            if let Some(area) = sidebar_area {
                let render_state = Arc::new(Mutex::new(sidebar_snapshot.render_state.clone()));
                let lifecycle = Arc::new(Mutex::new(sidebar_snapshot.lifecycle.clone()));
                let sidebar_seed =
                    crate::components::Sidebar::capture_render_seed(&self.context, &self.session_id);
                crate::components::Sidebar::new(self.session_id.clone())
                .render_reactive(
                    crate::components::Sidebar::render_inputs_from_seed(&sidebar_seed),
                    buffer,
                    area,
                    render_state.clone(),
                    lifecycle.clone(),
                    true,
                    None,
                    crate::components::SidebarChromeProps {
                        mode: crate::components::SidebarChromeMode::Overlay,
                        container_area: self.area,
                        layout_width: self.area.width,
                        open_button_area: None,
                        close_button_area: None,
                        backdrop_area: Some(self.area),
                    },
                );

                let mut state = view.state.lock();
                state.sidebar.render_state = render_state.lock().clone();
                state.sidebar.lifecycle = lifecycle.lock().clone();
            }
            *self.sidebar_state.lock() = Some(view.state.lock().sidebar.clone());
        }
    }
}


fn make_message(id: &str, role: MessageRole, content: String, parts: Vec<MessagePart>) -> Message {
    Message {
        id: id.to_string(),
        role,
        content,
        created_at: Utc::now(),
        agent: None,
        model: Some("openai/gpt-5".to_string()),
        mode: None,
        finish: None,
        error: None,
        completed_at: None,
        cost: 0.0,
        tokens: TokenUsage::default(),
        metadata: None,
        multimodal: None,
        parts,
    }
}

fn long_block(label: &str, repeat: usize) -> String {
    std::iter::repeat_n(
        format!("{label} keeps the viewport busy with a wider paragraph for reratui caching."),
        repeat,
    )
    .collect::<Vec<_>>()
    .join(" ")
}

fn buffer_text(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn multiline_reasoning_block(label: &str, lines: usize) -> String {
    (0..lines)
        .map(|idx| {
            format!(
                "{label} step {} keeps the reasoning panel expanded.",
                idx + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_perf_session_messages() -> Vec<Message> {
    vec![
        make_message(
            "user-1",
            MessageRole::User,
            long_block("user-1", 4),
            vec![MessagePart::Text {
                text: long_block("user-1", 4),
            }],
        ),
        make_message(
            "assistant-1",
            MessageRole::Assistant,
            long_block("assistant-1", 4),
            vec![
                MessagePart::Reasoning {
                    text: multiline_reasoning_block("assistant-1 reasoning", 8),
                },
                MessagePart::Text {
                    text: long_block("assistant-1 reply", 6),
                },
            ],
        ),
        make_message(
            "user-2",
            MessageRole::User,
            long_block("user-2", 3),
            vec![MessagePart::Text {
                text: long_block("user-2", 3),
            }],
        ),
        make_message(
            "assistant-2",
            MessageRole::Assistant,
            long_block("assistant-2", 5),
            vec![MessagePart::Text {
                text: long_block("assistant-2", 5),
            }],
        ),
        make_message(
            "user-3",
            MessageRole::User,
            long_block("user-3", 3),
            vec![MessagePart::Text {
                text: long_block("user-3", 3),
            }],
        ),
        make_message(
            "assistant-3",
            MessageRole::Assistant,
            long_block("assistant-3", 5),
            vec![MessagePart::Text {
                text: long_block("assistant-3", 5),
            }],
        ),
    ]
}

#[test]
fn builds_context_usage_bar_clamped_to_width() {
    assert_eq!(super::context_usage_bar(Some(0), 5), "[░░░░░]");
    assert_eq!(super::context_usage_bar(Some(50), 5), "[███░░]");
    assert_eq!(super::context_usage_bar(Some(140), 5), "[█████]");
}

#[test]
fn formats_context_usage_label_without_meter() {
    assert_eq!(
        super::format_context_usage_label(12_450, Some(200_000)),
        "12.4K/200K 6%"
    );
}

#[test]
fn formats_context_usage_meter_with_compact_bar() {
    assert_eq!(
        super::format_context_usage_meter(12_450, Some(200_000)),
        Some(("ctx 12.4K/200K [█░░░░░░░] 6%".to_string(), Some(6)))
    );
}

#[test]
fn formats_context_usage_meter_without_limit_as_fallback_label() {
    assert_eq!(
        super::format_context_usage_meter(12_450, None),
        Some(("ctx 12.4K".to_string(), None))
    );
}

fn perf_snapshot_with_messages() -> (Arc<AppContext>, String, SessionMessagesSnapshot) {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Perf Session".to_string()));
        session.set_messages(&session_id, build_perf_session_messages());
        session_id
    };
    let snapshot =
        SessionMessagesSnapshot::from_seed(&SessionMessagesSnapshotSeed::capture(&context, &session_id));
    (context, session_id, snapshot)
}

fn render_perf_session_messages(
    area: Rect,
    snapshot: &SessionMessagesSnapshot,
    viewport: &SessionMessageViewportState,
    reasoning: &SessionReasoningState,
    message_cache: &SessionMessageOutputCache,
    render_model_cache: &SessionRenderModelCache,
) -> super::SessionMessagesOutput {
    clear_fiber_tree();
    clear_render_context();
    set_fiber_tree(FiberTree::new());
    init_render_context();
    with_render_context_mut(|ctx| ctx.prepare_for_render());
    reset_component_position_counter();
    clear_global_handlers();

    let output = Arc::new(Mutex::new(None));
    let root = Element::component(TestSessionMessagesRender {
        area,
        snapshot: snapshot.clone(),
        viewport: viewport.clone(),
        reasoning: reasoning.clone(),
        message_cache: message_cache.clone(),
        render_model_cache: render_model_cache.clone(),
        output: output.clone(),
    });
    let mut buffer = Buffer::empty(area);
    root.render(area, &mut buffer);

    with_render_context_mut(|ctx| {
        ctx.mark_unseen_for_unmount();
        ctx.process_unmounts();
        ctx.begin_batch();
        let _ = ctx.end_batch();
        ctx.flush_effects();
    });
    clear_current_event();
    clear_fiber_tree();
    clear_render_context();

    let result = output.lock().take().expect("session messages output");
    result
}

fn render_session_view_once(
    view: &SessionView,
    context: &Arc<AppContext>,
    area: Rect,
    prompt: &Prompt,
) -> Buffer {
    clear_fiber_tree();
    clear_render_context();
    set_fiber_tree(FiberTree::new());
    init_render_context();
    with_render_context_mut(|ctx| ctx.prepare_for_render());
    reset_component_position_counter();
    clear_global_handlers();

    let mut buffer = Buffer::empty(area);
    {
        let mut surface = BufferSurface::new(&mut buffer);
        view.render(context, &mut surface, area, prompt);
    }

    with_render_context_mut(|ctx| {
        ctx.mark_unseen_for_unmount();
        ctx.process_unmounts();
        ctx.begin_batch();
        let _ = ctx.end_batch();
        ctx.flush_effects();
    });
    clear_current_event();
    clear_fiber_tree();
    clear_render_context();

    buffer
}

fn render_reactive_session_messages_with_event(
    context: &Arc<AppContext>,
    session_id: &str,
    area: Rect,
    viewport: &SessionMessageViewportState,
    reasoning: &SessionReasoningState,
    event: Option<Event>,
) -> (super::SessionMessagesOutput, Option<super::SessionSidebarChromeState>) {
    clear_fiber_tree();
    clear_render_context();
    set_fiber_tree(FiberTree::new());
    init_render_context();
    with_render_context_mut(|ctx| ctx.prepare_for_render());
    reset_component_position_counter();
    clear_global_handlers();
    set_current_event(event.map(Arc::new));

    let output = Arc::new(Mutex::new(None));
    let sidebar_state = Arc::new(Mutex::new(None));
    let root = Element::component(TestReactiveSessionMessagesComponent {
        area,
        context: context.clone(),
        session_id: session_id.to_string(),
        viewport: viewport.clone(),
        reasoning: reasoning.clone(),
        output: output.clone(),
        sidebar_state: sidebar_state.clone(),
    });
    let mut buffer = Buffer::empty(area);
    root.render(area, &mut buffer);

    with_render_context_mut(|ctx| {
        ctx.mark_unseen_for_unmount();
        ctx.process_unmounts();
        ctx.begin_batch();
        let _ = ctx.end_batch();
        ctx.flush_effects();
    });
    clear_current_event();
    clear_fiber_tree();
    clear_render_context();

    let result = output
        .lock()
        .take()
        .expect("reactive session messages output");
    let sidebar = sidebar_state.lock().take();
    (result, sidebar)
}

fn render_reactive_session_view_with_event(
    context: &Arc<AppContext>,
    view: &SessionView,
    prompt: &Prompt,
    area: Rect,
    event: Option<Event>,
) {
    clear_fiber_tree();
    clear_render_context();
    set_fiber_tree(FiberTree::new());
    init_render_context();
    with_render_context_mut(|ctx| ctx.prepare_for_render());
    reset_component_position_counter();
    clear_global_handlers();
    set_current_event(event.map(Arc::new));

    let root = Element::component(TestReactiveSessionViewComponent {
        context: context.clone(),
        view: view.clone(),
        prompt: prompt.clone(),
        area,
    });
    let mut buffer = Buffer::empty(area);
    root.render(area, &mut buffer);

    with_render_context_mut(|ctx| {
        ctx.mark_unseen_for_unmount();
        ctx.process_unmounts();
        ctx.begin_batch();
        let _ = ctx.end_batch();
        ctx.flush_effects();
    });
    clear_current_event();
    clear_fiber_tree();
    clear_render_context();
}

#[test]
fn scrollbar_row_maps_to_expected_offsets() {
    let area = Some(Rect {
        x: 10,
        y: 5,
        width: 1,
        height: 11,
    });
    assert_eq!(map_scrollbar_row_to_offset(area, 5, 100), 0);
    assert_eq!(map_scrollbar_row_to_offset(area, 10, 100), 50);
    assert_eq!(map_scrollbar_row_to_offset(area, 15, 100), 100);
}

#[test]
fn session_view_renders_to_buffer_surface() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 100, 30);
    let mut buffer = Buffer::empty(area);
    let cursor = {
        let mut surface = BufferSurface::new(&mut buffer);
        view.render(&context, &mut surface, area, &prompt);
        surface.cursor_position()
    };

    let rendered = buffer
        .content
        .iter()
        .filter(|cell| !cell.symbol().trim().is_empty())
        .count();
    assert!(rendered > 0);
    assert!(cursor.is_some());
}

#[test]
fn session_view_keeps_non_empty_prompt_input_visible_while_running() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Visible Prompt".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_status(&session_id, SessionStatus::Running);
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let mut prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    prompt.set_input("session hidden text".to_string());

    let area = Rect::new(0, 0, 78, 24);
    let mut buffer = Buffer::empty(area);
    {
        let mut surface = BufferSurface::new(&mut buffer);
        view.render(&context, &mut surface, area, &prompt);
    }

    let rendered = buffer_text(&buffer);
    assert!(
        rendered.contains("session hidden text"),
        "session prompt input should remain visible while running:\n{rendered}"
    );
}

#[test]
fn session_view_uses_taiji_marker_for_running_new_session_header() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(None);
        session.set_current_session_id(session_id.clone());
        session.set_status(&session_id, SessionStatus::Running);
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("☯ New Session"),
        "running new session header should use taiji marker:\n{rendered}"
    );
}

#[test]
fn session_view_first_render_keeps_transcript_visible_with_existing_assistant_output() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Transcript Visible".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "show me the result".to_string(),
                    vec![MessagePart::Text {
                        text: "show me the result".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "final answer".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: "thinking step one\nthinking step two".to_string(),
                        },
                        MessagePart::Text {
                            text: "final answer".to_string(),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);

    let rendered = buffer_text(&buffer);
    assert!(
        rendered.contains("final answer"),
        "assistant transcript should be visible on first session render:\n{rendered}"
    );
    assert!(
        rendered.contains("reasoning"),
        "reasoning transcript should be visible on first session render:\n{rendered}"
    );

    let messages_area = view
        .state
        .lock()
        .viewport
        .last_messages_area
        .expect("messages area");
    assert!(
        messages_area.height > 1,
        "messages viewport should not collapse to a single line on first render: {messages_area:?}"
    );
}

#[test]
fn session_view_inserts_single_blank_line_between_user_and_assistant_blocks() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Single Gap".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "show me the result".to_string(),
                    vec![MessagePart::Text {
                        text: "show me the result".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "final answer".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: "thinking step one\nthinking step two".to_string(),
                        },
                        MessagePart::Text {
                            text: "final answer".to_string(),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);
    let lines = rendered.lines().collect::<Vec<_>>();
    let user_line = lines
        .iter()
        .position(|line| line.contains("show me the result"))
        .expect("user transcript line");
    let reasoning_line = lines
        .iter()
        .position(|line| line.contains("▼ reasoning"))
        .expect("reasoning header line");
    let blank_lines = lines[user_line + 1..reasoning_line]
        .iter()
        .filter(|line| line.trim().is_empty())
        .count();

    assert_eq!(
        blank_lines, 1,
        "user/assistant blocks should be separated by a single blank line:\n{rendered}"
    );
}

#[test]
fn session_view_first_render_keeps_latest_reasoning_visible_for_tall_assistant_message() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Latest Reasoning Visible".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "investigate this author".to_string(),
                    vec![MessagePart::Text {
                        text: "investigate this author".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    long_block("assistant-final", 16),
                    vec![
                        MessagePart::Reasoning {
                            text: multiline_reasoning_block("latest reasoning", 6),
                        },
                        MessagePart::Text {
                            text: long_block("assistant-final", 16),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 18);
    let buffer = render_session_view_once(&view, &context, area, &prompt);

    let rendered = buffer_text(&buffer);
    assert!(
        rendered.contains("reasoning"),
        "latest reasoning header should remain visible on first render even when assistant body is tall:\n{rendered}"
    );
}

#[test]
fn reactive_session_component_handles_page_down_without_event_loop() {
    let (context, session_id, _snapshot) = perf_snapshot_with_messages();
    context.navigate_session(session_id.clone());
    let area = Rect::new(0, 0, 72, 10);

    let (first, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let (second, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::PageDown,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))),
    );
    assert!(
        second.viewport.scroll_offset > first.viewport.scroll_offset,
        "PageDown should be handled inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_handles_mouse_wheel_without_event_loop() {
    let (context, session_id, _snapshot) = perf_snapshot_with_messages();
    context.navigate_session(session_id.clone());
    let area = Rect::new(0, 0, 72, 10);

    let (first, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let messages_area = first
        .viewport
        .last_messages_area
        .expect("messages area should be captured");
    let (second, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: messages_area.x,
            row: messages_area.y,
            modifiers: KeyModifiers::NONE,
        })),
    );
    assert!(
        second.viewport.scroll_offset > first.viewport.scroll_offset,
        "mouse wheel scrolling should be handled inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_handles_sidebar_attached_focus_toggle() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Sidebar Focus".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "sidebar focus".to_string(),
                vec![MessagePart::Text {
                    text: "sidebar focus".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let (_second, sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ))),
    );
    let sidebar = sidebar.expect("sidebar state should be captured");

    assert!(
        sidebar.lifecycle.attached_session_focus,
        "ctrl+j should toggle attached-session focus inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_handles_sidebar_workspace_focus_toggle() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Sidebar Workspace".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "workspace focus".to_string(),
                vec![MessagePart::Text {
                    text: "workspace focus".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let (_second, sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ))),
    );
    let sidebar = sidebar.expect("sidebar state should be captured");

    assert!(
        sidebar.lifecycle.workspace_focus,
        "ctrl+k should toggle workspace focus inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_handles_sidebar_visibility_toggle() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Sidebar Toggle".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "sidebar toggle".to_string(),
                vec![MessagePart::Text {
                    text: "sidebar toggle".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let (_second, sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ))),
    );
    let sidebar = sidebar.expect("sidebar state should be captured");

    assert!(
        !sidebar.lifecycle.visible,
        "ctrl+s should toggle sidebar visibility inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_emits_session_navigation_intent_for_attached_enter() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Attached Enter".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "attached enter".to_string(),
                vec![MessagePart::Text {
                    text: "attached enter".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());
    context.set_attached_sessions(
        &session_id,
        vec![crate::context::AttachedSessionInfo {
            session_id: "attached-session".to_string(),
            stage_name: "Child".to_string(),
            stage_title: "Attached".to_string(),
            stage_id: Some("stage-1".to_string()),
            stage_index: Some(1),
            stage_total: Some(1),
            status: "running".to_string(),
        }],
    );

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    view.toggle_sidebar_attached_session_focus(
        crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1),
    );
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let _ = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))),
    );
    let events = context.drain_ui_events(8);
    assert!(
        events.iter().any(|event| matches!(
            event,
            crate::event::Event::Custom(custom)
                if matches!(
                    custom.as_ref(),
                    crate::event::CustomEvent::SessionNavigationIntent {
                        kind: crate::event::SessionNavigationIntentKind::Session(session_id),
                    } if session_id == "attached-session"
                )
        )),
        "attached sidebar enter should emit session navigation intent inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_emits_process_kill_intent() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Process Kill".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "process kill".to_string(),
                vec![MessagePart::Text {
                    text: "process kill".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());
    *context.processes.write() = vec![agendao_core::process_registry::ProcessInfo {
        pid: 42,
        name: "worker".to_string(),
        kind: agendao_core::process_registry::ProcessKind::Agent,
        started_at: 0,
        cpu_percent: 0.0,
        memory_kb: 0,
    }];

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    view.toggle_sidebar_process_focus(
        crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1),
    );
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let _ = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Delete,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))),
    );
    let events = context.drain_ui_events(8);
    assert!(
        events.iter().any(|event| matches!(
            event,
            crate::event::Event::Custom(custom)
                if matches!(
                    custom.as_ref(),
                    crate::event::CustomEvent::SessionSidebarIntent {
                        kind: crate::event::SessionSidebarIntentKind::KillSelectedProcess,
                    }
                )
        )),
        "process sidebar delete should emit kill intent inside the reactive session component"
    );
}

#[test]
fn reactive_session_component_handles_escape_sidebar_focus_clear() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Sidebar Escape".to_string()));
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "sidebar escape".to_string(),
                vec![MessagePart::Text {
                    text: "sidebar escape".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = context.ensure_session_view_handle(&session_id);
    view.toggle_sidebar(crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1));
    view.toggle_sidebar_workspace_focus(
        crate::context::SESSION_SIDEBAR_WIDE_THRESHOLD.saturating_sub(1),
    );
    let area = Rect::new(0, 0, 72, 16);

    let (first, _) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let (_second, sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))),
    );
    let sidebar = sidebar.expect("sidebar state should be captured");

    assert!(!sidebar.lifecycle.workspace_focus);
    assert!(!sidebar.lifecycle.attached_session_focus);
    assert!(!sidebar.lifecycle.process_focus);
}

#[test]
fn reactive_session_component_handles_scrollbar_drag_without_event_loop() {
    let (context, session_id, _snapshot) = perf_snapshot_with_messages();
    context.toggle_scrollbar();
    context.navigate_session(session_id.clone());
    let area = Rect::new(0, 0, 72, 10);

    let (first, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    let scrollbar_area = first
        .viewport
        .last_scrollbar_area
        .expect("scrollbar area should be captured");

    let (clicked, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &first.viewport,
        &first.reasoning,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: scrollbar_area.x,
            row: scrollbar_area.y.saturating_add(scrollbar_area.height.saturating_sub(1)),
            modifiers: KeyModifiers::NONE,
        })),
    );
    assert!(
        clicked.viewport.scrollbar_drag_active,
        "scrollbar click should start a reactive drag state"
    );
    assert!(
        clicked.viewport.scroll_offset > first.viewport.scroll_offset,
        "scrollbar click should update scroll offset inside the reactive session component"
    );

    let (released, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &clicked.viewport,
        &clicked.reasoning,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: scrollbar_area.x,
            row: scrollbar_area.y.saturating_add(scrollbar_area.height.saturating_sub(1)),
            modifiers: KeyModifiers::NONE,
        })),
    );
    assert!(
        !released.viewport.scrollbar_drag_active,
        "scrollbar mouse-up should end the reactive drag state"
    );
}

#[test]
fn session_view_surfaces_hidden_reasoning_hint_when_display_thinking_is_disabled() {
    let context = Arc::new(AppContext::new());
    context.apply_config(&agendao_config::Config {
        ui_preferences: Some(agendao_config::UiPreferencesConfig {
            show_thinking: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    });
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Hidden Reasoning Hint".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "please think first".to_string(),
                    vec![MessagePart::Text {
                        text: "please think first".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "done".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: [
                                "hidden reasoning step one",
                                "hidden reasoning step two",
                                "hidden reasoning step three",
                            ]
                            .join("\n"),
                        },
                        MessagePart::Text {
                            text: "done".to_string(),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("▶ reasoning"),
        "hidden reasoning should keep reasoning triangle header semantics:\n{rendered}"
    );
    assert!(
        rendered.contains("[ Expand ]"),
        "hidden reasoning should surface an inline expand affordance:\n{rendered}"
    );
    assert!(
        !rendered.contains("reasoning hidden by display preference"),
        "hidden reasoning should no longer render the old placeholder prose:\n{rendered}"
    );
}

#[test]
fn hidden_reasoning_header_click_expands_reasoning_block() {
    let context = Arc::new(AppContext::new());
    context.apply_config(&agendao_config::Config {
        ui_preferences: Some(agendao_config::UiPreferencesConfig {
            show_thinking: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    });
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Hidden Reasoning Expand".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "done".to_string(),
                vec![
                    MessagePart::Reasoning {
                        text: [
                            "hidden reasoning step one",
                            "hidden reasoning step two",
                            "hidden reasoning step three",
                        ]
                        .join("\n"),
                    },
                    MessagePart::Text {
                        text: "done".to_string(),
                    },
                ],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);

    let initial = render_session_view_once(&view, &context, area, &prompt);
    let initial_text = buffer_text(&initial);
    assert!(initial_text.contains("▶ reasoning"), "{initial_text}");
    assert!(initial_text.contains("[ Expand ]"), "{initial_text}");
    assert!(!initial_text.contains("hidden reasoning step three"), "{initial_text}");

    let messages_area = view
        .state
        .lock()
        .viewport
        .last_messages_area
        .expect("messages area");
    assert!(view.handle_click(messages_area.x + 2, messages_area.y));

    let expanded = render_session_view_once(&view, &context, area, &prompt);
    let expanded_text = buffer_text(&expanded);
    assert!(
        expanded_text.contains("hidden reasoning step one")
            || expanded_text.contains("hidden reasoning step two"),
        "{expanded_text}"
    );
    assert!(expanded_text.contains("hidden reasoning step three"), "{expanded_text}");
    assert!(expanded_text.contains("┆ collapse"), "{expanded_text}");
}

#[test]
fn plain_message_body_click_is_not_marked_as_consumed_left_click() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Plain Body".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "plain body line one\nplain body line two".to_string(),
                vec![MessagePart::Text {
                    text: "plain body line one\nplain body line two".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);

    let _ = render_session_view_once(&view, &context, area, &prompt);
    let messages_area = view.selection_area().expect("messages area");

    assert!(!view.consumes_left_click(messages_area.x + 8, messages_area.y + 1));
    assert!(view.contains_messages_point(messages_area.x + 8, messages_area.y + 1));
}

#[test]
fn plain_message_body_click_requests_scoped_selection_start() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Plain Body Selection".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "plain body line one\nplain body line two".to_string(),
                vec![MessagePart::Text {
                    text: "plain body line one\nplain body line two".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);

    let _ = render_session_view_once(&view, &context, area, &prompt);
    let messages_area = view.selection_area().expect("messages area");

    assert_eq!(
        view.left_mouse_down_outcome(messages_area.x + 8, messages_area.y + 1),
        super::SessionLeftMouseDownOutcome::BeginSelection {
            area: messages_area,
        }
    );
}

#[test]
fn session_sidebar_open_button_click_uses_session_view_authority() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Sidebar Open".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "hello".to_string(),
                vec![MessagePart::Text {
                    text: "hello".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    {
        let mut state = view.state.lock();
        state.sidebar.lifecycle.mode = crate::context::SidebarMode::Hide;
        state.sidebar.lifecycle.visible = false;
    }
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);

    let _ = render_session_view_once(&view, &context, area, &prompt);
    let button = view
        .state
        .lock()
        .sidebar
        .open_button_area
        .expect("open button area");

    assert!(view.handle_click(button.x, button.y));
    assert!(view.sidebar_visible(area.width));
}

#[test]
fn scrollbar_drag_and_mouse_up_are_marked_as_consumed_by_session_view() {
    let (context, session_id, _snapshot) = perf_snapshot_with_messages();
    context.toggle_scrollbar();
    context.navigate_session(session_id.clone());
    let view = context.ensure_session_view_handle(&session_id);
    let area = Rect::new(0, 0, 72, 10);

    let (first, _sidebar) = render_reactive_session_messages_with_event(
        &context,
        &session_id,
        area,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        None,
    );
    {
        let mut state = view.state.lock();
        state.viewport = first.viewport.clone();
    }
    let scrollbar_area = first
        .viewport
        .last_scrollbar_area
        .expect("scrollbar area should be captured");

    assert!(view.handle_scrollbar_click(
        scrollbar_area.x,
        scrollbar_area.y.saturating_add(scrollbar_area.height.saturating_sub(1))
    ));
    assert!(view.consumes_left_drag(
        scrollbar_area.x,
        scrollbar_area.y.saturating_add(scrollbar_area.height.saturating_sub(1))
    ));
    assert!(view.consumes_left_mouse_up());
}

#[test]
fn assistant_segments_render_with_block_spacing_between_reasoning_tool_and_text() {
    let (_context, _session_id, mut snapshot) = perf_snapshot_with_messages();
    snapshot.messages = vec![make_message(
        "assistant-1",
        MessageRole::Assistant,
        "final answer".to_string(),
        vec![
            MessagePart::Reasoning {
                text: "thinking".to_string(),
            },
            MessagePart::ToolCall {
                id: "tool-1".to_string(),
                name: "search".to_string(),
                arguments: "{}".to_string(),
            },
            MessagePart::Text {
                text: "final answer".to_string(),
            },
        ],
    )];

    let output = render_perf_session_messages(
        Rect::new(0, 0, 78, 30),
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    let rendered_lines = output
        .message_cache
        .entries
        .get("assistant-1")
        .expect("assistant cache entry")
        .output
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let blank_lines = rendered_lines
        .iter()
        .filter(|line| line.trim().is_empty())
        .count();
    assert!(
        blank_lines >= 2,
        "assistant semantic blocks should have visible spacing between them: {rendered_lines:?}"
    );
    let mut max_consecutive_blank_lines = 0usize;
    let mut current_blank_run = 0usize;
    for line in &rendered_lines {
        if line.trim().is_empty() {
            current_blank_run += 1;
            max_consecutive_blank_lines = max_consecutive_blank_lines.max(current_blank_run);
        } else {
            current_blank_run = 0;
        }
    }
    assert_eq!(
        max_consecutive_blank_lines, 1,
        "adjacent assistant blocks should be separated by a single blank line: {rendered_lines:?}"
    );
}

#[test]
fn reasoning_block_has_no_extra_outer_blank_lines() {
    let (_context, _session_id, mut snapshot) = perf_snapshot_with_messages();
    snapshot.messages = vec![make_message(
        "assistant-1",
        MessageRole::Assistant,
        "final answer".to_string(),
        vec![
            MessagePart::Reasoning {
                text: "step one\nstep two\nstep three".to_string(),
            },
            MessagePart::Text {
                text: "final answer".to_string(),
            },
        ],
    )];

    let output = render_perf_session_messages(
        Rect::new(0, 0, 78, 30),
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    let rendered_lines = output
        .message_cache
        .entries
        .get("assistant-1")
        .expect("assistant cache entry")
        .output
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let reasoning_start = rendered_lines
        .iter()
        .position(|line| line.contains("reasoning"))
        .expect("reasoning header");
    assert!(
        !rendered_lines[reasoning_start].trim().is_empty(),
        "{rendered_lines:?}"
    );
    assert!(
        !rendered_lines
            .get(reasoning_start + 1)
            .expect("reasoning body")
            .trim()
            .is_empty(),
        "{rendered_lines:?}"
    );
}

#[test]
fn session_render_preserves_transcript_order_across_assistant_and_tool_messages() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Transcript Order".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "find papers".to_string(),
                    vec![MessagePart::Text {
                        text: "find papers".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "thinking".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: "thinking".to_string(),
                        },
                        MessagePart::ToolCall {
                            id: "tool-1".to_string(),
                            name: "websearch".to_string(),
                            arguments: "{\"query\":\"papers\"}".to_string(),
                        },
                    ],
                ),
                make_message(
                    "tool-1",
                    MessageRole::Tool,
                    "papers found".to_string(),
                    vec![MessagePart::ToolResult {
                        id: "tool-1".to_string(),
                        result: "papers found".to_string(),
                        is_error: false,
                        title: Some("websearch".to_string()),
                        metadata: None,
                    }],
                ),
                make_message(
                    "assistant-2",
                    MessageRole::Assistant,
                    "final answer".to_string(),
                    vec![MessagePart::Text {
                        text: "final answer".to_string(),
                    }],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 30);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let transcript = buffer_text(&buffer);

    let reasoning_idx = transcript.find("reasoning").expect("reasoning visible");
    let tool_idx = transcript.find("websearch").expect("tool visible");
    let final_idx = transcript.rfind("final answer").expect("assistant text visible");
    assert!(
        reasoning_idx < tool_idx && tool_idx < final_idx,
        "transcript should keep reasoning -> tool -> final text order:\n{transcript}"
    );
}

#[test]
fn session_view_keeps_reasoning_anchor_when_same_assistant_grows_with_more_blocks() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Reasoning Anchor".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "investigate this author".to_string(),
                    vec![MessagePart::Text {
                        text: "investigate this author".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "final answer".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: multiline_reasoning_block("anchor reasoning", 6),
                        },
                        MessagePart::ToolCall {
                            id: "tool-1".to_string(),
                            name: "search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        MessagePart::Text {
                            text: "final answer".to_string(),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());
    let view = SessionView::new(session_id.clone());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 18);

    let first = render_session_view_once(&view, &context, area, &prompt);
    let first_rendered = buffer_text(&first);
    assert!(
        first_rendered.contains("reasoning"),
        "initial render should show reasoning header:\n{first_rendered}"
    );

    {
        let mut session = context.session.write();
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "investigate this author".to_string(),
                    vec![MessagePart::Text {
                        text: "investigate this author".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    long_block("assistant-final", 20),
                    vec![
                        MessagePart::Reasoning {
                            text: multiline_reasoning_block("anchor reasoning", 6),
                        },
                        MessagePart::ToolCall {
                            id: "tool-1".to_string(),
                            name: "search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        MessagePart::ToolCall {
                            id: "tool-2".to_string(),
                            name: "skill_search".to_string(),
                            arguments: "{\"query\":\"pubmed\"}".to_string(),
                        },
                        MessagePart::Text {
                            text: long_block("assistant-final", 20),
                        },
                    ],
                ),
            ],
        );
    }

    let second = render_session_view_once(&view, &context, area, &prompt);
    let second_rendered = buffer_text(&second);
    assert!(
        second_rendered.contains("reasoning"),
        "reasoning header should remain visible when the same assistant message grows:\n{second_rendered}"
    );
}

#[test]
fn overlay_sidebar_backdrop_click_closes_sidebar() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    view.toggle_sidebar(100);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 100, 30);
    assert!(view.sidebar_visible(area.width));
    render_reactive_session_view_with_event(&context, &view, &prompt, area, None);
    render_reactive_session_view_with_event(
        &context,
        &view,
        &prompt,
        area,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::empty(),
        })),
    );
    assert!(!view.sidebar_visible(area.width));
}

#[test]
fn docked_sidebar_close_button_click_closes_sidebar() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 140, 30);
    render_reactive_session_view_with_event(&context, &view, &prompt, area, None);

    let close_button = view
        .state
        .lock()
        .sidebar
        .close_button_area
        .expect("docked sidebar close button");
    assert!(view.sidebar_visible(area.width));
    render_reactive_session_view_with_event(
        &context,
        &view,
        &prompt,
        area,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: close_button.x,
            row: close_button.y,
            modifiers: KeyModifiers::empty(),
        })),
    );
    assert!(!view.sidebar_visible(area.width));
}

#[test]
fn hidden_sidebar_open_button_click_opens_sidebar_reactively() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 100, 30);

    render_reactive_session_view_with_event(&context, &view, &prompt, area, None);

    let open_button = view
        .state
        .lock()
        .sidebar
        .open_button_area
        .expect("hidden sidebar open button");
    assert!(!view.sidebar_visible(area.width));
    render_reactive_session_view_with_event(
        &context,
        &view,
        &prompt,
        area,
        Some(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: open_button.x,
            row: open_button.y,
            modifiers: KeyModifiers::empty(),
        })),
    );
    assert!(view.sidebar_visible(area.width));
}

#[test]
fn session_messages_area_uses_full_main_width_without_outer_inset() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 100, 30);
    let mut buffer = Buffer::empty(area);
    let mut surface = BufferSurface::new(&mut buffer);

    view.render(&context, &mut surface, area, &prompt);

    let messages_area = view
        .state
        .lock()
        .viewport
        .last_messages_area
        .expect("messages area");
    assert_eq!(messages_area.x, area.x);
}

#[test]
fn session_messages_start_below_padded_header_in_wide_layout() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 140, 30);
    let mut buffer = Buffer::empty(area);
    let mut surface = BufferSurface::new(&mut buffer);

    view.render(&context, &mut surface, area, &prompt);

    let messages_area = view
        .state
        .lock()
        .viewport
        .last_messages_area
        .expect("messages area");
    assert_eq!(messages_area.y, area.y.saturating_add(3));
}

#[test]
fn session_render_model_memo_key_tracks_width_and_reasoning_state() {
    let context = Arc::new(AppContext::new());
    let snapshot = SessionMessagesSnapshot::from_seed(
        &SessionMessagesSnapshotSeed::capture(&context, "session-1"),
    );
    let empty_reasoning = SessionReasoningState::default();

    let base = build_session_render_model_memo_key(&snapshot, 80, &empty_reasoning);
    let same = build_session_render_model_memo_key(&snapshot, 80, &empty_reasoning);
    assert_eq!(base, same);

    let mut expanded_reasoning = SessionReasoningState::default();
    expanded_reasoning
        .expanded
        .insert("message-1:0".to_string());
    assert_ne!(
        base,
        build_session_render_model_memo_key(&snapshot, 80, &expanded_reasoning)
    );
    assert_ne!(
        base,
        build_session_render_model_memo_key(&snapshot, 79, &empty_reasoning)
    );
}

#[test]
fn session_render_model_memo_key_tracks_same_length_text_changes() {
    let (_context, _session_id, mut snapshot) = perf_snapshot_with_messages();
    let empty_reasoning = SessionReasoningState::default();
    let base = build_session_render_model_memo_key(&snapshot, 72, &empty_reasoning);

    let message = snapshot
        .messages
        .iter_mut()
        .find(|msg| msg.id == "assistant-2")
        .expect("assistant-2 message");
    let replacement = "Z".repeat(message.content.len());
    message.content = replacement;
    if let Some(MessagePart::Text { text }) = message
        .parts
        .iter_mut()
        .find(|part| matches!(part, MessagePart::Text { .. }))
    {
        *text = "Y".repeat(text.len());
    } else {
        panic!("assistant-2 should have a text part");
    }

    let changed = build_session_render_model_memo_key(&snapshot, 72, &empty_reasoning);
    assert_ne!(base, changed);
}

#[test]
fn session_render_model_cache_reuses_model_on_identical_inputs() {
    let context = Arc::new(AppContext::new());
    let snapshot = SessionMessagesSnapshot::from_seed(
        &SessionMessagesSnapshotSeed::capture(&context, "session-1"),
    );
    let area = Rect::new(0, 0, 80, 20);
    let mut buffer = Buffer::empty(area);

    let reasoning = SessionReasoningState::default();
    let (model, _, cache) = resolve_session_render_model(
        area,
        &snapshot,
        &reasoning,
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
        &mut buffer,
    );
    let (reused_model, _, _) = resolve_session_render_model(
        area,
        &snapshot,
        &reasoning,
        &SessionMessageOutputCache::default(),
        &cache,
        &mut buffer,
    );

    assert!(Arc::ptr_eq(&model, &reused_model));
}

#[test]
fn session_render_model_cache_rebuilds_on_same_length_text_change() {
    let (_context, _session_id, snapshot) = perf_snapshot_with_messages();
    let area = Rect::new(0, 0, 72, 24);
    let first = render_perf_session_messages(
        area,
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    let mut changed_snapshot = snapshot.clone();
    let message = changed_snapshot
        .messages
        .iter_mut()
        .find(|msg| msg.id == "assistant-2")
        .expect("assistant-2 message");
    let replacement = "Q".repeat(message.content.len());
    message.content = replacement;
    if let Some(MessagePart::Text { text }) = message
        .parts
        .iter_mut()
        .find(|part| matches!(part, MessagePart::Text { .. }))
    {
        *text = "R".repeat(text.len());
    } else {
        panic!("assistant-2 should have a text part");
    }

    reset_session_render_perf_counters();
    let second = render_perf_session_messages(
        area,
        &changed_snapshot,
        &first.viewport,
        &first.reasoning,
        &first.message_cache,
        &first.render_model_cache,
    );
    let counters = snapshot_session_render_perf_counters();

    assert_ne!(
        first.render_model_cache.memo_key,
        second.render_model_cache.memo_key
    );
    assert_ne!(
        first.viewport.render_model_memo_key,
        second.viewport.render_model_memo_key
    );
    assert_eq!(counters.render_model_cache_hits, 0);
    assert_eq!(counters.render_model_rebuilds, 1);
}

#[test]
fn session_viewport_content_memo_key_tracks_scroll_and_height() {
    let base = build_session_viewport_content_memo_key(42, 10, 20);
    assert_eq!(base, build_session_viewport_content_memo_key(42, 10, 20));
    assert_ne!(base, build_session_viewport_content_memo_key(42, 11, 20));
    assert_ne!(base, build_session_viewport_content_memo_key(42, 10, 21));
    assert_ne!(base, build_session_viewport_content_memo_key(43, 10, 20));
}

#[test]
fn message_render_output_clone_shares_line_storage() {
    let output = super::MessageRenderOutput::new(vec![ratatui::text::Line::from("hello")]);
    let cloned = output.clone();
    assert!(Arc::ptr_eq(&output.lines, &cloned.lines));
}

#[test]
fn scroll_only_reuses_render_model_and_skips_message_rebuilds() {
    let (_context, _session_id, snapshot) = perf_snapshot_with_messages();
    let area = Rect::new(0, 0, 72, 10);
    let first = render_perf_session_messages(
        area,
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    let mut scrolled_viewport = first.viewport.clone();
    scrolled_viewport.scroll_offset = scrolled_viewport.scroll_offset.saturating_sub(6);
    reset_session_render_perf_counters();
    let second = render_perf_session_messages(
        area,
        &snapshot,
        &scrolled_viewport,
        &first.reasoning,
        &first.message_cache,
        &first.render_model_cache,
    );
    let counters = snapshot_session_render_perf_counters();

    assert_eq!(counters.render_model_cache_hits, 1);
    assert_eq!(counters.render_model_rebuilds, 0);
    assert_eq!(counters.message_cache_hits, 0);
    assert_eq!(counters.message_cache_misses, 0);
    assert_eq!(counters.visible_range_recomputes, 1);
    assert!(counters.visible_lines_written > 0);
    assert_eq!(
        second.render_model_cache.memo_key,
        first.render_model_cache.memo_key
    );
    assert_eq!(
        second.viewport.render_model_memo_key,
        first.viewport.render_model_memo_key
    );
}

#[test]
fn snapshot_capture_uses_session_scoped_compaction_authority() {
    let context = Arc::new(AppContext::new());
    let (active_session_id, target_session_id) = {
        let mut session = context.session.write();
        let active_session_id = session.create_session(Some("Active".to_string()));
        let target_session_id = session.create_session(Some("Target".to_string()));
        session.set_messages(
            &target_session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "final answer".to_string(),
                vec![MessagePart::Text {
                    text: "final answer".to_string(),
                }],
            )],
        );
        session.set_status(&target_session_id, SessionStatus::Compacting);
        (active_session_id, target_session_id)
    };

    context.navigate_session(active_session_id);
    context.apply_session_projection_snapshot(
        &target_session_id,
        None,
        Vec::new(),
        None,
        None,
        Some(agendao_types::ContextCompactionSummary {
            trigger: "auto".to_string(),
            phase: Some("pre_request".to_string()),
            reason: Some("context_pressure".to_string()),
            forced: false,
            request_context_tokens: Some(58_000),
            live_context_tokens: Some(58_000),
            limit_tokens: Some(100_000),
            body_chars: None,
            message_count_before: None,
            compacted_message_count: None,
            kept_message_count: None,
            summary: None,
        }),
        Some(agendao_types::ContextCompactionLifecycleSummary {
            trigger: "auto".to_string(),
            phase: Some("pre_request".to_string()),
            reason: Some("context_pressure".to_string()),
            status: agendao_types::ContextCompactionLifecycleStatus::Started,
            forced: false,
            request_context_tokens: Some(58_000),
            live_context_tokens: Some(58_000),
            limit_tokens: Some(100_000),
            body_chars: None,
            installed: None,
        }),
        None,
        None,
    );

    let snapshot = SessionMessagesSnapshot::from_seed(
        &SessionMessagesSnapshotSeed::capture(&context, &target_session_id),
    );
    let last = snapshot
        .messages
        .last()
        .expect("snapshot should include synthetic compaction message");

    assert!(
        last.content.contains("Compacting conversation"),
        "snapshot should use session-scoped projection authority even when another route is active"
    );
}

#[test]
fn snapshot_seed_and_key_preserve_session_scoped_authority() {
    let context = Arc::new(AppContext::new());
    let (active_session_id, target_session_id) = {
        let mut session = context.session.write();
        let active_session_id = session.create_session(Some("Active".to_string()));
        let target_session_id = session.create_session(Some("Target".to_string()));
        session.set_messages(
            &target_session_id,
            vec![make_message(
                "assistant-1",
                MessageRole::Assistant,
                "final answer".to_string(),
                vec![MessagePart::Text {
                    text: "final answer".to_string(),
                }],
            )],
        );
        session.set_status(&target_session_id, SessionStatus::Compacting);
        (active_session_id, target_session_id)
    };

    context.navigate_session(active_session_id);
    context.apply_session_projection_snapshot(
        &target_session_id,
        None,
        Vec::new(),
        None,
        None,
        Some(agendao_types::ContextCompactionSummary {
            trigger: "auto".to_string(),
            phase: Some("pre_request".to_string()),
            reason: Some("context_pressure".to_string()),
            forced: false,
            request_context_tokens: Some(58_000),
            live_context_tokens: Some(58_000),
            limit_tokens: Some(100_000),
            body_chars: None,
            message_count_before: None,
            compacted_message_count: None,
            kept_message_count: None,
            summary: None,
        }),
        None,
        None,
        None,
    );

    let seed = super::SessionMessagesSnapshotSeed::capture(&context, &target_session_id);
    let key = super::SessionMessagesSnapshotKey::capture(&context, &target_session_id);
    let snapshot = super::SessionMessagesSnapshot::from_seed(&seed);

    assert_eq!(key.session_id, target_session_id);
    assert_eq!(seed.session_id, target_session_id);
    assert!(
        snapshot
            .messages
            .last()
            .expect("synthetic compaction message")
            .content
            .contains("Compacting conversation"),
        "seed-derived snapshot should keep session-scoped authority even when another route is active"
    );
}

#[test]
fn session_view_header_uses_session_scoped_runtime_authority() {
    let context = Arc::new(AppContext::new());
    let (active_session_id, target_session_id) = {
        let mut session = context.session.write();
        let active_session_id = session.create_session(Some("Active".to_string()));
        let target_session_id = session.create_session(Some("Target".to_string()));
        session.set_status(&target_session_id, SessionStatus::Running);
        (active_session_id, target_session_id)
    };

    context.navigate_session(active_session_id.clone());
    context.apply_session_runtime_snapshot(SessionRuntimeState {
        session_id: active_session_id.clone(),
        run_status: SessionRunStatusKind::Idle,
        current_message_id: None,
        usage: None,
        active_stage_id: None,
        active_stage_count: 0,
        active_tools: Vec::new(),
        pending_question: None,
        pending_permission: Some(PendingPermissionSummary {
            permission_id: "perm_1".to_string(),
            requested_at: 1,
            tool: Some("bash".to_string()),
        }),
        pending_followup_count: 0,
        attached_sessions: Vec::new(),
    });

    let view = SessionView::new(target_session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("◐ Target"),
        "target session header should keep its own running authority:\n{rendered}"
    );
    assert!(
        !rendered.contains("AWAITING PERMISSION"),
        "active route permission should not leak into a different session header:\n{rendered}"
    );
}

#[test]
fn session_render_seed_preserves_session_scoped_runtime_authority() {
    let context = Arc::new(AppContext::new());
    let (active_session_id, target_session_id) = {
        let mut session = context.session.write();
        let active_session_id = session.create_session(Some("Active".to_string()));
        let target_session_id = session.create_session(Some("Target".to_string()));
        session.set_status(&target_session_id, SessionStatus::Running);
        (active_session_id, target_session_id)
    };

    context.navigate_session(active_session_id.clone());
    context.apply_session_runtime_snapshot(SessionRuntimeState {
        session_id: active_session_id.clone(),
        run_status: SessionRunStatusKind::Idle,
        current_message_id: None,
        usage: None,
        active_stage_id: None,
        active_stage_count: 0,
        active_tools: Vec::new(),
        pending_question: None,
        pending_permission: Some(PendingPermissionSummary {
            permission_id: "perm_1".to_string(),
            requested_at: 1,
            tool: Some("bash".to_string()),
        }),
        pending_followup_count: 0,
        attached_sessions: Vec::new(),
    });

    let seed = SessionRenderSnapshotSeed::capture(&context, &target_session_id);
    let snapshot = SessionRenderSnapshot::from_seed(&seed);

    assert!(snapshot.header.status_running);
    assert_eq!(snapshot.header.title, "Target");
    assert_ne!(
        snapshot.header.status_label.as_deref(),
        Some("AWAITING PERMISSION"),
        "active route permission should not leak into another session seed"
    );
}

#[test]
fn reasoning_toggle_rebuilds_only_affected_message_output() {
    let (_context, _session_id, snapshot) = perf_snapshot_with_messages();
    let area = Rect::new(0, 0, 72, 10);
    let mut completed_snapshot = snapshot.clone();
    if let Some(message) = completed_snapshot.messages.get_mut(1) {
        message.completed_at = Some(Utc::now());
    }
    let first = render_perf_session_messages(
        area,
        &completed_snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    let reasoning_id = first
        .reasoning
        .toggle_hits
        .first()
        .map(|hit| hit.reasoning_id.clone())
        .expect("collapsed reasoning toggle should be present");
    let mut expanded_reasoning = first.reasoning.clone();
    expanded_reasoning.expanded.insert(reasoning_id.clone());
    reset_session_render_perf_counters();
    let second = render_perf_session_messages(
        area,
        &completed_snapshot,
        &first.viewport,
        &expanded_reasoning,
        &first.message_cache,
        &first.render_model_cache,
    );
    let counters = snapshot_session_render_perf_counters();

    assert_eq!(counters.render_model_cache_hits, 0);
    assert_eq!(counters.render_model_rebuilds, 1);
    assert_eq!(counters.message_cache_hits, 5);
    assert_eq!(counters.message_cache_misses, 1);
    assert_eq!(counters.visible_range_recomputes, 1);
    assert!(counters.visible_lines_written > 0);
    assert!(
        second
            .reasoning
            .toggle_hits
            .iter()
            .any(|hit| hit.reasoning_id == reasoning_id)
    );
    assert!(
        second.viewport.rendered_line_count > first.viewport.rendered_line_count,
        "expanded reasoning should increase rendered line count"
    );
}

#[test]
fn live_reasoning_defaults_to_expanded_without_toggle() {
    let (_context, _session_id, snapshot) = perf_snapshot_with_messages();
    let area = Rect::new(0, 0, 72, 18);

    let output = render_perf_session_messages(
        area,
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );

    assert!(
        output.viewport.rendered_line_count > 18,
        "live reasoning should render expanded body lines by default"
    );
    assert!(
        output.reasoning.toggle_hits.iter().any(|hit| hit.reasoning_id.starts_with("assistant-1:")),
        "live reasoning should remain toggleable"
    );
}

#[test]
fn scroll_to_message_uses_compact_message_first_line_index() {
    let (context, session_id, snapshot) = perf_snapshot_with_messages();
    let view = SessionView::new(session_id);
    let output = render_perf_session_messages(
        Rect::new(0, 0, 72, 10),
        &snapshot,
        &SessionMessageViewportState::default(),
        &SessionReasoningState::default(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
    );
    {
        let mut state = view.state.lock();
        state.viewport = output.viewport.clone();
    }

    let expected_scroll_offset = output
        .viewport
        .message_first_lines
        .get("assistant-2")
        .copied()
        .expect("message first line should be indexed after render")
        .min(
            output
                .viewport
                .rendered_line_count
                .saturating_sub(output.viewport.messages_viewport_height),
        );

    view.scroll_to_message(&context, "assistant-2");

    let state = view.state.lock();
    assert_eq!(state.viewport.scroll_offset, expected_scroll_offset);
}

#[test]
fn session_view_reuses_message_snapshot_cache_when_snapshot_key_is_unchanged() {
    let (context, session_id, _snapshot) = perf_snapshot_with_messages();
    let view = SessionView::new(session_id.clone());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 72, 18);

    let _ = render_session_view_once(&view, &context, area, &prompt);
    let first_cache = {
        let state = view.state.lock();
        (
            state.snapshot_cache_key.clone(),
            state.snapshot_cache.clone(),
        )
    };

    let _ = render_session_view_once(&view, &context, area, &prompt);
    let second_cache = {
        let state = view.state.lock();
        (
            state.snapshot_cache_key.clone(),
            state.snapshot_cache.clone(),
        )
    };

    assert!(first_cache.0.is_some(), "snapshot cache key should be populated");
    assert!(second_cache.0.is_some(), "second snapshot cache key should be populated");
    assert!(first_cache.0 == second_cache.0, "snapshot cache key should be stable");
    assert!(first_cache.1.is_some(), "snapshot cache should be populated");
    let first_snapshot = first_cache.1.expect("first snapshot cache");
    let second_snapshot = second_cache.1.expect("second snapshot cache");
    assert_eq!(first_snapshot.messages.len(), second_snapshot.messages.len());
    assert_eq!(first_snapshot.directory, second_snapshot.directory);
}

#[test]
fn assistant_blocks_render_with_widget_backed_left_border() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Widget Block".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![
                make_message(
                    "user-1",
                    MessageRole::User,
                    "show me the answer".to_string(),
                    vec![MessagePart::Text {
                        text: "show me the answer".to_string(),
                    }],
                ),
                make_message(
                    "assistant-1",
                    MessageRole::Assistant,
                    "final answer".to_string(),
                    vec![
                        MessagePart::Reasoning {
                            text: "reasoning line one\nreasoning line two".to_string(),
                        },
                        MessagePart::Text {
                            text: "final answer".to_string(),
                        },
                    ],
                ),
            ],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 24);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("│ ▼ reasoning") || rendered.contains("│  ▼ reasoning"),
        "reasoning block should keep the left border shell:\n{rendered}"
    );
    assert!(
        rendered.contains("│ ☪ final answer") || rendered.contains("│  ☪ final answer"),
        "assistant text block should keep the left border shell:\n{rendered}"
    );
}

#[test]
fn user_blocks_render_with_widget_backed_left_border() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("User Block".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "user-1",
                MessageRole::User,
                "show me the answer".to_string(),
                vec![MessagePart::Text {
                    text: "show me the answer".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 18);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("│ show me the answer") || rendered.contains("│  show me the answer"),
        "user block should render through the unified left border shell:\n{rendered}"
    );
}

#[test]
fn tool_messages_render_with_widget_backed_left_border() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Tool Block".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "tool-1",
                MessageRole::Tool,
                "tool output".to_string(),
                vec![
                    MessagePart::ToolCall {
                        id: "call_1".to_string(),
                        name: "bash".to_string(),
                        arguments: "echo hi".to_string(),
                    },
                    MessagePart::ToolResult {
                        id: "call_1".to_string(),
                        result: "hi".to_string(),
                        is_error: false,
                        title: Some("bash".to_string()),
                        metadata: None,
                    },
                ],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 90, 22);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("│") && rendered.contains("bash"),
        "tool message should render through the unified left border shell:\n{rendered}"
    );
}

#[test]
fn plain_messages_render_with_widget_backed_left_border() {
    let context = Arc::new(AppContext::new());
    let session_id = {
        let mut session = context.session.write();
        let session_id = session.create_session(Some("Plain Block".to_string()));
        session.set_current_session_id(session_id.clone());
        session.set_messages(
            &session_id,
            vec![make_message(
                "system-1",
                MessageRole::System,
                "plain body line one\nplain body line two".to_string(),
                vec![MessagePart::Text {
                    text: "plain body line one\nplain body line two".to_string(),
                }],
            )],
        );
        session_id
    };
    context.navigate_session(session_id.clone());

    let view = SessionView::new(session_id);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 78, 18);
    let buffer = render_session_view_once(&view, &context, area, &prompt);
    let rendered = buffer_text(&buffer);

    assert!(
        rendered.contains("│ plain body line one") || rendered.contains("│  plain body line one"),
        "plain message should render through the unified left border shell:\n{rendered}"
    );
}

#[test]
fn synthetic_compaction_message_is_only_present_while_compacting() {
    let summary = agendao_types::ContextCompactionSummary {
        trigger: "auto".to_string(),
        phase: Some("pre_request".to_string()),
        reason: Some("context_pressure".to_string()),
        forced: false,
        request_context_tokens: Some(58_000),
        live_context_tokens: Some(58_000),
        limit_tokens: Some(100_000),
        body_chars: None,
        message_count_before: None,
        compacted_message_count: None,
        kept_message_count: None,
        summary: None,
    };

    let message = super::synthetic_compaction_message(
        "ses_1",
        (
            crate::context::SessionStatus::Compacting,
            Some(summary.clone()),
            Some(agendao_types::ContextCompactionLifecycleSummary {
                trigger: "auto".to_string(),
                phase: Some("pre_request".to_string()),
                reason: Some("context_pressure".to_string()),
                status: agendao_types::ContextCompactionLifecycleStatus::Started,
                forced: false,
                request_context_tokens: Some(58_000),
                live_context_tokens: Some(58_000),
                limit_tokens: Some(100_000),
                body_chars: None,
                installed: None,
            }),
        ),
    )
    .expect("compaction block should render while compacting");

    assert!(message.content.contains("Compacting conversation"));
    assert!(message.content.contains("compressing"));
    assert!(message.content.contains("58K"));
    assert!(message.content.contains("context pressure"));

    assert!(
        super::synthetic_compaction_message(
            "ses_1",
            (crate::context::SessionStatus::Running, Some(summary), None)
        )
        .is_none()
    );
}
