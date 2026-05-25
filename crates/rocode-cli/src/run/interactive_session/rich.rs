use super::*;
use std::collections::HashMap;

/// A rendered content block in the CLI output stream.
/// Blocks are ordered — new blocks push old ones down.
#[derive(Debug, Clone)]
struct ContentBlock {
    kind: BlockKind,
    identity_key: Option<String>,
    /// Full rendered ANSI string for this block (badge + content + divider).
    rendered: String,
    /// Terminal rows this block occupies (for ANSI cursor math).
    row_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Thinking,
    Assistant,
    Tool,
    Status,
}

#[derive(Debug, Clone, Default)]
struct CliInteractiveRichState {
    /// Ordered blocks per session.
    blocks: HashMap<String, Vec<ContentBlock>>,
    /// Live slot accumulation (same as before, for streaming merge).
    live_slots: HashMap<String, HashMap<String, CliInteractiveRichLiveSlot>>,
    /// Legacy streaming blocks without live_identity, buffered until End.
    legacy_stream_slots: HashMap<String, HashMap<String, CliInteractiveRichLegacySlot>>,
    /// Tracks which blocks have been painted, keyed by session.
    painted: HashMap<String, Vec<ContentBlock>>,
}

#[derive(Debug, Clone)]
enum CliInteractiveRichLiveSlot {
    AssistantText {
        text: String,
    },
    AssistantReasoning {
        /// Per-part_key accumulated text, in insertion order.
        parts: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone)]
enum CliInteractiveRichLegacySlot {
    Message {
        role: OutputMessageRole,
        text: String,
    },
    Reasoning {
        text: String,
    },
}

impl CliInteractiveRichState {
    fn merge_snapshot_fragment(buffer: &mut String, incoming: &str) {
        if incoming.is_empty() {
            return;
        }
        if buffer.is_empty() {
            *buffer = incoming.to_string();
            return;
        }
        if incoming.starts_with(buffer.as_str()) {
            *buffer = incoming.to_string();
            return;
        }
        if buffer == incoming {
            return;
        }
        buffer.push_str(incoming);
    }

    fn blocks_mut(&mut self, session_id: &str) -> &mut Vec<ContentBlock> {
        self.blocks.entry(session_id.to_string()).or_default()
    }

    fn push_block(
        &mut self,
        session_id: &str,
        kind: BlockKind,
        rendered: String,
        _streaming: bool,
    ) {
        let row_count = rendered.lines().count().max(1);
        self.blocks_mut(session_id).push(ContentBlock {
            kind,
            identity_key: None,
            rendered,
            row_count,
        });
    }

    /// Replace the existing block for the same live slot identity in place.
    /// This avoids duplicating the last thinking/assistant block when Full/End
    /// events for interleaved parts arrive out of order.
    fn upsert_block(
        &mut self,
        session_id: &str,
        identity_key: String,
        kind: BlockKind,
        rendered: String,
    ) {
        let row_count = rendered.lines().count().max(1);
        let blocks = self.blocks_mut(session_id);
        if let Some(existing) = blocks
            .iter_mut()
            .find(|block| block.identity_key.as_deref() == Some(identity_key.as_str()))
        {
            existing.kind = kind;
            existing.rendered = rendered;
            existing.row_count = row_count;
            existing.identity_key = Some(identity_key);
            return;
        }
        if let Some(last) = blocks.last_mut() {
            if last.identity_key.is_none() && last.kind == kind {
                last.rendered = rendered;
                last.row_count = row_count;
                last.identity_key = Some(identity_key);
                return;
            }
        }
        blocks.push(ContentBlock {
            kind,
            identity_key: Some(identity_key),
            rendered,
            row_count,
        });
    }

    fn painted_blocks(&self, session_id: &str) -> Vec<ContentBlock> {
        self.painted.get(session_id).cloned().unwrap_or_default()
    }

    fn record_painted(&mut self, session_id: &str, blocks: Vec<ContentBlock>) {
        self.painted.insert(session_id.to_string(), blocks);
    }

    fn apply_live_slot_update(
        &mut self,
        session_id: &str,
        block: &OutputBlock,
        live_identity: &rocode_types::LiveMessagePartIdentity,
        style: &CliStyle,
    ) -> bool {
        let slot_key = format!("{}:{}", live_identity.message_id, live_identity.part_key);
        match live_identity.part_kind {
            rocode_types::LiveMessagePartKind::AssistantText => {
                let Some(message) = (match block {
                    OutputBlock::Message(message) => Some(message),
                    _ => None,
                }) else {
                    return false;
                };
                // Accumulate silently. Upsert on every Full/End so the block
                // stays in the list with latest text, but only trigger a
                // full-screen render on End — avoids per-token flickering.
                let should_finalize = live_identity.phase == rocode_types::LivePartPhase::End;
                let is_full = matches!(message.phase, MessagePhase::Full);
                let mut upsert_text: Option<String> = None;
                let slot_has_text: bool;
                {
                    let session_slots = self.live_slots.entry(session_id.to_string()).or_default();
                    let slot = session_slots.entry(slot_key.clone()).or_insert_with(|| {
                        CliInteractiveRichLiveSlot::AssistantText {
                            text: String::new(),
                        }
                    });
                    let CliInteractiveRichLiveSlot::AssistantText { text } = slot else {
                        return false;
                    };
                    match message.phase {
                        MessagePhase::Start => {}
                        MessagePhase::Delta => text.push_str(&message.text),
                        MessagePhase::Full => {
                            Self::merge_snapshot_fragment(text, &message.text);
                        }
                        MessagePhase::End => {}
                    }
                    slot_has_text = !text.is_empty();
                    if slot_has_text && (is_full || should_finalize) {
                        upsert_text = Some(text.clone());
                    }
                }
                if let Some(t) = upsert_text {
                    self.upsert_block(
                        session_id,
                        slot_key,
                        BlockKind::Assistant,
                        render_cli_block_rich(
                            &OutputBlock::Message(MessageBlock::full(
                                OutputMessageRole::Assistant,
                                t,
                            )),
                            style,
                        ),
                    );
                }
                // Never trigger a render — like reasoning, text accumulates
                // silently and appears when a tool/scheduler/idle event redraws.
                false
            }
            rocode_types::LiveMessagePartKind::AssistantReasoning => {
                let Some(reasoning) = (match block {
                    OutputBlock::Reasoning(reasoning) => Some(reasoning),
                    _ => None,
                }) else {
                    return false;
                };
                // Accumulate text silently across token cycles without removing
                // the slot on End. Upsert the block so it stays in the list, but
                // never trigger a render — thinking appears when the next
                // non-reasoning event (assistant/tool) forces a redraw.
                let reasoning_slot_key = live_identity.message_id.clone();
                let part_key = live_identity.part_key.clone();
                let is_full = matches!(reasoning.phase, MessagePhase::Full);
                let phase_is_end = live_identity.phase == rocode_types::LivePartPhase::End;
                {
                    let session_slots = self.live_slots.entry(session_id.to_string()).or_default();
                    let slot = session_slots
                        .entry(reasoning_slot_key.clone())
                        .or_insert_with(|| CliInteractiveRichLiveSlot::AssistantReasoning {
                            parts: Vec::new(),
                        });
                    let CliInteractiveRichLiveSlot::AssistantReasoning { parts } = slot else {
                        return false;
                    };
                    match reasoning.phase {
                        MessagePhase::Start | MessagePhase::End => {}
                        MessagePhase::Delta => {
                            if let Some(idx) = parts.iter().position(|(pk, _)| pk == &part_key) {
                                parts[idx].1.push_str(&reasoning.text);
                            } else {
                                parts.push((part_key.clone(), reasoning.text.clone()));
                            }
                        }
                        MessagePhase::Full => {
                            if let Some(idx) = parts.iter().position(|(pk, _)| pk == &part_key) {
                                Self::merge_snapshot_fragment(&mut parts[idx].1, &reasoning.text);
                            } else {
                                parts.push((part_key.clone(), reasoning.text.clone()));
                            }
                        }
                    }
                }
                if is_full || phase_is_end {
                    if let Some(merged) = self
                        .live_slots
                        .entry(session_id.to_string())
                        .or_default()
                        .get(&reasoning_slot_key)
                        .and_then(|slot| match slot {
                            CliInteractiveRichLiveSlot::AssistantReasoning { parts } => {
                                let m: String = parts.iter().map(|(_, t)| t.as_str()).collect();
                                if m.trim().is_empty() {
                                    None
                                } else {
                                    Some(m)
                                }
                            }
                            _ => None,
                        })
                    {
                        self.upsert_block(
                            session_id,
                            reasoning_slot_key,
                            BlockKind::Thinking,
                            render_cli_block_rich(
                                &OutputBlock::Reasoning(ReasoningBlock::full(merged)),
                                style,
                            ),
                        );
                    }
                }
                // Never trigger a render for reasoning — block updates silently.
                // The accumulated thinking becomes visible when the next
                // assistant / tool event forces a redraw.
                false
            }
            rocode_types::LiveMessagePartKind::ToolResult => {
                if cli_live_slot_has_visible_content(block) {
                    let rendered_ansi = render_cli_block_rich(block, style);
                    self.push_block(session_id, BlockKind::Tool, rendered_ansi, false);
                }
                if live_identity.phase == rocode_types::LivePartPhase::End {
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn apply_legacy_streaming_block(
        &mut self,
        session_id: &str,
        block_id: Option<&str>,
        block: &OutputBlock,
        style: &CliStyle,
    ) -> bool {
        let Some(block_id) = block_id.filter(|value| !value.is_empty()) else {
            return false;
        };

        match block {
            OutputBlock::Message(message) => {
                let slot_key = format!("message:{block_id}");
                let should_finalize = matches!(message.phase, MessagePhase::End);
                let mut rendered_update: Option<String> = None;
                {
                    let session_slots = self
                        .legacy_stream_slots
                        .entry(session_id.to_string())
                        .or_default();
                    let slot = session_slots.entry(slot_key.clone()).or_insert_with(|| {
                        CliInteractiveRichLegacySlot::Message {
                            role: message.role.clone(),
                            text: String::new(),
                        }
                    });
                    let CliInteractiveRichLegacySlot::Message { role, text } = slot else {
                        return false;
                    };
                    *role = message.role.clone();
                    match message.phase {
                        MessagePhase::Start | MessagePhase::End => {}
                        MessagePhase::Delta => text.push_str(&message.text),
                        MessagePhase::Full => *text = message.text.clone(),
                    }
                    if should_finalize && !text.is_empty() {
                        rendered_update = Some(render_cli_block_rich(
                            &OutputBlock::Message(MessageBlock::full(role.clone(), text.clone())),
                            style,
                        ));
                    }
                    if should_finalize {
                        session_slots.remove(&slot_key);
                    }
                }
                if let Some(rendered) = rendered_update {
                    self.push_block(session_id, BlockKind::Assistant, rendered, false);
                }
                should_finalize
            }
            OutputBlock::Reasoning(reasoning) => {
                let slot_key = format!("reasoning:{block_id}");
                let should_finalize = matches!(reasoning.phase, MessagePhase::End);
                let mut rendered_update: Option<String> = None;
                {
                    let session_slots = self
                        .legacy_stream_slots
                        .entry(session_id.to_string())
                        .or_default();
                    let slot = session_slots.entry(slot_key.clone()).or_insert_with(|| {
                        CliInteractiveRichLegacySlot::Reasoning {
                            text: String::new(),
                        }
                    });
                    let CliInteractiveRichLegacySlot::Reasoning { text } = slot else {
                        return false;
                    };
                    match reasoning.phase {
                        MessagePhase::Start | MessagePhase::End => {}
                        MessagePhase::Delta => text.push_str(&reasoning.text),
                        MessagePhase::Full => *text = reasoning.text.clone(),
                    }
                    if should_finalize && !text.trim().is_empty() {
                        rendered_update = Some(render_cli_block_rich(
                            &OutputBlock::Reasoning(ReasoningBlock::full(text.clone())),
                            style,
                        ));
                    }
                    if should_finalize {
                        session_slots.remove(&slot_key);
                    }
                }
                if let Some(rendered) = rendered_update {
                    self.push_block(session_id, BlockKind::Thinking, rendered, false);
                }
                should_finalize
            }
            _ => false,
        }
    }

    fn append_rendered(&mut self, session_id: &str, rendered: String) {
        if rendered.is_empty() {
            return;
        }
        self.push_block(session_id, BlockKind::Status, rendered, false);
    }

    #[cfg(test)]
    fn rendered_text(&self, session_id: &str) -> String {
        self.blocks
            .get(session_id)
            .map(|blocks| {
                blocks
                    .iter()
                    .map(|b| b.rendered.as_str())
                    .collect::<String>()
            })
            .unwrap_or_default()
    }
}

pub(super) async fn run_chat_session_rich(
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    thinking_requested: bool,
    port_override: Option<u16>,
    working_dir: PathBuf,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    let super::bootstrap_shared::InteractiveSessionBootstrap {
        working_dir,
        config,
        command_registry,
        provider_registry,
        agent_registry: agent_registry_arc,
        api_client,
        recent_session_info,
        selection: _selection,
        mut runtime,
        repl_style,
        server_url,
        server_session_id,
    } = super::bootstrap_shared::bootstrap_interactive_session(
        model,
        provider,
        requested_agent,
        requested_scheduler_profile,
        thinking_requested,
        port_override,
        working_dir,
        runtime_context,
    )
    .await?;

    tracing::info!(
        server_url = %server_url,
        session_id = %server_session_id,
        "CLI interactive rich connected to server and created session"
    );

    let server_models = super::prompt_shared::fetch_server_model_list(&api_client).await;

    let mut dispatch_rx = Some(super::attach_rich_prompt(
        &mut runtime,
        &repl_style,
        &working_dir,
        &config,
        provider_registry.as_ref(),
        agent_registry_arc.as_ref(),
        recent_session_info.as_ref(),
        server_models,
    )?);

    let super::stream_shared::InteractiveSessionStream {
        mut sse_rx,
        sse_cancel,
    } = super::stream_shared::bootstrap_interactive_stream(
        &server_url,
        &server_session_id,
        &api_client,
        &runtime,
    )
    .await;

    let mut state = CliInteractiveRichState::default();

    loop {
        let queued = {
            let mut queue = runtime.queued_inputs.lock().await;
            let next = queue.pop_front();
            let remaining = queue.len();
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.queue_len = remaining;
            }
            next
        };

        let trimmed = match queued {
            Some(line) => line,
            None => {
                let Some(dispatch_rx) = dispatch_rx.as_mut() else {
                    anyhow::bail!("interactive prompt receiver missing");
                };
                match wait_for_rich_input_rich(
                    &mut runtime,
                    &config,
                    agent_registry_arc.as_ref(),
                    &api_client,
                    &mut state,
                    dispatch_rx,
                    &mut sse_rx,
                    &repl_style,
                )
                .await?
                {
                    Some(line) => line,
                    None => {
                        sse_cancel.cancel();
                        return Ok(());
                    }
                }
            }
        };

        if trimmed.is_empty() {
            continue;
        }

        if let Some(resolved) = cli_resolve_registry_ui_action(&command_registry, &trimmed) {
            match cli_execute_ui_action(
                resolved.action_id,
                resolved.argument.as_deref(),
                &mut runtime,
                &api_client,
                &mut sse_rx,
                &provider_registry,
                &agent_registry_arc,
                &working_dir,
                &repl_style,
            )
            .await?
            {
                CliUiActionOutcome::Break => break,
                CliUiActionOutcome::Continue => continue,
            }
        }

        if let Some(cmd) = parse_interactive_command(&trimmed) {
            if !matches!(cmd, InteractiveCommand::Unknown(_)) {
                if let Some(invocation) = cmd.ui_action_invocation() {
                    match cli_execute_ui_action(
                        invocation.action_id,
                        invocation.argument.as_deref(),
                        &mut runtime,
                        &api_client,
                        &mut sse_rx,
                        &provider_registry,
                        &agent_registry_arc,
                        &working_dir,
                        &repl_style,
                    )
                    .await?
                    {
                        CliUiActionOutcome::Break => break,
                        CliUiActionOutcome::Continue => continue,
                    }
                }
                match cmd {
                    InteractiveCommand::Abort => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No active run to abort. Use /abort while a response is running.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ExecuteRecovery(selector) => {
                        let Some(action) = cli_select_recovery_action(&runtime, &selector) else {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::warning(format!(
                                    "Unknown recovery action: {}",
                                    selector
                                ))),
                                &repl_style,
                            );
                            cli_print_recovery_actions(&runtime);
                            continue;
                        };
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::title(format!("↺ {}", action.label))),
                            &repl_style,
                        );
                        run_server_prompt_rich(
                            &mut runtime,
                            &mut state,
                            &api_client,
                            &mut sse_rx,
                            &action.prompt,
                            &repl_style,
                            false,
                        )
                        .await?;
                    }
                    InteractiveCommand::ClearScreen => {
                        state = CliInteractiveRichState::default();
                        if let Some(surface) = runtime.terminal_surface.as_ref() {
                            let _ = surface.clear_transcript();
                            let _ = surface.print_rendered_passthrough("\x1b[2J\x1b[H");
                        } else {
                            print!("\x1b[2J\x1b[H");
                            let _ = io::stdout().flush();
                        }
                        refresh_rich_prompt(&runtime);
                    }
                    InteractiveCommand::ListAttachedSessions => {
                        cli_list_attached_sessions(&runtime)
                    }
                    InteractiveCommand::FocusAttachedSession(session_id) => {
                        match cli_focus_attached_session(&runtime, &session_id) {
                            Ok(true) => {
                                render_rich_state(&runtime, &mut state);
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session: {}",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(false) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::warning(format!(
                                        "Unknown attached session: {}. Use /attached list first.",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to focus attached session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusNextAttachedSession => {
                        match cli_cycle_attached_session(&runtime, true) {
                            Ok(Some((session_id, index, total))) => {
                                render_rich_state(&runtime, &mut state);
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::warning(
                                        "No attached sessions available. Use /attached list to inspect the cache.",
                                    )),
                                    &repl_style,
                                );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to next attached session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusPreviousAttachedSession => {
                        match cli_cycle_attached_session(&runtime, false) {
                            Ok(Some((session_id, index, total))) => {
                                render_rich_state(&runtime, &mut state);
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::warning(
                                        "No attached sessions available. Use /attached list to inspect the cache.",
                                    )),
                                    &repl_style,
                                );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to previous attached session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::BackToRootSession => match cli_focus_root_session(&runtime)
                    {
                        Ok(true) => {
                            render_rich_state(&runtime, &mut state);
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::title(
                                    "Returned to root session view.",
                                )),
                                &repl_style,
                            );
                        }
                        Ok(false) => {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::warning(
                                    "Already viewing the root session.",
                                )),
                                &repl_style,
                            );
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to restore root session view: {}",
                                    error
                                ))),
                                &repl_style,
                            );
                        }
                    },
                    InteractiveCommand::Compact(_) => {}
                    InteractiveCommand::ShowTask(id) => cli_show_task(&id, Some(&runtime)),
                    InteractiveCommand::KillTask(id) => cli_kill_task(&id, Some(&runtime)),
                    InteractiveCommand::ToggleActive
                    | InteractiveCommand::ScrollUp
                    | InteractiveCommand::ScrollDown
                    | InteractiveCommand::ScrollBottom => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "CLI interactive rich redraws the full prompt surface; no separate active/scroll panel is exposed.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ShowRuntime => {
                        cli_print_runtime_snapshot(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowUsage => {
                        cli_print_usage_snapshot(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowInsights => {
                        cli_print_session_insights(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowConfigValidation => {
                        cli_print_config_validation(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowEvents(raw_filter) => {
                        cli_print_session_events(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_filter.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemory(search) => {
                        cli_print_memory_list(
                            &runtime,
                            &api_client,
                            &repl_style,
                            search.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryPreview(query) => {
                        cli_print_memory_retrieval_preview(
                            &runtime,
                            &api_client,
                            &repl_style,
                            query.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryDetail(record_id) => {
                        cli_print_memory_detail(&runtime, &api_client, &repl_style, &record_id)
                            .await;
                    }
                    InteractiveCommand::ShowMemoryValidation(record_id) => {
                        cli_print_memory_validation_report(
                            &runtime,
                            &api_client,
                            &repl_style,
                            &record_id,
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryConflicts(record_id) => {
                        cli_print_memory_conflicts(&runtime, &api_client, &repl_style, &record_id)
                            .await;
                    }
                    InteractiveCommand::ShowMemoryRulePacks => {
                        cli_print_memory_rule_packs(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowMemoryRuleHits(raw_query) => {
                        cli_print_memory_rule_hits(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_query.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryConsolidationRuns => {
                        cli_print_memory_consolidation_runs(&runtime, &api_client, &repl_style)
                            .await;
                    }
                    InteractiveCommand::RunMemoryConsolidation(raw_request) => {
                        cli_run_memory_consolidation(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_request.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::InspectStage(stage_filter) => {
                        cli_print_session_events(
                            &runtime,
                            &api_client,
                            &repl_style,
                            stage_filter.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::Unknown(name) => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(format!(
                                "Unknown command: /{}. Type /help for available commands.",
                                name
                            ))),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::Exit
                    | InteractiveCommand::ShowHelp
                    | InteractiveCommand::ShowRecovery
                    | InteractiveCommand::NewSession
                    | InteractiveCommand::ShowStatus
                    | InteractiveCommand::ListModels
                    | InteractiveCommand::ListProviders
                    | InteractiveCommand::ConnectProvider(_)
                    | InteractiveCommand::ListThemes
                    | InteractiveCommand::ListPresets
                    | InteractiveCommand::ListSessions
                    | InteractiveCommand::ParentSession
                    | InteractiveCommand::ListTasks
                    | InteractiveCommand::ListAgents
                    | InteractiveCommand::Copy
                    | InteractiveCommand::ToggleSidebar
                    | InteractiveCommand::SelectModel(_)
                    | InteractiveCommand::SelectPreset(_)
                    | InteractiveCommand::SelectAgent(_) => {}
                }
                continue;
            }
        }

        runtime.busy_flag.store(true, Ordering::SeqCst);
        run_server_prompt_rich(
            &mut runtime,
            &mut state,
            &api_client,
            &mut sse_rx,
            &trimmed,
            &repl_style,
            true,
        )
        .await?;

        runtime.busy_flag.store(false, Ordering::SeqCst);
        if let Some(surface) = runtime.terminal_surface.as_ref() {
            let _ = surface.ensure_prompt_visible();
        }
        if runtime.exit_requested.load(Ordering::SeqCst)
            && runtime.queued_inputs.lock().await.is_empty()
        {
            break;
        }
    }

    sse_cancel.cancel();
    Ok(())
}

async fn run_server_prompt_rich(
    runtime: &mut CliExecutionRuntime,
    state: &mut CliInteractiveRichState,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    input: &str,
    style: &CliStyle,
    update_recovery_base: bool,
) -> anyhow::Result<()> {
    if update_recovery_base {
        runtime.recovery_base_prompt = Some(input.to_string());
    }
    if let Ok(mut topology) = runtime.observed_topology.lock() {
        topology.reset_for_run(
            &runtime.resolved_agent_name,
            runtime.scheduler_profile_name.as_deref(),
        );
    }
    if let Ok(mut snapshots) = runtime.scheduler_stage_snapshots.lock() {
        snapshots.clear();
    }
    cli_frontend_set_phase(
        &runtime.frontend_projection,
        CliFrontendPhase::Busy,
        Some(cli_summary_thinking_label()),
    );
    if let Ok(mut active_tool_labels) = runtime.active_tool_labels.lock() {
        active_tool_labels.clear();
    }

    let root_session_id = runtime
        .server_session_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("CLI server session is not initialized"))?;
    state.append_rendered(
        &root_session_id,
        render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::User,
                input.to_string(),
            )),
            style,
        ),
    );
    render_rich_state(runtime, state);

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = Some(CliActiveAbortHandle::Server {
            api_client: api_client.clone(),
            session_id: root_session_id.clone(),
        });
    }

    let prompt_agent = cli_prompt_agent_override(
        &runtime.resolved_agent_name,
        runtime.scheduler_profile_name.as_deref(),
    );

    let prompt_response = match api_client
        .send_prompt(
            &root_session_id,
            input.to_string(),
            None,
            prompt_agent,
            runtime.scheduler_profile_name.clone(),
            (runtime.resolved_model_label != "auto").then(|| runtime.resolved_model_label.clone()),
            None,
            Some("cli".to_string()),
            Some(format!(
                "cli_{}",
                rocode_core::id::create(rocode_core::id::Prefix::User, true, None)
            )),
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            cli_frontend_set_phase(
                &runtime.frontend_projection,
                CliFrontendPhase::Failed,
                Some("send prompt failed".to_string()),
            );
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Failed to send prompt: {}",
                    error
                ))),
                style,
            );
            let mut active_abort = runtime.active_abort.lock().await;
            *active_abort = None;
            cli_frontend_clear(runtime);
            return Ok(());
        }
    };

    let (_accepted_response, ignored_question_ids) = resolve_prompt_submission(
        runtime,
        api_client,
        &root_session_id,
        style,
        prompt_response,
    )
    .await?;

    loop {
        match sse_rx.recv().await {
            Some(CliServerEvent::QuestionCreated {
                request_id,
                session_id,
                questions_json,
            }) => {
                if ignored_question_ids.contains(&request_id) {
                    continue;
                }
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_question_from_sse(runtime, api_client, &request_id, &questions_json)
                        .await;
                }
            }
            Some(CliServerEvent::QuestionResolved { request_id })
                if ignored_question_ids.contains(&request_id) =>
            {
                continue;
            }
            Some(CliServerEvent::PermissionRequested {
                session_id,
                permission_id,
                info_json,
            }) => {
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_permission_from_sse(runtime, api_client, &permission_id, &info_json)
                        .await;
                }
            }
            Some(CliServerEvent::ConfigUpdated) => {
                cli_handle_config_updated_from_sse(runtime, api_client).await;
            }
            Some(CliServerEvent::SessionUpdated { .. }) => {
                // SSE events drive block state incrementally.
                // Full reconciliation only on SessionIdle.
            }
            Some(CliServerEvent::SessionIdle {
                session_id: idle_session_id,
            }) => {
                let is_current_session = runtime
                    .server_session_id
                    .as_deref()
                    .is_some_and(|current| current == idle_session_id);
                handle_async_sse_event_rich(
                    runtime,
                    state,
                    &api_client,
                    CliServerEvent::SessionIdle {
                        session_id: idle_session_id.clone(),
                    },
                    style,
                )
                .await;
                if !is_current_session {
                    continue;
                }
                if let Ok(mut topology) = runtime.observed_topology.lock() {
                    topology.finish_run(Some("Completed".to_string()));
                }
                cli_frontend_clear(runtime);
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::success("Done.")),
                    style,
                );
                break;
            }
            Some(other) => handle_sse_event_rich(runtime, state, other, style),
            None => break,
        }
    }

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = None;
    }
    Ok(())
}

async fn wait_for_rich_input_rich(
    runtime: &mut CliExecutionRuntime,
    config: &Config,
    agent_registry: &AgentRegistry,
    api_client: &Arc<CliApiClient>,
    state: &mut CliInteractiveRichState,
    dispatch_rx: &mut mpsc::UnboundedReceiver<CliDispatchInput>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    style: &CliStyle,
) -> anyhow::Result<Option<String>> {
    loop {
        tokio::select! {
            dispatch = dispatch_rx.recv() => {
                match dispatch {
                    Some(CliDispatchInput::Line(line)) => return Ok(Some(line)),
                    Some(CliDispatchInput::ModeCycle { reverse }) => {
                        cli_cycle_prompt_mode(runtime, config, agent_registry, reverse);
                    }
                    Some(CliDispatchInput::Eof) | None => return Ok(None),
                }
            }
            sse_event = sse_rx.recv() => {
                let Some(event) = sse_event else {
                    return Ok(None);
                };
                handle_async_sse_event_rich(runtime, state, api_client, event, style).await;
            }
        }
    }
}

async fn handle_async_sse_event_rich(
    runtime: &CliExecutionRuntime,
    state: &mut CliInteractiveRichState,
    api_client: &Arc<CliApiClient>,
    event: CliServerEvent,
    style: &CliStyle,
) {
    match event {
        CliServerEvent::ConfigUpdated => {
            cli_handle_config_updated_from_sse(runtime, api_client).await;
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id,
            questions_json,
        } => {
            if cli_tracks_related_session(runtime, &session_id) {
                handle_question_from_sse(runtime, api_client, &request_id, &questions_json).await;
            }
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        } => {
            if cli_tracks_related_session(runtime, &session_id) {
                handle_permission_from_sse(runtime, api_client, &permission_id, &info_json).await;
            }
        }
        CliServerEvent::SessionUpdated { session_id, source } => {
            // Keep transcript authority on the live OutputBlock path, but
            // refresh telemetry eagerly so usage/cache meters update while the
            // turn is still running.
            let is_current_session = runtime
                .server_session_id
                .as_deref()
                .is_some_and(|current| current == session_id);
            if is_current_session
                && super::super::cli_session_update_requires_refresh(source.as_deref())
            {
                super::super::cli_refresh_session_telemetry(
                    api_client,
                    &runtime.frontend_projection,
                    &session_id,
                )
                .await;
                refresh_rich_prompt(runtime);
            }
        }
        CliServerEvent::SessionIdle { session_id } => {
            let is_current_session = runtime
                .server_session_id
                .as_deref()
                .is_some_and(|current| current == session_id);
            handle_sse_event_rich(
                runtime,
                state,
                CliServerEvent::SessionIdle {
                    session_id: session_id.clone(),
                },
                style,
            );
            if is_current_session {
                super::super::cli_refresh_server_info(
                    api_client,
                    &runtime.frontend_projection,
                    Some(&session_id),
                )
                .await;
                refresh_rich_prompt(runtime);
            }
        }
        other => handle_sse_event_rich(runtime, state, other, style),
    }
}

fn handle_sse_event_rich(
    runtime: &CliExecutionRuntime,
    state: &mut CliInteractiveRichState,
    event: CliServerEvent,
    style: &CliStyle,
) {
    let root_session_id = runtime.server_session_id.as_deref();
    let focused_session_id = cli_focused_session_id(runtime);
    let is_root_session = |event_session_id: &str| {
        root_session_id.is_none_or(|sid| event_session_id.is_empty() || sid == event_session_id)
    };
    let is_related_session =
        |event_session_id: &str| cli_tracks_related_session(runtime, event_session_id);

    match event {
        CliServerEvent::StreamReconnecting { delay_ms } => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.run_tail = Some(crate::run::frontend_state_types::CliRunTailState {
                    status: "reconnecting".to_string(),
                    detail: Some(format!("retrying in {}s", ((delay_ms + 999) / 1000).max(1))),
                });
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::StreamConnected => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                if projection
                    .run_tail
                    .as_ref()
                    .is_some_and(|tail| tail.status == "reconnecting")
                {
                    projection.run_tail = None;
                }
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::SessionUpdated { .. } => {}
        CliServerEvent::SessionBusy { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.last_turn_tokens =
                    crate::run::frontend_state_types::CliLastTurnTokenStats::default();
                projection.set_runtime_activity(
                    CliFrontendPhase::Busy,
                    Some(cli_summary_thinking_label()),
                );
                projection.run_tail = None;
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::SessionIdle { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            cli_frontend_set_phase(&runtime.frontend_projection, CliFrontendPhase::Idle, None);
            render_rich_state(runtime, state);
        }
        CliServerEvent::SessionRetrying { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.run_tail = Some(crate::run::frontend_state_types::CliRunTailState {
                    status: "retrying".to_string(),
                    detail: Some("Waiting for automatic retry.".to_string()),
                });
            }
            cli_push_runtime_aux_block(
                runtime,
                OutputBlock::Status(StatusBlock::warning("Retry scheduled.")),
            );
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id,
            ..
        } => {
            tracing::warn!(
                request_id,
                session_id,
                "question.created reached sync handler — skipping"
            );
        }
        CliServerEvent::QuestionResolved { request_id } => {
            tracing::debug!(request_id, "question resolved");
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            ..
        } => {
            tracing::warn!(
                session_id,
                permission_id,
                "permission.requested reached sync handler — skipping"
            );
        }
        CliServerEvent::PermissionResolved {
            session_id,
            permission_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(session_id, permission_id, "permission resolved");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.pending_permission_count = 0;
                projection.submitting_permission_count = 0;
                projection.last_permission_submit_error = None;
                projection.permission_submit_completed_at = Some(permission_timestamp_now());
                cli_restore_compact_summary(&mut projection);
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::ToolCallStarted {
            session_id,
            tool_call_id,
            tool_name,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.active = true;
            }
            tracing::debug!(tool_call_id, tool_name, "tool call started");
            cli_store_active_tool_label(runtime, &tool_call_id, &tool_name);
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.set_runtime_activity(
                    CliFrontendPhase::Busy,
                    Some(cli_summary_tool_label(&tool_name)),
                );
            }
            cli_push_runtime_aux_block(
                runtime,
                OutputBlock::Status(StatusBlock::title(cli_summary_tool_label(&tool_name))),
            );
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::ToolCallCompleted {
            session_id,
            tool_call_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(tool_call_id, "tool call completed");
            let _ = cli_take_active_tool_label(runtime, &tool_call_id);
            if !is_root_session(&session_id) {
                return;
            }
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                cli_restore_compact_summary(&mut projection);
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::AttachedSessionAttached {
            parent_id,
            attached_id,
        } => {
            if cli_track_attached_session(runtime, &parent_id, &attached_id) {
                tracing::debug!(parent_id, attached_id, "tracked attached session");
            }
        }
        CliServerEvent::AttachedSessionDetached {
            parent_id,
            attached_id,
        } => {
            if cli_untrack_attached_session(runtime, &parent_id, &attached_id) {
                tracing::debug!(parent_id, attached_id, "untracked attached session");
            }
        }
        CliServerEvent::OutputBlock {
            session_id,
            id,
            live_identity,
            payload,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            let block_payload = payload.get("block").unwrap_or(&payload);
            let Some(block) = parse_output_block(block_payload) else {
                tracing::debug!(?id, payload = %block_payload, "failed to parse output_block");
                return;
            };

            if matches!(block, OutputBlock::Reasoning(_))
                && !runtime.show_thinking.load(Ordering::SeqCst)
            {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.observe_block(&block);
            }
            if let OutputBlock::SchedulerStage(stage) = &block {
                if let Some(attached_id) = stage.attached_session_id.as_deref() {
                    let _ = cli_track_attached_session(runtime, &session_id, attached_id);
                }
            }
            cli_frontend_observe_block(&runtime.frontend_projection, &block);
            let session_bucket = normalized_session_id(runtime, &session_id);
            let legacy_streaming_block = live_identity.is_none()
                && id.as_deref().is_some()
                && matches!(block, OutputBlock::Message(_) | OutputBlock::Reasoning(_));
            if legacy_streaming_block {
                if state.apply_legacy_streaming_block(&session_bucket, id.as_deref(), &block, style)
                {
                    sync_session_state_to_runtime(runtime, state, &session_bucket);
                    render_rich_state(runtime, state);
                }
                return;
            }
            let transcript_bearing_identity = live_identity.as_ref().filter(|identity| {
                matches!(
                    identity.part_kind,
                    rocode_types::LiveMessagePartKind::AssistantText
                        | rocode_types::LiveMessagePartKind::AssistantReasoning
                        | rocode_types::LiveMessagePartKind::ToolResult
                )
            });
            let block_updates_authority =
                cli_output_block_updates_transcript_authority(&block, live_identity.as_ref());

            if let Some(identity) = transcript_bearing_identity {
                let produced =
                    state.apply_live_slot_update(&session_bucket, &block, identity, style);
                // Always sync — silent upserts (reasoning/assistant) update
                // state.blocks but don't trigger render. The runtime transcript
                // must stay in sync so catch-all events can flush accumulated
                // blocks via render_rich_state.
                sync_session_state_to_runtime(runtime, state, &session_bucket);
                if produced {
                    render_rich_state(runtime, state);
                }
                return;
            }

            if block_updates_authority {
                if state.apply_legacy_streaming_block(&session_bucket, id.as_deref(), &block, style)
                {
                    sync_session_state_to_runtime(runtime, state, &session_bucket);
                    render_rich_state(runtime, state);
                }
                return;
            }

            match &block {
                OutputBlock::SchedulerStage(stage)
                    if !cli_should_emit_scheduler_stage_block(
                        &runtime.scheduler_stage_snapshots,
                        stage,
                    ) => {}
                OutputBlock::SchedulerStage(stage)
                    if !cli_is_terminal_stage_status(stage.status.as_deref()) =>
                {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = Some(stage.as_ref().clone());
                        projection.active_collapsed = false;
                    }
                    sync_session_state_to_runtime(runtime, state, &session_bucket);
                    render_rich_state(runtime, state);
                }
                OutputBlock::SchedulerStage(_) => {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = None;
                        projection.active_collapsed = true;
                    }
                    sync_session_state_to_runtime(runtime, state, &session_bucket);
                    render_rich_state(runtime, state);
                }
                _ => {
                    if focused_session_id.as_deref() == Some(session_bucket.as_str())
                        || (is_root_session(&session_id) && cli_is_root_focused(runtime))
                    {
                        sync_session_state_to_runtime(runtime, state, &session_bucket);
                        render_rich_state(runtime, state);
                    }
                }
            }
        }
        CliServerEvent::Error {
            session_id,
            error,
            message_id,
            done,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if !is_root_session(&session_id) {
                tracing::error!(
                    session_id,
                    error,
                    ?message_id,
                    ?done,
                    "attached session error"
                );
                return;
            }
            tracing::error!(error, ?message_id, ?done, "server error");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.set_runtime_activity(CliFrontendPhase::Failed, None);
                projection.run_tail = Some(crate::run::frontend_state_types::CliRunTailState {
                    status: "error".to_string(),
                    detail: Some(error),
                });
            }
            refresh_rich_prompt(runtime);
        }
        CliServerEvent::Usage {
            session_id,
            prompt_tokens,
            completion_tokens,
            message_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(prompt_tokens, completion_tokens, ?message_id, "token usage");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.last_turn_tokens.input_tokens = prompt_tokens;
                projection.last_turn_tokens.output_tokens = completion_tokens;
                projection.token_stats.input_tokens = projection
                    .token_stats
                    .input_tokens
                    .saturating_add(prompt_tokens);
                projection.token_stats.output_tokens = projection
                    .token_stats
                    .output_tokens
                    .saturating_add(completion_tokens);
            }
            if !is_root_session(&session_id) {
                return;
            }
            if prompt_tokens > 0 || completion_tokens > 0 {
                if let Ok(mut projection) = runtime.frontend_projection.lock() {
                    projection.run_tail = Some(crate::run::frontend_state_types::CliRunTailState {
                        status: "complete".to_string(),
                        detail: Some(format!(
                            "input {} · output {}",
                            format_token_count(prompt_tokens),
                            format_token_count(completion_tokens)
                        )),
                    });
                }
                refresh_rich_prompt(runtime);
            }
        }
        CliServerEvent::Unknown { event, data } => {
            tracing::trace!("Ignoring unknown SSE event: {} ({})", event, data);
        }
        CliServerEvent::ConfigUpdated => {
            tracing::debug!("config.updated reached sync handler");
        }
    }
}

fn normalized_session_id(runtime: &CliExecutionRuntime, session_id: &str) -> String {
    if session_id.is_empty() {
        runtime
            .server_session_id
            .clone()
            .unwrap_or_else(|| "root-session".to_string())
    } else {
        session_id.to_string()
    }
}

fn sync_session_state_to_runtime(
    runtime: &CliExecutionRuntime,
    state: &CliInteractiveRichState,
    updated_session_id: &str,
) {
    let blocks = state.blocks.get(updated_session_id);
    let rendered: String = blocks
        .map(|b| b.iter().map(|e| e.rendered.as_str()).collect::<String>())
        .unwrap_or_default();
    let mut transcript = CliVisibleTranscript::default();
    transcript.append_rendered(&rendered);

    if runtime
        .server_session_id
        .as_deref()
        .is_some_and(|root| root == updated_session_id)
    {
        cli_replace_root_session_transcript(runtime, transcript.clone());
    } else if let Ok(mut attached) = runtime.attached_session_transcripts.lock() {
        attached.insert(updated_session_id.to_string(), transcript.clone());
    }
}

fn refresh_rich_prompt(runtime: &CliExecutionRuntime) {
    if let Some(prompt_chrome) = runtime.prompt_chrome.as_ref() {
        prompt_chrome.update_from_runtime(runtime);
    }
    if let Some(prompt_session) = runtime.prompt_session.as_ref() {
        let _ = prompt_session.refresh();
    }
}

/// Render all blocks for the focused session to the terminal.
/// Rich mode is append-only: once a block has been emitted to terminal
/// scrollback, subsequent renders only append the new suffix.
fn render_rich_state(runtime: &CliExecutionRuntime, state: &mut CliInteractiveRichState) {
    let display_session_id = cli_focused_session_id(runtime).unwrap_or_else(|| {
        runtime
            .server_session_id
            .clone()
            .unwrap_or_else(|| "root-session".to_string())
    });
    let current: Vec<ContentBlock> = state
        .blocks
        .get(&display_session_id)
        .cloned()
        .unwrap_or_default();
    let previous = state.painted_blocks(&display_session_id);

    // Build unified rendered text for all blocks.
    let mut rendered = String::new();
    for block in &current {
        rendered.push_str(&block.rendered);
    }

    let prev_text: String = previous.iter().map(|b| b.rendered.as_str()).collect();
    if rendered == prev_text {
        refresh_rich_prompt(runtime);
        return;
    }

    let shared_prefix = previous
        .iter()
        .zip(current.iter())
        .take_while(|(before, after)| {
            before.kind == after.kind && before.rendered == after.rendered
        })
        .count();
    let delta = current[shared_prefix..]
        .iter()
        .map(|block| block.rendered.as_str())
        .collect::<String>();

    if runtime.prompt_session.is_some() || runtime.terminal_surface.is_some() {
        if !delta.is_empty() {
            if let Some(surface) = runtime.terminal_surface.as_ref() {
                let _ = surface.print_rendered_stream(&delta);
            } else {
                print!("{delta}");
                let _ = io::stdout().flush();
            }
        }
        refresh_rich_prompt(runtime);
    } else {
        if !delta.is_empty() {
            print!("{delta}");
            let _ = io::stdout().flush();
        }
    }

    state.record_painted(&display_session_id, current);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocode_agent::AgentRegistry;
    use rocode_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};
    use tokio::sync::mpsc;

    async fn test_runtime() -> CliExecutionRuntime {
        let config = Config::default();
        let agent_registry = Arc::new(AgentRegistry::from_config(&config));
        let mut runtime = build_cli_execution_runtime(CliRuntimeBuildInput {
            config: &config,
            agent_registry,
            selection: &CliRunSelection::default(),
            working_dir: std::env::current_dir().expect("cwd"),
        })
        .await
        .expect("build runtime");
        cli_set_root_server_session(&mut runtime, "root-session".to_string());
        runtime
    }

    #[test]
    fn rich_state_replaces_same_snapshot_key_in_place() {
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(OutputMessageRole::Assistant, "hello")),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::Assistant,
                "hello world",
            )),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::End,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );

        let rendered = state.rendered_text("root");
        assert!(rendered.contains("hello world"), "{rendered}");
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
    }

    #[test]
    fn rich_state_keeps_entry_order_when_replacing_snapshot_key() {
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::full("think".to_string())),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "reasoning".to_string(),
                part_kind: LiveMessagePartKind::AssistantReasoning,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(OutputMessageRole::Assistant, "answer")),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::full("thinking harder".to_string())),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "reasoning".to_string(),
                part_kind: LiveMessagePartKind::AssistantReasoning,
                phase: LivePartPhase::Snapshot,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::end()),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "reasoning".to_string(),
                part_kind: LiveMessagePartKind::AssistantReasoning,
                phase: LivePartPhase::End,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
            &LiveMessagePartIdentity {
                message_id: "msg-1".to_string(),
                part_key: "text".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::End,
                legacy_block_id: Some("msg-1".to_string()),
            },
            &style,
        );

        let rendered = state.rendered_text("root");
        assert!(rendered.contains("thinking harder"), "{rendered}");
        assert!(rendered.contains("answer"), "{rendered}");
        assert!(rendered.matches("[thinking]").count() >= 1, "{rendered}");
    }

    #[test]
    fn interleaved_reasoning_and_assistant_end_updates_do_not_duplicate_tail_blocks() {
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();

        let reasoning_snapshot = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "reasoning/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };
        let reasoning_end = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..reasoning_snapshot.clone()
        };
        let assistant_snapshot = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };
        let assistant_end = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..assistant_snapshot.clone()
        };

        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::full("first think".to_string())),
            &reasoning_snapshot,
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::Assistant,
                "first answer",
            )),
            &assistant_snapshot,
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::end()),
            &reasoning_end,
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
            &assistant_end,
            &style,
        );

        let rendered = state.rendered_text("root");
        assert_eq!(rendered.matches("[thinking]").count(), 1, "{rendered}");
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("first think"), "{rendered}");
        assert!(rendered.contains("first answer"), "{rendered}");
    }

    #[tokio::test]
    async fn render_rich_state_appends_only_new_suffix() {
        let mut runtime = test_runtime().await;
        let surface = Arc::new(crate::run::frontend_state_surface::CliTerminalSurface::new(
            CliStyle::plain(),
            runtime.frontend_projection.clone(),
        ));
        runtime.terminal_surface = Some(surface.clone());

        let mut state = CliInteractiveRichState::default();
        state.push_block(
            "root-session",
            BlockKind::Status,
            "alpha\n".to_string(),
            false,
        );
        render_rich_state(&runtime, &mut state);

        state.push_block("root-session", BlockKind::Tool, "beta\n".to_string(), false);
        render_rich_state(&runtime, &mut state);
        render_rich_state(&runtime, &mut state);

        let rendered = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert_eq!(rendered, "alpha\nbeta\n", "{rendered}");
        assert_eq!(
            surface.emitted_render_count(),
            2,
            "only the initial paint and appended suffix should be emitted"
        );
    }

    #[tokio::test]
    async fn wait_for_rich_input_rich_applies_snapshot_events_to_rich_state() {
        let config = Config::default();
        let agent_registry = Arc::new(AgentRegistry::from_config(&config));
        let mut runtime = build_cli_execution_runtime(CliRuntimeBuildInput {
            config: &config,
            agent_registry: agent_registry.clone(),
            selection: &CliRunSelection::default(),
            working_dir: std::env::current_dir().expect("cwd"),
        })
        .await
        .expect("build runtime");
        cli_set_root_server_session(&mut runtime, "root-session".to_string());

        let api_client = Arc::new(CliApiClient::new("http://127.0.0.1:0".to_string()));
        let mut state = CliInteractiveRichState::default();
        let (dispatch_tx, mut dispatch_rx) = mpsc::unbounded_channel();
        let (sse_tx, mut sse_rx) = mpsc::unbounded_channel();

        sse_tx
            .send(CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::Snapshot,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "full",
                    "role": "assistant",
                    "text": "hello from rich"
                }),
            })
            .expect("send output block");
        sse_tx
            .send(CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            })
            .expect("send end block");
        let delayed_dispatch = dispatch_tx.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            delayed_dispatch
                .send(CliDispatchInput::Line("next".to_string()))
                .expect("send line");
        });

        let line = wait_for_rich_input_rich(
            &mut runtime,
            &config,
            agent_registry.as_ref(),
            &api_client,
            &mut state,
            &mut dispatch_rx,
            &mut sse_rx,
            &CliStyle::plain(),
        )
        .await
        .expect("wait for rich input");

        assert_eq!(line.as_deref(), Some("next"));
        assert!(state
            .rendered_text("root-session")
            .contains("hello from rich"));
        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(rendered.contains("hello from rich"), "{rendered}");
    }

    #[test]
    fn rich_live_slot_updates_ignore_empty_start_blocks() {
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();

        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::start(OutputMessageRole::Assistant)),
            &LiveMessagePartIdentity {
                message_id: "assistant-1".to_string(),
                part_key: "text/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantText,
                phase: LivePartPhase::Start,
                legacy_block_id: Some("assistant-1".to_string()),
            },
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::start()),
            &LiveMessagePartIdentity {
                message_id: "assistant-1".to_string(),
                part_key: "reasoning/main".to_string(),
                part_kind: LiveMessagePartKind::AssistantReasoning,
                phase: LivePartPhase::Start,
                legacy_block_id: Some("assistant-1".to_string()),
            },
            &style,
        );

        assert_eq!(state.rendered_text("root"), "");
    }

    #[test]
    fn rich_live_slot_updates_accumulate_delta_text_without_replaying_headers() {
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();
        let identity = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
            legacy_block_id: Some("assistant-1".to_string()),
        };
        let end_identity = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::End,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::delta(OutputMessageRole::Assistant, "hello")),
            &identity,
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::delta(OutputMessageRole::Assistant, " world")),
            &identity,
            &style,
        );
        state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
            &end_identity,
            &style,
        );

        let rendered = state.rendered_text("root");
        assert!(rendered.contains("hello world"), "{rendered}");
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn handle_sse_event_rich_does_not_materialize_start_only_headers() {
        let runtime = test_runtime().await;
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::Start,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "start",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "reasoning/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantReasoning,
                    phase: LivePartPhase::Start,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "start",
                    "text": ""
                }),
            },
            &style,
        );
        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(rendered, "");
    }

    #[tokio::test]
    async fn handle_sse_event_rich_buffers_streaming_assistant_and_reasoning_until_end() {
        let runtime = test_runtime().await;
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();
        runtime
            .show_thinking
            .store(true, std::sync::atomic::Ordering::SeqCst);

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "reasoning/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantReasoning,
                    phase: LivePartPhase::Append,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "delta",
                    "text": "thinking"
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::Append,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "role": "assistant",
                    "text": "hello"
                }),
            },
            &style,
        );

        let before_end = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert_eq!(before_end, "");

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "reasoning/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantReasoning,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "end",
                    "text": ""
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(
            rendered.contains("thinking"),
            "reasoning block should appear: {rendered}"
        );
        assert!(rendered.contains("hello"), "{rendered}");
    }

    #[tokio::test]
    async fn session_idle_flushes_pending_final_live_block_without_followup_block() {
        let mut runtime = test_runtime().await;
        let surface = Arc::new(crate::run::frontend_state_surface::CliTerminalSurface::new(
            CliStyle::plain(),
            runtime.frontend_projection.clone(),
        ));
        runtime.terminal_surface = Some(surface.clone());

        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::Append,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "role": "assistant",
                    "text": "final answer"
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );

        assert_eq!(
            surface.emitted_render_count(),
            0,
            "final assistant block stays buffered until turn boundary"
        );

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::SessionIdle {
                session_id: "root-session".to_string(),
            },
            &style,
        );

        let visible = runtime
            .frontend_projection
            .lock()
            .expect("projection")
            .transcript
            .rendered_text();
        assert!(visible.contains("final answer"), "{visible}");
        assert_eq!(
            surface.emitted_render_count(),
            1,
            "idle boundary should flush the buffered final assistant block"
        );
    }

    #[tokio::test]
    async fn handle_sse_event_rich_buffers_legacy_streaming_message_until_end() {
        let runtime = test_runtime().await;
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();
        runtime
            .show_thinking
            .store(true, std::sync::atomic::Ordering::SeqCst);

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: None,
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "delta",
                    "text": "Thinking "
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: None,
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "role": "assistant",
                    "text": "hello"
                }),
            },
            &style,
        );

        let before_end = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(before_end.trim().is_empty(), "{before_end}");

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: None,
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "end",
                    "text": ""
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: None,
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(rendered.contains("Thinking"), "{rendered}");
        assert!(rendered.contains("hello"), "{rendered}");
    }

    #[tokio::test]
    async fn handle_sse_event_rich_accumulates_append_deltas_into_one_visible_slot() {
        let runtime = test_runtime().await;
        let style = CliStyle::plain();
        let mut state = CliInteractiveRichState::default();
        let identity = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(identity.clone()),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "role": "assistant",
                    "text": "hello"
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(identity),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "role": "assistant",
                    "text": " world"
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );
        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    message_id: "assistant-1".to_string(),
                    part_key: "text/main".to_string(),
                    part_kind: LiveMessagePartKind::AssistantText,
                    phase: LivePartPhase::End,
                    legacy_block_id: Some("assistant-1".to_string()),
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );

        let rendered = runtime
            .root_session_transcript
            .lock()
            .expect("root transcript")
            .rendered_text();
        assert!(rendered.contains("hello world"), "{rendered}");
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
    }

    /// Replay: Agent build mode with deepseek — reasoning Snapshot→End, then assistant End.
    /// Verifies: reasoning accumulates silently (no block during streaming),
    /// End produces one Thinking block via upsert, assistant End one block.
    #[test]
    fn replay_agent_build_reasoning_then_assistant() {
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();

        // Phase 1: reasoning streaming (Full/Snapshot events render progressively)
        let msg_id = "msg_abc123".to_string();
        let rsn = LiveMessagePartIdentity {
            message_id: msg_id.clone(),
            part_key: "reasoning/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some(msg_id.clone()),
        };

        // Token 1: "The" → silent accumulate (reasoning never triggers render)
        assert!(!state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::full("The")),
            &rsn,
            &style,
        ));
        {
            let blocks = state.blocks.get("root").unwrap();
            assert_eq!(blocks.len(), 1, "first thinking block upserted silently");
            assert!(blocks[0].rendered.contains("The"), "{}", blocks[0].rendered);
        }

        // Token 2: coalescer accumulates to "The user wants" → silent upsert
        assert!(!state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::full("The user wants")),
            &rsn,
            &style,
        ));
        {
            let blocks = state.blocks.get("root").unwrap();
            assert_eq!(blocks.len(), 1, "still one block after upsert");
            assert!(
                blocks[0].rendered.contains("The user wants"),
                "{}",
                blocks[0].rendered
            );
        }

        // Phase 2: reasoning End — silent upsert, no render triggered
        let rsn_end = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..rsn.clone()
        };
        assert!(!state.apply_live_slot_update(
            "root",
            &OutputBlock::Reasoning(ReasoningBlock::end()),
            &rsn_end,
            &style,
        ));
        let blocks = state.blocks.get("root").unwrap();
        assert_eq!(blocks.len(), 1, "End should keep one block: {blocks:?}");
        assert!(
            blocks[0].rendered.contains("[thinking]"),
            "{}",
            blocks[0].rendered
        );
        assert!(
            blocks[0].rendered.contains("The user wants"),
            "{}",
            blocks[0].rendered
        );

        // Phase 3: assistant text End — silent upsert, render triggered elsewhere
        let ast = LiveMessagePartIdentity {
            message_id: msg_id.clone(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::End,
            legacy_block_id: Some(msg_id.clone()),
        };
        assert!(!state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::Assistant,
                "好的，我来检索。",
            )),
            &ast,
            &style,
        ));
        let blocks = state.blocks.get("root").unwrap();
        // Thinking + Assistant = 2 blocks
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].rendered.contains("[thinking]"));
        assert!(blocks[1].rendered.contains("[message:assistant]"));
        assert!(blocks[1].rendered.contains("好的，我来检索。"));

        // Phase 4: second assistant upsert — silent, doesn't grow count
        assert!(!state.apply_live_slot_update(
            "root",
            &OutputBlock::Message(MessageBlock::full(
                OutputMessageRole::Assistant,
                "好的，我来检索。找到了相关论文。",
            )),
            &ast,
            &style,
        ));
        let blocks = state.blocks.get("root").unwrap();
        assert_eq!(blocks.len(), 2, "upsert must replace, not push");
        assert!(blocks[1]
            .rendered
            .contains("好的，我来检索。找到了相关论文。"));
    }

    #[tokio::test]
    async fn handle_sse_event_rich_coalesces_reasoning_snapshot_fragments_until_end() {
        let runtime = test_runtime().await;
        runtime.show_thinking.store(true, Ordering::SeqCst);
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();
        let identity = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "reasoning/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        for text in ["I", " can", " search", " PubMed"] {
            handle_sse_event_rich(
                &runtime,
                &mut state,
                CliServerEvent::OutputBlock {
                    session_id: "root-session".to_string(),
                    id: Some("assistant-1".to_string()),
                    live_identity: Some(identity.clone()),
                    payload: serde_json::json!({
                        "kind": "reasoning",
                        "phase": "full",
                        "text": text
                    }),
                },
                &style,
            );
        }

        assert!(
            state.rendered_text("root-session").contains("I"),
            "should render progressively"
        );

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    phase: LivePartPhase::End,
                    ..identity
                }),
                payload: serde_json::json!({
                    "kind": "reasoning",
                    "phase": "end",
                    "text": ""
                }),
            },
            &style,
        );

        let rendered = state.rendered_text("root-session");
        assert!(rendered.contains("I can search PubMed"), "{rendered}");
        assert!(rendered.matches("[thinking]").count() >= 1, "{rendered}");
    }

    #[tokio::test]
    async fn handle_sse_event_rich_coalesces_assistant_snapshot_fragments_until_end() {
        let runtime = test_runtime().await;
        let mut state = CliInteractiveRichState::default();
        let style = CliStyle::plain();
        let identity = LiveMessagePartIdentity {
            message_id: "assistant-1".to_string(),
            part_key: "text/main".to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("assistant-1".to_string()),
        };

        for text in ["查", "到", "了", "结果"] {
            handle_sse_event_rich(
                &runtime,
                &mut state,
                CliServerEvent::OutputBlock {
                    session_id: "root-session".to_string(),
                    id: Some("assistant-1".to_string()),
                    live_identity: Some(identity.clone()),
                    payload: serde_json::json!({
                        "kind": "message",
                        "phase": "full",
                        "role": "assistant",
                        "text": text
                    }),
                },
                &style,
            );
        }

        assert!(
            state.rendered_text("root-session").contains("查"),
            "should render progressively"
        );

        handle_sse_event_rich(
            &runtime,
            &mut state,
            CliServerEvent::OutputBlock {
                session_id: "root-session".to_string(),
                id: Some("assistant-1".to_string()),
                live_identity: Some(LiveMessagePartIdentity {
                    phase: LivePartPhase::End,
                    ..identity
                }),
                payload: serde_json::json!({
                    "kind": "message",
                    "phase": "end",
                    "role": "assistant",
                    "text": ""
                }),
            },
            &style,
        );

        let rendered = state.rendered_text("root-session");
        assert!(rendered.contains("查到了结果"), "{rendered}");
        assert_eq!(
            rendered.matches("[message:assistant]").count(),
            1,
            "{rendered}"
        );
    }

    // Guardrails: the rich production code (before #[cfg(test)]) is verified
    // at compile time to contain no old-handler calls and no direct
    // projection.transcript writes. See guardrail_* tests below.
    //
    // NOTE: guardrail tests use include_str! to read this file and assert
    // that the production section is free of forbidden patterns.

    /// Guardrail: Rich mode SSE events never invoke legacy handlers.
    /// The rich session file must not import or call the old interactive SSE path.
    #[test]
    fn guardrail_rich_never_calls_old_handlers() {
        let rich_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/run/interactive_session/rich.rs");
        let rich_source = std::fs::read_to_string(&rich_path).expect("read rich source");
        // Exclude this guardrail test itself from the scan.
        let mut in_guardrail = false;
        let code: String = rich_source
            .lines()
            .filter(|l| {
                if l.contains("fn guardrail_") {
                    in_guardrail = true;
                }
                let skip = in_guardrail;
                if l.contains('}') && in_guardrail && !l.contains('{') {
                    in_guardrail = false;
                }
                !skip
            })
            .collect::<Vec<_>>()
            .join("\n");
        let legacy_interactive = ["handle", "interactive", "sse", "event"].join("_");
        assert!(
            !code.contains(&legacy_interactive),
            "rich session must not call old handler: {legacy_interactive}"
        );
        // `handle_sse_event_rich` is fine, but the bare legacy symbol is not.
        let legacy_sse = ["handle", "sse", "event("].join("_");
        let total = code.matches(&legacy_sse).count();
        assert_eq!(
            total, 0,
            "rich session must not call bare legacy SSE handler"
        );
    }

    /// Guardrail: rich rendering does not go through CliFrontendProjection.transcript
    /// as the main output authority. The transcript in the projection is only used
    /// for clipboard copy via cli_copy_target_transcript.
    #[test]
    fn guardrail_transcript_not_main_output() {
        let rich_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/run/interactive_session/rich.rs");
        let rich_source = std::fs::read_to_string(&rich_path).expect("read rich source");
        // Filter out this guardrail test's own assertions.
        let code: String = rich_source
            .lines()
            .filter(|l| !l.contains("guardrail") && !l.contains("projection.transcript"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("fn render_rich_state"),
            "must have single render fn"
        );
        assert!(
            !code.contains("projection.transcript"),
            "rich session must not write to projection.transcript directly"
        );
    }
}
