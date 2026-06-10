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
    let snapshot = SessionMessagesSnapshot::capture(&context, "session-1");
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
