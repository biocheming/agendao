use super::{
    SessionMessageOutputCache, SessionMessageViewportState, SessionMessagesSnapshot,
    SessionReasoningState, SessionRenderModelCache, SessionView,
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
    with_render_context_mut,
};

use crate::{
    components::Prompt,
    context::{AppContext, Message, MessagePart, MessageRole, SessionStatus, TokenUsage},
    ui::BufferSurface,
};
use std::collections::HashSet;
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
    let snapshot = SessionMessagesSnapshot::capture(&context, &session_id);
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
fn overlay_sidebar_backdrop_click_closes_sidebar() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    view.toggle_sidebar(100);
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 100, 30);
    let mut buffer = Buffer::empty(area);
    let mut surface = BufferSurface::new(&mut buffer);

    view.render(&context, &mut surface, area, &prompt);

    assert!(view.sidebar_visible(area.width));
    assert!(view.handle_sidebar_click(&context, 1, 1));
    assert!(!view.sidebar_visible(area.width));
}

#[test]
fn docked_sidebar_close_button_click_closes_sidebar() {
    let context = Arc::new(AppContext::new());
    let view = SessionView::new("session-1".to_string());
    let prompt = Prompt::new(context.clone())
        .with_placeholder("Ask anything... \"Fix a TODO in the codebase\"");
    let area = Rect::new(0, 0, 140, 30);
    let mut buffer = Buffer::empty(area);
    let mut surface = BufferSurface::new(&mut buffer);

    view.render(&context, &mut surface, area, &prompt);

    let close_button = view
        .state
        .lock()
        .sidebar
        .close_button_area
        .expect("docked sidebar close button");
    assert!(view.sidebar_visible(area.width));
    assert!(view.handle_sidebar_click(&context, close_button.x, close_button.y));
    assert!(!view.sidebar_visible(area.width));
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
    let snapshot = SessionMessagesSnapshot::capture(&context, "session-1");

    let base = build_session_render_model_memo_key(&snapshot, 80, &HashSet::new());
    let same = build_session_render_model_memo_key(&snapshot, 80, &HashSet::new());
    assert_eq!(base, same);

    let mut expanded = HashSet::new();
    expanded.insert("message-1:0".to_string());
    assert_ne!(
        base,
        build_session_render_model_memo_key(&snapshot, 80, &expanded)
    );
    assert_ne!(
        base,
        build_session_render_model_memo_key(&snapshot, 79, &HashSet::new())
    );
}

#[test]
fn session_render_model_memo_key_tracks_same_length_text_changes() {
    let (_context, _session_id, mut snapshot) = perf_snapshot_with_messages();
    let base = build_session_render_model_memo_key(&snapshot, 72, &HashSet::new());

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

    let changed = build_session_render_model_memo_key(&snapshot, 72, &HashSet::new());
    assert_ne!(base, changed);
}

#[test]
fn session_render_model_cache_reuses_model_on_identical_inputs() {
    let context = Arc::new(AppContext::new());
    let snapshot = SessionMessagesSnapshot::capture(&context, "session-1");
    let area = Rect::new(0, 0, 80, 20);
    let mut buffer = Buffer::empty(area);

    let (model, _, cache) = resolve_session_render_model(
        area,
        &snapshot,
        &HashSet::new(),
        &SessionMessageOutputCache::default(),
        &SessionRenderModelCache::default(),
        &mut buffer,
    );
    let (reused_model, _, _) = resolve_session_render_model(
        area,
        &snapshot,
        &HashSet::new(),
        &SessionMessageOutputCache::default(),
        &cache,
        &mut buffer,
    );

    assert!(Arc::ptr_eq(&model, &reused_model));
}

#[test]
fn session_render_model_cache_rebuilds_on_same_length_text_change() {
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
