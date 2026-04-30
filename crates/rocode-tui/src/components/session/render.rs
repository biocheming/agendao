fn render_session_messages_child(
    area: Rect,
    snapshot: &SessionMessagesSnapshot,
    viewport: &SessionMessageViewportState,
    reasoning: &SessionReasoningState,
    message_cache: &SessionMessageOutputCache,
    render_model_cache: &SessionRenderModelCache,
    buffer: &mut Buffer,
) -> SessionMessagesOutput {
    if area.height == 0 || area.width == 0 {
        let mut next_viewport = viewport.clone();
        next_viewport.last_messages_area = None;
        next_viewport.last_scrollbar_area = None;
        next_viewport.rendered_line_count = 0;
        next_viewport.messages_viewport_height = 0;
        next_viewport.render_model_memo_key = None;
        next_viewport.message_first_lines.clear();
        let mut next_reasoning = reasoning.clone();
        next_reasoning.toggle_hits.clear();
        return SessionMessagesOutput {
            viewport: next_viewport,
            reasoning: next_reasoning,
            message_cache: message_cache.clone(),
            render_model_cache: render_model_cache.clone(),
        };
    }

    let was_near_bottom = viewport
        .rendered_line_count
        .saturating_sub(viewport.messages_viewport_height)
        .saturating_sub(viewport.scroll_offset)
        <= 2;
    let theme = snapshot.theme.clone();
    let show_scrollbar = snapshot.show_scrollbar && area.width > 3;
    let messages_area = if show_scrollbar {
        Rect {
            x: area.x,
            y: area.y,
            width: area.width.saturating_sub(1),
            height: area.height,
        }
    } else {
        area
    };
    let scrollbar_area = show_scrollbar.then_some(Rect {
        x: messages_area.x + messages_area.width,
        y: area.y,
        width: 1,
        height: area.height,
    });
    let (model, next_message_cache, next_render_model_cache) = resolve_session_render_model(
        messages_area,
        snapshot,
        &reasoning.expanded,
        message_cache,
        render_model_cache,
        buffer,
    );

    let mut next_viewport = viewport.clone();
    let model_changed = next_viewport.render_model_memo_key != next_render_model_cache.memo_key;
    next_viewport.last_messages_area = Some(messages_area);
    next_viewport.last_scrollbar_area = scrollbar_area;
    if model_changed {
        next_viewport.render_model_memo_key = next_render_model_cache.memo_key;
        next_viewport.message_first_lines = model.message_first_lines.clone();
    }
    next_viewport.rendered_line_count = model.rendered_line_count;
    next_viewport.messages_viewport_height = usize::from(messages_area.height);

    let mut next_reasoning = reasoning.clone();
    if model_changed {
        next_reasoning.toggle_hits = model.toggle_hits.clone();
        next_reasoning
            .expanded
            .retain(|id| model.visible_reasoning_ids.contains(id));
    }
    let max_scroll = next_viewport
        .rendered_line_count
        .saturating_sub(next_viewport.messages_viewport_height);
    if was_near_bottom || next_viewport.scroll_offset > max_scroll {
        next_viewport.scroll_offset = max_scroll;
    }

    render_session_message_viewport(
        messages_area,
        scrollbar_area,
        &theme,
        model,
        next_viewport.scroll_offset,
        next_viewport.messages_viewport_height,
        buffer,
    );

    SessionMessagesOutput {
        viewport: next_viewport,
        reasoning: next_reasoning,
        message_cache: next_message_cache,
        render_model_cache: next_render_model_cache,
    }
}

fn resolve_session_render_model(
    area: Rect,
    snapshot: &SessionMessagesSnapshot,
    expanded_reasoning: &HashSet<String>,
    message_cache: &SessionMessageOutputCache,
    render_model_cache: &SessionRenderModelCache,
    buffer: &mut Buffer,
) -> (
    Arc<SessionRenderModel>,
    SessionMessageOutputCache,
    SessionRenderModelCache,
) {
    let theme = snapshot.theme.clone();
    let content_width = usize::from(area.width);
    let memo_key = build_session_render_model_memo_key(snapshot, content_width, expanded_reasoning);

    if render_model_cache.memo_key == Some(memo_key) {
        if let Some(model) = render_model_cache.model.as_ref() {
            record_session_render_perf(|counters| counters.render_model_cache_hits += 1);
            return (
                model.clone(),
                message_cache.clone(),
                render_model_cache.clone(),
            );
        }
    }

    let messages = snapshot.messages.as_slice();
    let terminal_messages: Vec<TerminalMessage> =
        messages.iter().map(terminal_message_from_context).collect();
    let resources = SessionRenderResources {
        theme: theme.clone(),
        messages,
        terminal_messages: &terminal_messages,
        revert_info: snapshot.revert_info.clone(),
        content_width,
        show_thinking: snapshot.show_thinking,
        show_timestamps: snapshot.show_timestamps,
        show_tool_details: snapshot.show_tool_details,
        semantic_hl: snapshot.semantic_hl,
        fallback_model: snapshot.fallback_model.clone(),
        user_bg: message_palette::user_message_bg(&theme),
        thinking_bg: message_palette::thinking_message_bg(&theme),
        assistant_border: message_palette::assistant_border_color(&theme),
        thinking_border: message_palette::thinking_border_color(&theme),
        message_gap_lines: 0,
    };
    let (rendered_messages, next_message_cache) =
        render_session_message_items(area, buffer, &resources, expanded_reasoning, message_cache);
    let model = Arc::new(build_session_render_model(
        memo_key,
        &resources,
        rendered_messages,
    ));
    record_session_render_perf(|counters| counters.render_model_rebuilds += 1);
    let next_render_model_cache = SessionRenderModelCache {
        memo_key: Some(memo_key),
        model: Some(model.clone()),
    };
    (model, next_message_cache, next_render_model_cache)
}

fn render_session_message_viewport(
    messages_area: Rect,
    scrollbar_area: Option<Rect>,
    theme: &crate::theme::Theme,
    model: Arc<SessionRenderModel>,
    scroll_offset: usize,
    viewport_height: usize,
    buffer: &mut Buffer,
) {
    if with_current_fiber(|_| ()).is_none() {
        let visible_ranges =
            collect_visible_chunk_ranges(&model.chunks, scroll_offset, viewport_height);
        render_session_message_viewport_widgets(
            buffer,
            messages_area,
            scrollbar_area,
            theme,
            &model,
            &visible_ranges,
            model.rendered_line_count,
            scroll_offset,
            viewport_height,
        );
        return;
    }

    let child = Element::component(SessionMessageViewportComponent {
        theme: theme.clone(),
        model,
        messages_area,
        scrollbar_area,
        scroll_offset,
        viewport_height,
    })
    .with_key("session-message-viewport");
    child.render(messages_area, buffer);
}

fn render_session_message_viewport_widgets(
    buffer: &mut Buffer,
    messages_area: Rect,
    scrollbar_area: Option<Rect>,
    theme: &crate::theme::Theme,
    model: &SessionRenderModel,
    visible_ranges: &[VisibleChunkRange],
    rendered_line_count: usize,
    scroll_offset: usize,
    viewport_height: usize,
) {
    render_visible_message_lines(buffer, messages_area, theme, model, visible_ranges);
    if let Some(scroll_area) = scrollbar_area {
        let mut surface = BufferSurface::new(buffer);
        let mut scrollbar_state = ScrollbarState::new(rendered_line_count)
            .position(scroll_offset)
            .viewport_content_length(viewport_height.max(1));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(Style::default().fg(theme.border_subtle))
            .thumb_symbol("█")
            .thumb_style(Style::default().fg(theme.primary));
        surface.render_stateful_widget(scrollbar, scroll_area, &mut scrollbar_state);
    }
}

fn render_visible_message_lines(
    buffer: &mut Buffer,
    area: Rect,
    theme: &crate::theme::Theme,
    model: &SessionRenderModel,
    visible_ranges: &[VisibleChunkRange],
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let blank_line = " ".repeat(area.width as usize);
    for row in 0..area.height {
        buffer.set_stringn(
            area.x,
            area.y + row,
            blank_line.as_str(),
            area.width as usize,
            Style::default().bg(theme.background),
        );
    }

    let mut y = area.y;
    let max_y = area.y.saturating_add(area.height);
    for range in visible_ranges {
        let Some(chunk) = model.chunks.get(range.chunk_index) else {
            continue;
        };
        for line in &chunk.lines[range.start_in_chunk..range.end_in_chunk] {
            if y >= max_y {
                return;
            }
            buffer.set_line(area.x, y, line, area.width);
            record_session_render_perf(|counters| counters.visible_lines_written += 1);
            y += 1;
        }
    }
}

fn build_session_render_model(
    memo_key: u64,
    resources: &SessionRenderResources<'_>,
    rendered_messages: Vec<SessionMessageRenderItem>,
) -> SessionRenderModel {
    let mut buffer = SessionRenderBuffer::default();
    let mut toggle_hits = Vec::new();
    let mut visible_reasoning_ids = HashSet::new();

    if let Some(revert) = resources.revert_info.as_ref() {
        let card_lines = super::revert_card::render_revert_card(revert, &resources.theme);
        buffer.append_non_message(shared_lines(paint_block_lines(
            card_lines,
            resources.theme.background_panel,
            resources.theme.warning,
            resources.content_width,
        )));
        if !resources.messages.is_empty() {
            buffer.push_spacing(
                resources.message_gap_lines,
                resources.theme.background,
                resources.content_width,
            );
        }
    }

    for item in rendered_messages {
        if item.gap_before {
            buffer.push_spacing(
                resources.message_gap_lines,
                resources.theme.background,
                resources.content_width,
            );
        }
        buffer.record_message_start(&item.message_id);

        let start_line = buffer.line_count();
        buffer.append_message(&item.message_id, item.output.lines.clone());
        toggle_hits.extend(item.output.toggle_line_offsets.into_iter().map(|hit| {
            ThinkingToggleHit {
                line_index: start_line + hit.line_offset,
                reasoning_id: hit.reasoning_id,
            }
        }));
        visible_reasoning_ids.extend(item.output.visible_reasoning_ids);
    }

    SessionRenderModel {
        memo_key,
        chunks: buffer.chunks,
        rendered_line_count: buffer.rendered_line_count,
        message_first_lines: buffer.message_first_lines,
        toggle_hits,
        visible_reasoning_ids,
    }
}

fn render_session_message_items(
    area: Rect,
    buffer: &mut Buffer,
    resources: &SessionRenderResources<'_>,
    expanded_reasoning: &HashSet<String>,
    message_cache: &SessionMessageOutputCache,
) -> (Vec<SessionMessageRenderItem>, SessionMessageOutputCache) {
    let inputs = build_session_message_render_inputs(resources, expanded_reasoning);
    let mut items = Vec::with_capacity(inputs.len());
    let mut next_cache_entries = HashMap::with_capacity(inputs.len());
    for input in inputs {
        if let Some(cached) = message_cache
            .entries
            .get(&input.message_id)
            .filter(|cached| cached.memo_key == input.memo_key)
        {
            record_session_render_perf(|counters| counters.message_cache_hits += 1);
            next_cache_entries.insert(input.message_id.clone(), cached.clone());
            items.push(SessionMessageRenderItem {
                message_id: input.message_id,
                gap_before: input.gap_before,
                output: cached.output.clone(),
            });
            continue;
        }
        record_session_render_perf(|counters| counters.message_cache_misses += 1);

        let output = Arc::new(Mutex::new(None));
        let child = Element::component(SessionMessageItemComponent {
            input: input.clone(),
            output: output.clone(),
        })
        .with_key(format!("session-message-item:{}", input.message_id));
        child.render(area, buffer);
        let next_item = output.lock().take().unwrap_or(SessionMessageRenderItem {
            message_id: input.message_id.clone(),
            gap_before: input.gap_before,
            output: MessageRenderOutput::new(Vec::new()),
        });
        next_cache_entries.insert(
            input.message_id.clone(),
            CachedMessageRenderOutput {
                memo_key: input.memo_key,
                output: next_item.output.clone(),
            },
        );
        items.push(next_item);
    }
    (
        items,
        SessionMessageOutputCache {
            entries: next_cache_entries,
        },
    )
}

fn build_session_message_render_inputs(
    resources: &SessionRenderResources<'_>,
    expanded_reasoning: &HashSet<String>,
) -> Vec<SessionMessageRenderInput> {
    let mut inputs = Vec::new();
    let mut last_visible_role: Option<MessageRole> = None;
    let mut rendered_first_system_prompt = false;
    let last_assistant_idx = resources
        .messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    for (idx, msg) in resources.messages.iter().enumerate() {
        if resources
            .terminal_messages
            .get(idx)
            .is_some_and(is_tool_result_carrier)
        {
            continue;
        }

        let gap_before = should_insert_message_gap(last_visible_role.as_ref(), &msg.role);
        let props = build_message_render_props(
            msg,
            idx,
            last_assistant_idx,
            resources,
            expanded_reasoning,
            &mut rendered_first_system_prompt,
        );
        last_visible_role = Some(message_role_for_render_props(&props));
        inputs.push(SessionMessageRenderInput {
            message_id: msg.id.clone(),
            gap_before,
            memo_key: build_message_output_memo_key(&props),
            props,
        });
    }

    inputs
}

fn should_insert_message_gap(prev_role: Option<&MessageRole>, current_role: &MessageRole) -> bool {
    prev_role.is_some_and(|prev| prev != current_role || matches!(current_role, MessageRole::User))
}

fn build_message_render_props(
    msg: &Message,
    idx: usize,
    last_assistant_idx: Option<usize>,
    resources: &SessionRenderResources<'_>,
    expanded_reasoning: &HashSet<String>,
    rendered_first_system_prompt: &mut bool,
) -> SessionMessageRenderProps {
    match msg.role {
        MessageRole::User => SessionMessageRenderProps::User(build_user_message_render_props(
            msg,
            resources,
            rendered_first_system_prompt,
        )),
        MessageRole::Assistant => {
            SessionMessageRenderProps::Assistant(build_assistant_message_render_props(
                msg,
                idx,
                last_assistant_idx,
                resources,
                expanded_reasoning,
            ))
        }
        MessageRole::System | MessageRole::Tool => {
            SessionMessageRenderProps::Plain(build_plain_message_render_props(msg, resources))
        }
    }
}

fn message_role_for_render_props(props: &SessionMessageRenderProps) -> MessageRole {
    match props {
        SessionMessageRenderProps::User(_) => MessageRole::User,
        SessionMessageRenderProps::Assistant(_) => MessageRole::Assistant,
        SessionMessageRenderProps::Plain(props) => props.msg.role.clone(),
    }
}

fn build_message_render_context(resources: &SessionRenderResources<'_>) -> MessageRenderContext {
    MessageRenderContext {
        theme: resources.theme.clone(),
        content_width: resources.content_width,
        show_timestamps: resources.show_timestamps,
        show_tool_details: resources.show_tool_details,
        semantic_hl: resources.semantic_hl,
        fallback_model: resources.fallback_model.clone(),
        user_bg: resources.user_bg,
        thinking_bg: resources.thinking_bg,
        assistant_border: resources.assistant_border,
        thinking_border: resources.thinking_border,
    }
}

fn build_session_render_model_memo_key(
    snapshot: &SessionMessagesSnapshot,
    content_width: usize,
    expanded_reasoning: &HashSet<String>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", snapshot.theme).hash(&mut hasher);
    format!("{:?}", snapshot.messages).hash(&mut hasher);
    format!("{:?}", snapshot.revert_info).hash(&mut hasher);
    content_width.hash(&mut hasher);
    snapshot.show_timestamps.hash(&mut hasher);
    snapshot.show_thinking.hash(&mut hasher);
    snapshot.show_tool_details.hash(&mut hasher);
    snapshot.semantic_hl.hash(&mut hasher);
    format!("{:?}", snapshot.fallback_model).hash(&mut hasher);
    format!("{:?}", expanded_reasoning).hash(&mut hasher);
    hasher.finish()
}

fn build_session_viewport_content_memo_key(
    render_model_memo_key: u64,
    scroll_offset: usize,
    viewport_height: usize,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    render_model_memo_key.hash(&mut hasher);
    scroll_offset.hash(&mut hasher);
    viewport_height.hash(&mut hasher);
    hasher.finish()
}

fn build_user_message_render_props(
    msg: &Message,
    resources: &SessionRenderResources<'_>,
    rendered_first_system_prompt: &mut bool,
) -> UserMessageRenderProps {
    let show_system_prompt = !*rendered_first_system_prompt
        && msg
            .metadata
            .as_ref()
            .and_then(|metadata| {
                metadata
                    .get("resolved_system_prompt_preview")
                    .or_else(|| metadata.get("resolved_system_prompt"))
            })
            .and_then(|value| value.as_str())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    if show_system_prompt {
        *rendered_first_system_prompt = true;
    }
    UserMessageRenderProps {
        msg: msg.clone(),
        context: build_message_render_context(resources),
        show_system_prompt,
    }
}

fn build_assistant_message_render_props(
    msg: &Message,
    idx: usize,
    last_assistant_idx: Option<usize>,
    resources: &SessionRenderResources<'_>,
    expanded_reasoning: &HashSet<String>,
) -> AssistantMessageRenderProps {
    let tool_results = collect_assistant_tool_results(resources.terminal_messages, idx);
    let running_tool_call =
        resolve_running_tool_call(msg, idx, last_assistant_idx, &tool_results).map(str::to_string);
    let footer_item = build_assistant_footer_item(msg, idx, last_assistant_idx, resources)
        .and_then(|item| match item {
            AssistantMessageItem::Footer(item) => Some(item),
            _ => None,
        });
    AssistantMessageRenderProps {
        msg: msg.clone(),
        context: build_message_render_context(resources),
        terminal_message: resources.terminal_messages.get(idx).cloned(),
        tool_results,
        running_tool_call,
        show_thinking: resources.show_thinking,
        expanded_reasoning: collect_message_expanded_reasoning(msg, expanded_reasoning),
        footer_item,
    }
}

fn build_plain_message_render_props(
    msg: &Message,
    resources: &SessionRenderResources<'_>,
) -> PlainMessageRenderProps {
    PlainMessageRenderProps {
        msg: msg.clone(),
        context: build_message_render_context(resources),
    }
}

fn build_user_message_memo_key(props: &UserMessageRenderProps) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", props.msg).hash(&mut hasher);
    props.show_system_prompt.hash(&mut hasher);
    format!("{:?}", props.context.theme).hash(&mut hasher);
    props.context.content_width.hash(&mut hasher);
    props.context.show_timestamps.hash(&mut hasher);
    format!("{:?}", props.context.user_bg).hash(&mut hasher);
    hasher.finish()
}

fn build_plain_message_memo_key(props: &PlainMessageRenderProps) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", props.msg).hash(&mut hasher);
    format!("{:?}", props.context.theme).hash(&mut hasher);
    props.context.content_width.hash(&mut hasher);
    hasher.finish()
}

fn build_assistant_message_memo_key(props: &AssistantMessageRenderProps) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", props.msg).hash(&mut hasher);
    format!("{:?}", props.context.theme).hash(&mut hasher);
    props.context.content_width.hash(&mut hasher);
    props.context.show_timestamps.hash(&mut hasher);
    props.context.show_tool_details.hash(&mut hasher);
    props.context.semantic_hl.hash(&mut hasher);
    format!("{:?}", props.context.fallback_model).hash(&mut hasher);
    format!("{:?}", props.context.user_bg).hash(&mut hasher);
    format!("{:?}", props.context.thinking_bg).hash(&mut hasher);
    format!("{:?}", props.context.assistant_border).hash(&mut hasher);
    format!("{:?}", props.context.thinking_border).hash(&mut hasher);
    format!("{:?}", props.terminal_message).hash(&mut hasher);
    format!("{:?}", props.tool_results).hash(&mut hasher);
    format!("{:?}", props.running_tool_call).hash(&mut hasher);
    props.show_thinking.hash(&mut hasher);
    format!("{:?}", props.expanded_reasoning).hash(&mut hasher);
    format!("{:?}", props.footer_item).hash(&mut hasher);
    hasher.finish()
}

fn build_message_output_memo_key(props: &SessionMessageRenderProps) -> u64 {
    match props {
        SessionMessageRenderProps::User(props) => build_user_message_memo_key(props),
        SessionMessageRenderProps::Assistant(props) => build_assistant_message_memo_key(props),
        SessionMessageRenderProps::Plain(props) => build_plain_message_memo_key(props),
    }
}

fn build_user_message_output(props: &UserMessageRenderProps) -> MessageRenderOutput {
    let user_lines = super::session_message::render_user_message(
        &props.msg,
        &props.context.theme,
        props.context.show_timestamps,
        props.msg.agent.as_deref(),
        props.show_system_prompt,
    );
    let message_border =
        user_border_color_for_agent(props.msg.agent.as_deref(), &props.context.theme);
    MessageRenderOutput::new(paint_block_lines(
        user_lines,
        props.context.user_bg,
        message_border,
        props.context.content_width,
    ))
}

fn build_assistant_message_items_from_props(
    props: &AssistantMessageRenderProps,
) -> Vec<AssistantMessageItem> {
    let mut items = Vec::new();

    if props.msg.parts.is_empty() {
        items.push(AssistantMessageItem::Text(AssistantTextItem {
            text: props.msg.content.clone(),
        }));
    } else if let Some(terminal_message) = props.terminal_message.as_ref() {
        items.extend(build_assistant_segment_items(
            terminal_message,
            &props.tool_results,
            props.running_tool_call.as_deref(),
            props.show_thinking,
        ));
    }

    if let Some(footer_item) = props.footer_item.clone() {
        items.push(AssistantMessageItem::Footer(footer_item));
    }

    items
}

fn render_assistant_block_outputs(
    area: Rect,
    buffer: &mut Buffer,
    props: &AssistantMessageRenderProps,
    style: AssistantMessageRenderStyle,
) -> Vec<AssistantSegmentRenderOutput> {
    let inputs = build_assistant_block_render_inputs(props, style);
    let mut outputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        let output = Arc::new(Mutex::new(None));
        let child = match &input.item {
            AssistantMessageItem::Spacer => Element::component(AssistantSpacerBlockComponent {
                context: props.context.clone(),
                style,
                memo_key: input.memo_key,
                output: output.clone(),
            })
            .with_key(format!(
                "assistant-message-block-spacer:{}:{}",
                props.msg.id, input.block_key
            )),
            AssistantMessageItem::Text(item) => Element::component(AssistantTextBlockComponent {
                msg: props.msg.clone(),
                context: props.context.clone(),
                style,
                memo_key: input.memo_key,
                item: item.clone(),
                output: output.clone(),
            })
            .with_key(format!(
                "assistant-message-block-text:{}:{}",
                props.msg.id, input.block_key
            )),
            AssistantMessageItem::Thinking(item) => {
                Element::component(AssistantThinkingBlockComponent {
                    msg: props.msg.clone(),
                    context: props.context.clone(),
                    expanded_reasoning: props.expanded_reasoning.clone(),
                    memo_key: input.memo_key,
                    item: item.clone(),
                    output: output.clone(),
                })
                .with_key(format!(
                    "assistant-message-block-thinking:{}:{}",
                    props.msg.id, input.block_key
                ))
            }
            AssistantMessageItem::Tool(item) => Element::component(AssistantToolOutputComponent {
                context: props.context.clone(),
                style,
                memo_key: input.memo_key,
                item: item.clone(),
                output: output.clone(),
            })
            .with_key(format!(
                "assistant-message-block-tool:{}:{}",
                props.msg.id, input.block_key
            )),
            AssistantMessageItem::File(item) => Element::component(AssistantFileBlockComponent {
                context: props.context.clone(),
                style,
                memo_key: input.memo_key,
                item: item.clone(),
                output: output.clone(),
            })
            .with_key(format!(
                "assistant-message-block-file:{}:{}",
                props.msg.id, input.block_key
            )),
            AssistantMessageItem::Image(item) => Element::component(AssistantImageBlockComponent {
                context: props.context.clone(),
                style,
                memo_key: input.memo_key,
                item: item.clone(),
                output: output.clone(),
            })
            .with_key(format!(
                "assistant-message-block-image:{}:{}",
                props.msg.id, input.block_key
            )),
            AssistantMessageItem::Footer(item) => {
                Element::component(AssistantFooterBlockComponent {
                    context: props.context.clone(),
                    style,
                    memo_key: input.memo_key,
                    item: item.clone(),
                    output: output.clone(),
                })
                .with_key(format!(
                    "assistant-message-block-footer:{}:{}",
                    props.msg.id, input.block_key
                ))
            }
        };
        child.render(area, buffer);
        let next_output = output.lock().take();
        if let Some(output) = next_output {
            outputs.push(output);
        } else {
            outputs.push(empty_assistant_segment_output());
        }
    }
    outputs
}

fn build_assistant_block_render_inputs(
    props: &AssistantMessageRenderProps,
    style: AssistantMessageRenderStyle,
) -> Vec<AssistantBlockRenderInput> {
    build_assistant_message_items_from_props(props)
        .into_iter()
        .enumerate()
        .map(|(index, item)| AssistantBlockRenderInput {
            block_key: format!("{index}:{}", assistant_block_kind(&item)),
            memo_key: build_assistant_block_memo_key(
                &props.msg,
                &props.context,
                style,
                &props.expanded_reasoning,
                &item,
            ),
            item,
        })
        .collect()
}

fn assistant_block_kind(item: &AssistantMessageItem) -> &'static str {
    match item {
        AssistantMessageItem::Spacer => "spacer",
        AssistantMessageItem::Text(_) => "text",
        AssistantMessageItem::Thinking(_) => "thinking",
        AssistantMessageItem::Tool(_) => "tool",
        AssistantMessageItem::File(_) => "file",
        AssistantMessageItem::Image(_) => "image",
        AssistantMessageItem::Footer(_) => "footer",
    }
}

fn build_assistant_block_memo_key(
    msg: &Message,
    context: &MessageRenderContext,
    style: AssistantMessageRenderStyle,
    expanded_reasoning: &HashSet<String>,
    item: &AssistantMessageItem,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", msg).hash(&mut hasher);
    format!("{:?}", context.theme).hash(&mut hasher);
    context.content_width.hash(&mut hasher);
    context.show_timestamps.hash(&mut hasher);
    context.show_tool_details.hash(&mut hasher);
    context.semantic_hl.hash(&mut hasher);
    format!("{:?}", context.fallback_model).hash(&mut hasher);
    format!("{:?}", style.marker).hash(&mut hasher);
    format!("{:?}", style.background).hash(&mut hasher);
    format!("{:?}", style.border).hash(&mut hasher);
    format!("{:?}", expanded_reasoning).hash(&mut hasher);
    hash_assistant_message_item(item, &mut hasher);
    hasher.finish()
}

fn hash_assistant_message_item(item: &AssistantMessageItem, hasher: &mut DefaultHasher) {
    match item {
        AssistantMessageItem::Spacer => "spacer".hash(hasher),
        AssistantMessageItem::Text(item) => {
            "text".hash(hasher);
            item.text.hash(hasher);
        }
        AssistantMessageItem::Thinking(item) => {
            "thinking".hash(hasher);
            item.part_index.hash(hasher);
            item.text.hash(hasher);
        }
        AssistantMessageItem::Tool(item) => {
            "tool".hash(hasher);
            item.name.hash(hasher);
            item.arguments.hash(hasher);
            format!("{:?}", item.state).hash(hasher);
            format!("{:?}", item.result).hash(hasher);
        }
        AssistantMessageItem::File(item) => {
            "file".hash(hasher);
            item.path.hash(hasher);
            item.mime.hash(hasher);
        }
        AssistantMessageItem::Image(item) => {
            "image".hash(hasher);
            item.url.hash(hasher);
        }
        AssistantMessageItem::Footer(item) => {
            "footer".hash(hasher);
            format!("{:?}", item.line).hash(hasher);
        }
    }
}

fn empty_assistant_segment_output() -> AssistantSegmentRenderOutput {
    AssistantSegmentRenderOutput {
        lines: Vec::new(),
        toggle_line_offsets: Vec::new(),
        visible_reasoning_ids: HashSet::new(),
    }
}

fn build_assistant_render_resources(
    context: &MessageRenderContext,
) -> SessionRenderResources<'static> {
    SessionRenderResources {
        theme: context.theme.clone(),
        messages: &[],
        terminal_messages: &[],
        revert_info: None,
        content_width: context.content_width,
        show_thinking: true,
        show_timestamps: context.show_timestamps,
        show_tool_details: context.show_tool_details,
        semantic_hl: context.semantic_hl,
        fallback_model: context.fallback_model.clone(),
        user_bg: context.user_bg,
        thinking_bg: context.thinking_bg,
        assistant_border: context.assistant_border,
        thinking_border: context.thinking_border,
        message_gap_lines: 0,
    }
}

fn render_assistant_spacer_block(
    style: AssistantMessageRenderStyle,
    context: &MessageRenderContext,
) -> AssistantSegmentRenderOutput {
    build_assistant_block_output(
        vec![Line::from("")],
        style.background,
        style.border,
        context.content_width,
    )
}

fn render_assistant_text_block(
    msg: &Message,
    item: &AssistantTextItem,
    context: &MessageRenderContext,
    style: AssistantMessageRenderStyle,
) -> AssistantSegmentRenderOutput {
    let resources = build_assistant_render_resources(context);
    build_assistant_text_segment_output(msg, &item.text, style, &resources)
}

fn render_assistant_thinking_output(
    msg: &Message,
    item: &AssistantThinkingItem,
    context: &MessageRenderContext,
    expanded_reasoning: &HashSet<String>,
) -> AssistantSegmentRenderOutput {
    let reasoning_id = format!("{}:{}", msg.id, item.part_index);
    let collapsed = !expanded_reasoning.contains(&reasoning_id);
    let rendered = super::session_text::render_reasoning_part(
        &item.text,
        &context.theme,
        collapsed,
        THINKING_PREVIEW_LINES,
    );
    if rendered.lines.is_empty() {
        return empty_assistant_segment_output();
    }

    let lines = paint_block_lines(
        rendered.lines,
        context.thinking_bg,
        context.thinking_border,
        context.content_width,
    );
    let mut toggle_line_offsets = Vec::new();
    let mut visible_reasoning_ids = HashSet::new();
    if rendered.collapsible {
        let end_line = lines.len().saturating_sub(1);
        visible_reasoning_ids.insert(reasoning_id.clone());
        toggle_line_offsets.push(ThinkingToggleHitOffset {
            line_offset: 0,
            reasoning_id: reasoning_id.clone(),
        });
        if end_line > 0 {
            toggle_line_offsets.push(ThinkingToggleHitOffset {
                line_offset: end_line,
                reasoning_id,
            });
        }
    }

    AssistantSegmentRenderOutput {
        lines,
        toggle_line_offsets,
        visible_reasoning_ids,
    }
}

fn render_assistant_tool_output(
    item: &AssistantToolBlockItem,
    style: AssistantMessageRenderStyle,
    context: &MessageRenderContext,
) -> AssistantSegmentRenderOutput {
    let resources = build_assistant_render_resources(context);
    let tool_lines = super::session_tool::render_tool_call(
        &item.name,
        &item.arguments,
        item.state,
        item.result.as_ref(),
        context.show_tool_details,
        &context.theme,
    );
    build_assistant_shared_items_output(tool_lines, style, &resources)
}

fn render_assistant_file_output(
    item: &AssistantFileItem,
    style: AssistantMessageRenderStyle,
    context: &MessageRenderContext,
) -> AssistantSegmentRenderOutput {
    let resources = build_assistant_render_resources(context);
    build_assistant_shared_items_output(
        render_shared_message_block_items(
            build_file_items(&item.path, &item.mime),
            super::session_text::ASSISTANT_MARKER,
            style.marker,
            &context.theme,
        ),
        style,
        &resources,
    )
}

fn render_assistant_image_output(
    item: &AssistantImageItem,
    style: AssistantMessageRenderStyle,
    context: &MessageRenderContext,
) -> AssistantSegmentRenderOutput {
    let resources = build_assistant_render_resources(context);
    build_assistant_shared_items_output(
        render_shared_message_block_items(
            build_image_items(&item.url),
            super::session_text::ASSISTANT_MARKER,
            style.marker,
            &context.theme,
        ),
        style,
        &resources,
    )
}

fn render_assistant_footer_output(
    item: &AssistantFooterItem,
    style: AssistantMessageRenderStyle,
    context: &MessageRenderContext,
) -> AssistantSegmentRenderOutput {
    let resources = build_assistant_render_resources(context);
    build_assistant_shared_items_output(vec![item.line.clone()], style, &resources)
}

fn build_assistant_segment_items(
    terminal_message: &TerminalMessage,
    tool_results: &HashMap<String, TerminalToolResultInfo>,
    running_tool_call: Option<&str>,
    show_thinking: bool,
) -> Vec<AssistantMessageItem> {
    compose_assistant_segments(
        terminal_message,
        tool_results,
        running_tool_call,
        show_thinking,
    )
    .into_iter()
    .map(build_assistant_message_item)
    .collect()
}

fn resolve_running_tool_call<'a>(
    msg: &'a Message,
    idx: usize,
    last_assistant_idx: Option<usize>,
    tool_results: &HashMap<String, rocode_command::terminal_presentation::TerminalToolResultInfo>,
) -> Option<&'a str> {
    let is_active_assistant =
        last_assistant_idx == Some(idx) && msg.finish.is_none() && msg.error.is_none();
    if !is_active_assistant {
        return None;
    }

    msg.parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::ToolCall { id, .. } if !tool_results.contains_key(id) => Some(id.as_str()),
            _ => None,
        })
        .next()
}

fn build_assistant_text_segment_output(
    msg: &Message,
    text: &str,
    style: AssistantMessageRenderStyle,
    resources: &SessionRenderResources<'_>,
) -> AssistantSegmentRenderOutput {
    let rendered =
        super::session_text::render_message_text_part(msg, text, &resources.theme, style.marker);
    let mut text_lines = rendered.lines;
    if resources.semantic_hl
        && rendered.allow_semantic_highlighting
        && rendered.source_len <= SEMANTIC_HIGHLIGHT_MAX_CHARS
    {
        text_lines = super::semantic_highlight::highlight_lines(text_lines, &resources.theme);
    }
    build_assistant_block_output(
        text_lines,
        style.background,
        style.border,
        resources.content_width,
    )
}

fn build_assistant_message_item(segment: TerminalAssistantSegment) -> AssistantMessageItem {
    match segment {
        TerminalAssistantSegment::Spacer => AssistantMessageItem::Spacer,
        TerminalAssistantSegment::Text { text, .. } => {
            AssistantMessageItem::Text(AssistantTextItem { text })
        }
        TerminalAssistantSegment::Reasoning { part_index, text } => {
            AssistantMessageItem::Thinking(AssistantThinkingItem { part_index, text })
        }
        TerminalAssistantSegment::ToolCall {
            name,
            arguments,
            state,
            result,
            ..
        } => AssistantMessageItem::Tool(AssistantToolBlockItem {
            name,
            arguments,
            state,
            result,
        }),
        TerminalAssistantSegment::File { path, mime, .. } => {
            AssistantMessageItem::File(AssistantFileItem { path, mime })
        }
        TerminalAssistantSegment::Image { url, .. } => {
            AssistantMessageItem::Image(AssistantImageItem { url })
        }
    }
}

fn build_assistant_shared_items_output(
    lines: Vec<Line<'static>>,
    style: AssistantMessageRenderStyle,
    resources: &SessionRenderResources<'_>,
) -> AssistantSegmentRenderOutput {
    build_assistant_block_output(
        lines,
        style.background,
        style.border,
        resources.content_width,
    )
}

fn build_assistant_block_output(
    lines: Vec<Line<'static>>,
    background: Color,
    border: Color,
    content_width: usize,
) -> AssistantSegmentRenderOutput {
    AssistantSegmentRenderOutput {
        lines: paint_block_lines(lines, background, border, content_width),
        toggle_line_offsets: Vec::new(),
        visible_reasoning_ids: HashSet::new(),
    }
}

fn build_assistant_footer_item(
    msg: &Message,
    idx: usize,
    last_assistant_idx: Option<usize>,
    resources: &SessionRenderResources<'_>,
) -> Option<AssistantMessageItem> {
    assistant_footer(
        resources.messages,
        idx,
        last_assistant_idx,
        msg,
        resources.fallback_model.as_deref(),
        &resources.theme,
    )
    .map(|line| AssistantMessageItem::Footer(AssistantFooterItem { line }))
}

fn build_plain_message_output(props: &PlainMessageRenderProps) -> MessageRenderOutput {
    let plain_lines: Vec<Line<'static>> = props
        .msg
        .content
        .lines()
        .map(|line_text| {
            Line::from(Span::styled(
                line_text.to_string(),
                Style::default().fg(props.context.theme.text_muted),
            ))
        })
        .collect();
    let painted: Vec<Line<'static>> = plain_lines
        .into_iter()
        .map(|line| {
            paint_block_line(
                line,
                props.context.theme.background,
                props.context.theme.border,
                props.context.content_width,
            )
        })
        .collect();
    MessageRenderOutput::new(painted)
}

fn collect_message_expanded_reasoning(
    msg: &Message,
    expanded_reasoning: &HashSet<String>,
) -> HashSet<String> {
    msg.parts
        .iter()
        .enumerate()
        .filter_map(|(part_index, part)| match part {
            MessagePart::Reasoning { .. } => {
                let reasoning_id = format!("{}:{}", msg.id, part_index);
                expanded_reasoning
                    .contains(&reasoning_id)
                    .then_some(reasoning_id)
            }
            _ => None,
        })
        .collect()
}

fn paint_block_lines(
    lines: Vec<Line<'static>>,
    background: Color,
    border_color: Color,
    width: usize,
) -> Vec<Line<'static>> {
    let painted: Vec<Line<'static>> = lines
        .into_iter()
        .flat_map(|line| wrap_block_line(line, width))
        .map(|line| paint_block_line(line, background, border_color, width))
        .collect();

    if painted.is_empty() {
        return painted;
    }

    let gutter = painted
        .first()
        .and_then(|line| line.spans.first())
        .map(|span| span.content.to_string())
        .filter(|value| is_gutter_span(value.as_str()));

    let padding_line = if let Some(gutter) = gutter {
        paint_block_line(
            Line::from(vec![Span::raw(gutter)]),
            background,
            border_color,
            width,
        )
    } else {
        paint_block_line(Line::from(""), background, border_color, width)
    };

    let mut padded = Vec::with_capacity(painted.len() + 2);
    padded.push(padding_line.clone());
    padded.extend(painted);
    padded.push(padding_line);
    padded
}

fn paint_block_line(
    line: Line<'static>,
    background: Color,
    border_color: Color,
    width: usize,
) -> Line<'static> {
    let mut styled = Vec::with_capacity(line.spans.len() + 1);
    let mut rendered_width = 0usize;

    for (idx, span) in line.spans.into_iter().enumerate() {
        rendered_width += UnicodeWidthStr::width(span.content.as_ref());
        let style = if idx == 0 && is_gutter_span(span.content.as_ref()) {
            span.style.fg(border_color).bg(background)
        } else {
            span.style.bg(background)
        };
        styled.push(Span::styled(span.content, style));
    }

    if rendered_width < width {
        styled.push(Span::styled(
            " ".repeat(width - rendered_width),
            Style::default().bg(background),
        ));
    }

    Line::from(styled)
}

fn wrap_block_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.spans.is_empty() {
        return vec![line];
    }

    let mut iter = line.spans.into_iter();
    let Some(gutter) = iter.next() else {
        return vec![Line::from("")];
    };
    if !is_gutter_span(gutter.content.as_ref()) {
        let mut all_spans = vec![gutter];
        all_spans.extend(iter);
        let wrapped = wrap_spans(all_spans, width);
        return wrapped.into_iter().map(Line::from).collect();
    }

    let body_spans: Vec<Span<'static>> = iter.collect();
    let gutter_width = UnicodeWidthStr::width(gutter.content.as_ref());
    if gutter_width >= width {
        return vec![Line::from(vec![gutter])];
    }

    let body_width = width
        .saturating_sub(gutter_width)
        .saturating_sub(MESSAGE_BLOCK_RIGHT_PADDING);
    if body_width == 0 {
        return vec![Line::from(vec![gutter])];
    }
    let wrapped_body = wrap_spans(body_spans, body_width);
    wrapped_body
        .into_iter()
        .map(|body| {
            let mut spans = Vec::with_capacity(body.len() + 1);
            spans.push(gutter.clone());
            spans.extend(body);
            Line::from(spans)
        })
        .collect()
}

fn is_gutter_span(content: &str) -> bool {
    let mut has_border = false;
    for ch in content.chars() {
        match ch {
            '│' | '┃' => has_border = true,
            ' ' => {}
            _ => return false,
        }
    }
    has_border
}

fn map_scrollbar_row_to_offset(area: Option<Rect>, row: u16, max_scroll: usize) -> usize {
    let Some(area) = area else {
        return 0;
    };
    if max_scroll == 0 || area.height <= 1 {
        return 0;
    }

    let top = area.y;
    let bottom = area.y.saturating_add(area.height.saturating_sub(1));
    let clamped_row = row.clamp(top, bottom);
    let relative_row = usize::from(clamped_row.saturating_sub(top));
    let track = usize::from(area.height.saturating_sub(1)).max(1);

    ((relative_row * max_scroll) + track / 2) / track
}

fn point_in_optional_rect(area: Option<Rect>, col: u16, row: u16) -> bool {
    let Some(area) = area else {
        return false;
    };
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    col >= area.x && col < max_x && row >= area.y && row < max_y
}

fn tint_sidebar_overlay(background: Color, accent: Color) -> Color {
    match (background, accent) {
        (Color::Rgb(br, bg, bb), Color::Rgb(ar, ag, ab)) => {
            let blend = |b: u8, a: u8| -> u8 { ((u16::from(b) * 4 + u16::from(a)) / 5) as u8 };
            Color::Rgb(blend(br, ar), blend(bg, ag), blend(bb, ab))
        }
        _ => background,
    }
}

fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans];
    }

    let mut out: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut current_width = 0usize;

    for span in spans {
        let style = span.style;
        for ch in span.content.chars() {
            if ch == '\n' {
                out.push(Vec::new());
                current_width = 0;
                continue;
            }

            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > width && !out.last().is_some_and(|line| line.is_empty()) {
                out.push(Vec::new());
                current_width = 0;
            }

            push_merged_span(out.last_mut().expect("line exists"), ch, style);
            current_width += ch_width;
        }
    }

    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

fn push_merged_span(line: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = line.last_mut() {
        if last.style == style {
            last.content.to_mut().push(ch);
            return;
        }
    }

    line.push(Span::styled(ch.to_string(), style));
}

fn terminal_message_from_context(message: &Message) -> TerminalMessage {
    let role = match message.role {
        MessageRole::User => TerminalMessageRole::User,
        MessageRole::Assistant => TerminalMessageRole::Assistant,
        MessageRole::System => TerminalMessageRole::System,
        MessageRole::Tool => TerminalMessageRole::Tool,
    };

    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => TerminalMessagePart::Text { text: text.clone() },
            MessagePart::Reasoning { text } => {
                TerminalMessagePart::Reasoning { text: text.clone() }
            }
            MessagePart::File { path, mime } => TerminalMessagePart::File {
                path: path.clone(),
                mime: mime.clone(),
            },
            MessagePart::Image { url } => TerminalMessagePart::Image { url: url.clone() },
            MessagePart::ToolCall {
                id,
                name,
                arguments,
            } => TerminalMessagePart::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            },
            MessagePart::ToolResult {
                id,
                result,
                is_error,
                title,
                metadata,
            } => TerminalMessagePart::ToolResult {
                id: id.clone(),
                result: result.clone(),
                is_error: *is_error,
                title: title.clone(),
                metadata: metadata.clone(),
            },
        })
        .collect();

    TerminalMessage {
        id: message.id.clone(),
        role,
        parts,
    }
}

fn assistant_footer(
    messages: &[Message],
    idx: usize,
    last_assistant_idx: Option<usize>,
    message: &Message,
    fallback_model: Option<&str>,
    theme: &crate::theme::Theme,
) -> Option<Line<'static>> {
    if !matches!(message.role, MessageRole::Assistant) {
        return None;
    }

    let is_last_assistant = last_assistant_idx == Some(idx);
    let is_interrupted = is_assistant_interrupted(message);
    let is_final = is_assistant_final(message);

    if !is_last_assistant && !is_final && !is_interrupted {
        return None;
    }

    let mode = message
        .mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| assistant_metadata_text(message, "scheduler_profile"))
        .or_else(|| {
            message
                .agent
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or("assistant");
    let mut spans = vec![
        Span::styled(
            "◆ ",
            Style::default().fg(if is_interrupted {
                theme.text_muted
            } else {
                assistant_marker_color(message.agent.as_deref(), theme)
            }),
        ),
        Span::styled(mode.to_string(), Style::default().fg(theme.text)),
    ];

    if let Some(model) = message
        .model
        .as_deref()
        .or(fallback_model)
        .filter(|value| !value.trim().is_empty())
    {
        spans.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
        spans.push(Span::styled(
            model.to_string(),
            Style::default().fg(theme.text_muted),
        ));
    }

    if let Some(duration) = assistant_duration(messages, idx, message, is_final) {
        spans.push(Span::styled(" · ", Style::default().fg(theme.text_muted)));
        spans.push(Span::styled(
            duration,
            Style::default().fg(theme.text_muted),
        ));
    }

    if is_interrupted {
        spans.push(Span::styled(
            " · interrupted",
            Style::default().fg(theme.text_muted),
        ));
    }

    Some(Line::from(spans))
}

fn assistant_metadata_text<'a>(message: &'a Message, key: &str) -> Option<&'a str> {
    message
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(key))
        .and_then(|value| value.as_str())
}

fn assistant_duration(
    messages: &[Message],
    idx: usize,
    message: &Message,
    is_final: bool,
) -> Option<String> {
    if !is_final {
        return None;
    }
    let user_start = messages[..idx]
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::User))
        .map(|m| m.created_at.timestamp_millis())?;
    let end = message.completed_at?.timestamp_millis();
    if end <= user_start {
        return None;
    }

    let elapsed = (end - user_start) as u64;
    if elapsed < 1_000 {
        Some(format!("{elapsed}ms"))
    } else {
        Some(format!("{:.1}s", elapsed as f64 / 1_000.0))
    }
}

fn is_assistant_final(message: &Message) -> bool {
    matches!(
        message.finish.as_deref(),
        Some(reason) if reason != "tool-calls" && reason != "unknown"
    )
}

fn is_assistant_interrupted(message: &Message) -> bool {
    if message.finish.as_deref() == Some("abort") {
        return true;
    }
    message
        .error
        .as_deref()
        .map(|err| {
            let lower = err.to_ascii_lowercase();
            lower.contains("messageabortederror")
                || lower.contains("abortederror")
                || lower.contains("abort")
        })
        .unwrap_or(false)
}

fn assistant_marker_color(agent: Option<&str>, theme: &crate::theme::Theme) -> Color {
    let Some(agent) = agent else {
        return theme.primary;
    };
    let mut hasher = DefaultHasher::new();
    agent.hash(&mut hasher);
    theme.agent_color(hasher.finish() as usize)
}

fn user_border_color_for_agent(agent: Option<&str>, theme: &crate::theme::Theme) -> Color {
    let Some(agent) = agent else {
        return theme.primary;
    };
    let mut hasher = DefaultHasher::new();
    agent.hash(&mut hasher);
    theme.agent_color(hasher.finish() as usize)
}

fn format_compact_number(value: u64) -> String {
    if value >= 1_000_000 {
        let compact = value as f64 / 1_000_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}M")
        } else {
            format!("{compact:.1}M")
        };
    }
    if value >= 1_000 {
        let compact = value as f64 / 1_000.0;
        return if compact.fract() == 0.0 {
            format!("{compact:.0}K")
        } else {
            format!("{compact:.1}K")
        };
    }
    value.to_string()
}

fn context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    rocode_types::context_usage_percent(used, limit)
}

fn context_usage_bar(percent: Option<u64>, width: usize) -> String {
    let safe_percent = percent.unwrap_or(0).min(100) as usize;
    let mut filled = ((safe_percent * width) + 50) / 100;
    if safe_percent > 0 && filled == 0 {
        filled = 1;
    }
    format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(width.saturating_sub(filled))
    )
}

fn context_pressure_suffix(percent: Option<u64>) -> Option<&'static str> {
    rocode_types::context_pressure_label(percent)
}

#[cfg_attr(not(test), allow(dead_code))]
fn format_context_usage_label(used: u64, limit: Option<u64>) -> String {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return format_compact_number(used);
    };

    let percent = context_usage_percent(used, limit);
    let mut label = format!(
        "{}/{} {}%",
        format_compact_number(used),
        format_compact_number(limit),
        percent.unwrap_or(0)
    );
    if let Some(suffix) = context_pressure_suffix(percent) {
        label.push(' ');
        label.push_str(suffix);
    }
    label
}

fn format_context_usage_meter(used: u64, limit: Option<u64>) -> Option<(String, Option<u64>)> {
    let limit = limit.filter(|limit| *limit > 0)?;
    let percent = context_usage_percent(used, limit);
    let mut label = format!(
        "{}/{} {} {}%",
        format_compact_number(used),
        format_compact_number(limit),
        context_usage_bar(percent, 8),
        percent.unwrap_or(0)
    );
    if let Some(suffix) = context_pressure_suffix(percent) {
        label.push(' ');
        label.push_str(suffix);
    }
    Some((label, percent))
}

fn total_session_tokens(usage: &rocode_session::SessionUsage) -> u64 {
    usage.input_tokens + usage.output_tokens + usage.reasoning_tokens
}
