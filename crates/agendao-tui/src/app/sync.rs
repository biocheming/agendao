use super::*;
use agendao_server_core::frontend_events::FrontendEvent;
use crate::context::collect_attached_sessions_from_stage_summaries;

pub(super) fn session_update_requires_sync(source: Option<&str>) -> bool {
    // P1-2: exclude high-frequency sources that are handled incrementally
    // via output blocks. "topology" (ReconcileReason::Topology) is the new
    // canonical source for scheduler stage deltas — it fires on every stage
    // message change and must NOT trigger a full session sync.
    !matches!(
        source,
        Some(
            "prompt.stream"
                | "stream.prompt"
                | "prompt.scheduler.stage.content"
                | "prompt.scheduler.stage.reasoning"
                | "prompt.scheduler.stage.child.final"
                | "direct_bridge"
                | "topology"
        )
    )
}

impl App {
    fn session_status_from_runtime_kind(status: &crate::api::SessionRunStatusKind) -> SessionStatus {
        match status {
            crate::api::SessionRunStatusKind::Idle => SessionStatus::Idle,
            crate::api::SessionRunStatusKind::Running => SessionStatus::Running,
            crate::api::SessionRunStatusKind::Compacting => SessionStatus::Compacting,
            crate::api::SessionRunStatusKind::WaitingOnUser => SessionStatus::WaitingOnUser,
            crate::api::SessionRunStatusKind::WaitingOnTool
            | crate::api::SessionRunStatusKind::Cancelling
            | crate::api::SessionRunStatusKind::Blocked
            | crate::api::SessionRunStatusKind::Sleeping => SessionStatus::Running,
        }
    }

    pub(super) fn apply_frontend_event(&mut self, event: &FrontendEvent) {
        match event {
            FrontendEvent::SessionRuntimeReplaced { session_id, runtime } => {
                self.context.apply_session_runtime_snapshot(runtime.clone());
                self.apply_session_run_status_change(
                    session_id,
                    Self::session_status_from_runtime_kind(&runtime.run_status),
                );
                self.reconcile_pending_user_inputs_from_runtime();
                if self.should_surface_event_for_session(session_id) {
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::SessionProjectionReplaced {
                session_id,
                topology,
                stages,
                usage,
                usage_books,
                context_compaction_summary,
                context_compaction_lifecycle_summary,
                cache_semantics,
                context_closure_contract,
            } => {
                self.context.apply_session_projection_snapshot(
                    session_id,
                    topology.clone(),
                    stages.clone(),
                    usage.clone(),
                    usage_books.clone(),
                    context_compaction_summary.clone(),
                    context_compaction_lifecycle_summary.clone(),
                    cache_semantics.clone(),
                    context_closure_contract.clone(),
                );
                if self.current_session_id().as_deref() == Some(session_id.as_str()) {
                    self.refresh_attached_sessions();
                    if self.status_dialog.is_open() {
                        self.refresh_active_status_dialog();
                    }
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::QuestionUpsert {
                session_id,
                question,
            } => {
                if self.should_surface_event_for_session(session_id) {
                    self.upsert_question_request(question.clone());
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::QuestionRemoved {
                session_id,
                question_id,
            } => {
                if self.should_surface_event_for_session(session_id) {
                    let reopening_current = self
                        .question_prompt
                        .current()
                        .is_some_and(|question| question.id == *question_id);
                    self.clear_question_tracking(question_id);
                    if reopening_current {
                        self.question_prompt.close();
                        self.open_next_question_prompt();
                    }
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::PermissionUpsert {
                session_id,
                permission,
            } => {
                if self.should_surface_event_for_session(session_id) {
                    self.enqueue_permission_request(permission.clone());
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::PermissionRemoved {
                session_id,
                permission_id,
                ..
            } => {
                if self.should_surface_event_for_session(session_id) {
                    self.clear_permission_request(permission_id);
                    self.permission_runtime.last_submit_error = None;
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::ToolCallUpsert {
                session_id,
                tool_call_id,
                tool_name,
                phase,
            } => {
                self.context.apply_tool_call_upsert(
                    session_id,
                    tool_call_id,
                    tool_name,
                    phase.clone(),
                );
                if self.should_surface_event_for_session(session_id) {
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::DiffReplaced { session_id, diffs } => {
                let mapped = diffs
                    .iter()
                    .map(|entry| crate::context::DiffEntry {
                        file: entry.path.clone(),
                        additions: entry.additions as u32,
                        deletions: entry.deletions as u32,
                    })
                    .collect::<Vec<_>>();
                let mut session_ctx = self.context.session.write();
                session_ctx.session_diff.insert(session_id.clone(), mapped);
                drop(session_ctx);
                if self.should_surface_event_for_session(session_id) {
                    self.event_caused_change = true;
                }
            }
            FrontendEvent::OutputBlockAppended {
                session_id,
                id,
                block,
                live_identity,
            } => {
                self.apply_output_block_change(session_id, id, block, live_identity);
            }
        }
    }

    pub(super) fn queue_question_sync(&mut self) {
        self.sync_runtime.pending_question_sync_due_at =
            Some(Instant::now() + Duration::from_millis(QUESTION_SYNC_DEBOUNCE_MS));
    }

    pub(super) fn queue_permission_sync(&mut self) {
        self.sync_runtime.pending_permission_sync_due_at =
            Some(Instant::now() + Duration::from_millis(PERMISSION_SYNC_DEBOUNCE_MS));
    }

    pub(super) fn queue_process_refresh(&mut self) {
        self.sync_runtime.pending_process_refresh_due_at =
            Some(Instant::now() + Duration::from_millis(PROCESS_REFRESH_DEBOUNCE_MS));
    }

    pub(super) fn reconcile_pending_user_inputs_from_runtime(&mut self) {
        let Some(runtime) = self.context.session_runtime() else {
            return;
        };
        if self.local_direct {
            if let Some(question) = runtime.pending_question.as_ref() {
                if !self.question_runtime.pending_ids.contains(&question.request_id) {
                    self.enqueue_question_request(
                        runtime.session_id.clone(),
                        question.request_id.clone(),
                        Some(question.questions.clone()),
                    );
                }
            }
            if let Some(permission) = runtime.pending_permission.as_ref() {
                if !self
                    .permission_runtime
                    .pending_ids
                    .contains(&permission.permission_id)
                {
                    self.queue_permission_sync();
                }
            }
            return;
        }
        if runtime.pending_question.is_some() {
            self.queue_question_sync();
        }
        if runtime.pending_permission.is_some() {
            self.queue_permission_sync();
        }
    }

    pub(super) fn handle_session_updated_reconcile(
        &mut self,
        session_id: &str,
        source: Option<&str>,
    ) {
        self.diagnostics.perf.session_updated_events = self
            .diagnostics
            .perf
            .session_updated_events
            .saturating_add(1);
        // P1-2/P1-3: session.updated is the RECONCILE FALLBACK path.
        // Incremental updates (output blocks, custom events) are the primary
        // refresh mechanism. This handler only triggers a debounced full sync
        // for non-droppable reconcile reasons.
        if let Route::Session { session_id: active } = self.context.current_route() {
            if active == session_id && session_update_requires_sync(source) {
                self.sync_runtime.pending_session_sync = Some(session_id.to_string());
                self.sync_runtime.pending_session_sync_due_at =
                    Some(Instant::now() + Duration::from_millis(SESSION_SYNC_DEBOUNCE_MS));
            }
        }
        self.sync_prompt_spinner_state();
    }

    fn is_optimistic_local_session_id(session_id: &str) -> bool {
        session_id.starts_with("local_session_")
    }

    pub(super) fn sync_config_from_server(&mut self) -> anyhow::Result<()> {
        self.context.sync_ui_preferences_from_server()?;
        self.refresh_theme_list_dialog();
        self.refresh_model_dialog();
        self.refresh_agent_dialog();
        self.sync_command_palette_labels();
        self.sync_prompt_spinner_style();
        Ok(())
    }

    pub(crate) fn ensure_session_view(&mut self, session_id: &str) {
        let had_active_view = self
            .context
            .session_view_handle()
            .as_ref()
            .is_some_and(|view| view.session_id() == session_id);

        self.context.ensure_session_view_handle(session_id);

        // Update the SSE session filter so the listener reconnects
        // with server-side filtering for this session.
        let filter_changed = self.sse_session_filter.borrow().as_deref() != Some(session_id);
        if filter_changed {
            self.sse_session_filter
                .send_replace(Some(session_id.to_string()));
        }

        // Reactive session renders call ensure_session_view() every frame.
        // Only arm telemetry when entering a session or when the SSE filter
        // actually changes, otherwise rendering a session view self-schedules
        // endless telemetry refreshes.
        let authority_gap = self.session_authority_gap(session_id);
        let repair_pending = self
            .sync_runtime
            .pending_session_sync
            .as_deref()
            == Some(session_id)
            || self
                .sync_runtime
                .pending_session_telemetry_sync
                .as_deref()
                == Some(session_id);
        if !had_active_view || filter_changed || (authority_gap && !repair_pending) {
            self.queue_session_scoped_repair(session_id);
            self.reconcile_pending_user_inputs_from_runtime();
        }
    }

    pub(super) fn queue_session_telemetry_refresh(&mut self, session_id: &str) {
        let current = self.current_session_id();
        if current.as_deref() != Some(session_id) {
            return;
        }
        if Self::is_optimistic_local_session_id(session_id) {
            return;
        }
        self.sync_runtime.pending_session_telemetry_sync = Some(session_id.to_string());
        self.sync_runtime.pending_session_telemetry_sync_due_at =
            Some(Instant::now() + Duration::from_millis(SESSION_TELEMETRY_SYNC_DEBOUNCE_MS));
    }

    fn session_projection_present(&self, session_id: &str) -> bool {
        self.context.execution_topology_for(session_id).is_some()
            || !self.context.stage_summaries_for(session_id).is_empty()
            || self.context.session_usage_for(session_id).is_some()
            || self.context.session_usage_books_for(session_id).is_some()
    }

    fn session_messages_cached(&self, session_id: &str) -> bool {
        self.context
            .session
            .read()
            .messages
            .contains_key(session_id)
    }

    pub(super) fn session_authority_gap(&self, session_id: &str) -> bool {
        !self.session_messages_cached(session_id)
            || self.context.session_runtime_for(session_id).is_none()
            || !self.session_projection_present(session_id)
    }

    pub(super) fn queue_session_scoped_repair(&mut self, session_id: &str) {
        if self.current_session_id().as_deref() != Some(session_id) {
            return;
        }
        if self.sync_runtime.pending_session_sync.as_deref() != Some(session_id) {
            self.sync_runtime.pending_session_sync = Some(session_id.to_string());
            self.sync_runtime.pending_session_sync_due_at =
                Some(Instant::now() + Duration::from_millis(SESSION_SYNC_DEBOUNCE_MS));
        }
        self.queue_session_telemetry_refresh(session_id);
    }

    pub(super) fn spawn_queued_session_telemetry_refresh(&mut self) {
        if self.sync_runtime.session_telemetry_sync_inflight {
            return;
        }
        let Some(session_id) = self.sync_runtime.pending_session_telemetry_sync.clone() else {
            return;
        };
        let due = self
            .sync_runtime
            .pending_session_telemetry_sync_due_at
            .unwrap_or_else(Instant::now);
        if Instant::now() < due {
            return;
        }
        let Some(client) = self.context.get_api_client() else {
            self.sync_runtime.pending_session_telemetry_sync = None;
            self.sync_runtime.pending_session_telemetry_sync_due_at = None;
            return;
        };

        self.sync_runtime.session_telemetry_sync_inflight = true;
        self.sync_runtime.pending_session_telemetry_sync = None;
        self.sync_runtime.pending_session_telemetry_sync_due_at = None;

        let context = self.context.clone();
        crate::app::app_impl::support::spawn_background_task(async move {
            let telemetry = match client.get_session_telemetry(&session_id) {
                Ok(telemetry) => Some(Box::new(telemetry)),
                Err(error) => {
                    tracing::debug!(%error, session_id, "failed to fetch session telemetry");
                    None
                }
            };
            let _ = context.emit_custom_event(CustomEvent::SessionTelemetryRefreshFinished {
                session_id,
                telemetry,
            });
        });
    }

    /// Navigate to the attached session of the currently active scheduler stage.
    ///
    /// Uses runtime/stage summary state as the primary authority, falling back
    /// to legacy message metadata only when that state is unavailable.
    pub(super) fn navigate_to_attached_session(&mut self) {
        let session_id = match self.current_session_id() {
            Some(id) => id,
            None => return,
        };
        let attached_id = {
            let session_ctx = self.context.session.read();
            let active_stage_id = self
                .context
                .session_runtime_for(&session_id)
                .and_then(|runtime| runtime.active_stage_id);
            active_stage_id
                .as_deref()
                .and_then(|active_stage_id| {
                    self.context
                        .stage_summaries_for(&session_id)
                        .iter()
                        .find(|stage| stage.stage_id == active_stage_id)
                        .and_then(|stage| stage.primary_attached_session_id.clone())
                })
                .or_else(|| {
                    session_ctx.messages.get(&session_id).and_then(|msgs| {
                        msgs.iter().rev().find_map(|msg| {
                            msg.metadata
                                .as_ref()
                                .and_then(|m| m.get("scheduler_stage_attached_session_id"))
                                .and_then(serde_json::Value::as_str)
                                .map(String::from)
                        })
                    })
                })
        };
        if let Some(attached_id) = attached_id {
            self.navigate_session_with_prompt_cleanup(attached_id.clone());
            self.ensure_session_view(&attached_id);
            let _ = self.sync_session_from_server(&attached_id);
        }
    }

    /// Navigate back to the parent session when available.
    ///
    /// Falls back to the previous route in router history, then to the home
    /// screen when no explicit parent is recorded.
    pub(super) fn navigate_to_parent_session(&mut self) {
        let parent_id = self.current_session_id().and_then(|session_id| {
            let session_ctx = self.context.session.read();
            session_ctx
                .sessions
                .get(&session_id)
                .and_then(|session| session.parent_id.clone())
        });

        if let Some(parent_id) = parent_id {
            self.navigate_session_with_prompt_cleanup(parent_id.clone());
            self.ensure_session_view(&parent_id);
            let _ = self.sync_session_from_server(&parent_id);
            return;
        }

        let previous_route = self.context.go_back();

        match previous_route {
            Some(Route::Session { session_id }) => {
                self.ensure_session_view(&session_id);
                let _ = self.sync_session_from_server(&session_id);
            }
            Some(Route::Home) => {}
            Some(_) => {}
            None => {
                self.navigate_home_with_prompt_cleanup();
            }
        }
    }

    pub(super) fn refresh_attached_sessions(&self) {
        let session_id = match self.current_session_id() {
            Some(id) => id,
            None => return,
        };
        let graph_root_id = self.context.graph_root_session_id(&session_id);
        let session_ctx = self.context.session.read();
        let stage_summaries = self.context.stage_summaries_for(&graph_root_id);
        let children = if !stage_summaries.is_empty() {
            collect_attached_sessions_from_stage_summaries(
                &stage_summaries,
                &session_ctx.sessions,
            )
        } else {
            match session_ctx.messages.get(&graph_root_id) {
                Some(msgs) => collect_attached_sessions(msgs),
                None => return,
            }
        };
        drop(session_ctx);
        self.context.set_attached_sessions(&graph_root_id, children);
    }

    pub(super) fn cache_session_from_api(&self, session: &SessionInfo) {
        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(map_api_session(session));
    }

    pub(super) fn create_optimistic_session(&mut self) -> String {
        let now = Utc::now();
        let session_id = format!("local_session_{}", now.timestamp_millis());
        let session = Session {
            id: session_id.clone(),
            title: "New Session".to_string(),
            created_at: now,
            updated_at: now,
            parent_id: None,
            share: None,
            metadata: None,
        };

        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(session);
        session_ctx.set_current_session_id(session_id.clone());
        session_ctx.messages.entry(session_id.clone()).or_default();
        session_ctx.set_status(&session_id, SessionStatus::Idle);
        session_id
    }

    pub(super) fn remove_optimistic_session(&mut self, session_id: &str) {
        let mut session_ctx = self.context.session.write();
        session_ctx.sessions.remove(session_id);
        session_ctx.messages.remove(session_id);
        session_ctx.session_status.remove(session_id);
        session_ctx.session_diff.remove(session_id);
        session_ctx.todos.remove(session_id);
        session_ctx.revert.remove(session_id);
        if session_ctx.current_session_id.as_deref() == Some(session_id) {
            session_ctx.current_session_id = None;
        }
    }

    pub(super) fn promote_optimistic_session(
        &mut self,
        optimistic_session_id: &str,
        session: &SessionInfo,
    ) {
        let mut session_ctx = self.context.session.write();
        let optimistic_messages = session_ctx
            .messages
            .remove(optimistic_session_id)
            .unwrap_or_default();
        let optimistic_status = session_ctx.session_status.remove(optimistic_session_id);
        let optimistic_diff = session_ctx.session_diff.remove(optimistic_session_id);
        let optimistic_todos = session_ctx.todos.remove(optimistic_session_id);
        let optimistic_revert = session_ctx.revert.remove(optimistic_session_id);
        session_ctx.sessions.remove(optimistic_session_id);

        let real_session_id = session.id.clone();
        session_ctx.upsert_session(map_api_session(session));
        session_ctx.set_current_session_id(real_session_id.clone());
        if !optimistic_messages.is_empty() {
            session_ctx
                .messages
                .insert(real_session_id.clone(), optimistic_messages);
        }
        if let Some(status) = optimistic_status {
            session_ctx
                .session_status
                .insert(real_session_id.clone(), status);
        }
        if let Some(diff) = optimistic_diff {
            session_ctx
                .session_diff
                .insert(real_session_id.clone(), diff);
        }
        if let Some(todos) = optimistic_todos {
            session_ctx.todos.insert(real_session_id.clone(), todos);
        }
        if let Some(revert) = optimistic_revert {
            session_ctx.revert.insert(real_session_id, revert);
        }
    }

    pub(super) fn append_optimistic_user_message(
        &mut self,
        session_id: &str,
        content: &str,
        agent: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> String {
        let now = Utc::now();
        let id = format!("local_user_{}", now.timestamp_millis());
        let message = Message {
            id: id.clone(),
            role: MessageRole::User,
            content: content.to_string(),
            created_at: now,
            agent,
            model,
            mode: variant,
            finish: None,
            error: None,
            completed_at: None,
            cost: 0.0,
            tokens: TokenUsage::default(),
            metadata: None,
            multimodal: None,
            parts: vec![ContextMessagePart::Text {
                text: content.to_string(),
            }],
        };

        let mut session_ctx = self.context.session.write();
        session_ctx
            .messages
            .entry(session_id.to_string())
            .or_default();
        session_ctx.add_message(session_id, message);
        if let Some(session) = session_ctx.sessions.get_mut(session_id) {
            session.updated_at = now;
        }
        id
    }

    pub(super) fn remove_optimistic_message(&mut self, session_id: &str, msg_id: &str) {
        let mut session_ctx = self.context.session.write();
        let rebuilt_index = if let Some(msgs) = session_ctx.messages.get_mut(session_id) {
            msgs.retain(|m| m.id != msg_id);
            let mut index = HashMap::with_capacity(msgs.len());
            for (pos, message) in msgs.iter().enumerate() {
                index.insert(message.id.clone(), pos);
            }
            Some(index)
        } else {
            None
        };
        if let Some(index) = rebuilt_index {
            session_ctx
                .message_index
                .insert(session_id.to_string(), index);
        }
    }

    pub(super) fn sync_session_from_server(&mut self, session_id: &str) -> anyhow::Result<()> {
        self.sync_session_from_server_with_mode(session_id, SessionSyncMode::Full)
    }

    pub(super) fn sync_session_from_server_with_mode(
        &mut self,
        session_id: &str,
        mode: SessionSyncMode,
    ) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };

        let anchor_id = if matches!(mode, SessionSyncMode::Incremental) {
            self.incremental_sync_anchor_id(session_id)
        } else {
            None
        };
        if matches!(mode, SessionSyncMode::Incremental) {
            if let Some(anchor_id) = anchor_id {
                let session = client.get_session(session_id)?;
                let messages =
                    client.get_messages_after(session_id, Some(anchor_id.as_str()), Some(256))?;
                let mapped_messages = messages
                    .iter()
                    .map(map_api_message)
                    .collect::<Vec<Message>>();

                let mut session_ctx = self.context.session.write();
                apply_incremental_session_sync(
                    &mut session_ctx,
                    session_id,
                    &session,
                    mapped_messages,
                );
                // Sync todo and diff on incremental path too
                if let Ok(api_todos) = client.get_session_todos(session_id) {
                    let todos: Vec<_> = api_todos.iter().map(map_api_todo).collect();
                    session_ctx.todos.insert(session_id.to_string(), todos);
                }
                if let Ok(api_diffs) = client.get_session_diff(session_id) {
                    let diffs: Vec<_> = api_diffs.iter().map(map_api_diff).collect();
                    session_ctx
                        .session_diff
                        .insert(session_id.to_string(), diffs);
                }
                drop(session_ctx);

                self.sync_runtime.last_session_sync = Instant::now();
                self.diagnostics.perf.session_sync_incremental = self
                    .diagnostics
                    .perf
                    .session_sync_incremental
                    .saturating_add(1);
                return Ok(());
            }
        }

        let session = client.get_session(session_id)?;
        let messages = client.get_messages(session_id)?;
        let mapped_messages = messages
            .iter()
            .map(map_api_message)
            .collect::<Vec<Message>>();
        let revert = session.revert.as_ref().map(map_api_revert);

        let mut session_ctx = self.context.session.write();
        session_ctx.upsert_session(map_api_session(&session));
        session_ctx.set_messages(session_id, mapped_messages);
        if let Some(revert_info) = revert {
            session_ctx
                .revert
                .insert(session_id.to_string(), revert_info);
        } else {
            session_ctx.revert.remove(session_id);
        }
        if let Ok(status_map) = client.get_session_status() {
            if let Some(status) = status_map.get(session_id) {
                session_ctx.set_status(session_id, map_api_run_status(status));
            }
        }
        // Sync todo items from server
        if let Ok(api_todos) = client.get_session_todos(session_id) {
            let todos: Vec<_> = api_todos.iter().map(map_api_todo).collect();
            session_ctx.todos.insert(session_id.to_string(), todos);
        }
        // Sync modified files / diff entries from server
        if let Ok(api_diffs) = client.get_session_diff(session_id) {
            let diffs: Vec<_> = api_diffs.iter().map(map_api_diff).collect();
            session_ctx
                .session_diff
                .insert(session_id.to_string(), diffs);
        }
        drop(session_ctx);

        self.sync_runtime.last_session_sync = Instant::now();
        self.diagnostics.perf.session_sync_full =
            self.diagnostics.perf.session_sync_full.saturating_add(1);
        Ok(())
    }

    /// Check if a session has scheduler handoff metadata and auto-switch mode.
    pub(super) fn check_scheduler_handoff(&mut self, session_id: &str) {
        if self.consumed_handoffs.contains(session_id) {
            return;
        }

        let (handoff_mode, handoff_command) = {
            let session_ctx = self.context.session.read();
            let session = session_ctx.sessions.get(session_id);
            let metadata = session.and_then(|s| s.metadata.as_ref());
            let mode = metadata
                .and_then(|m| m.get("scheduler_handoff_mode"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let command = metadata
                .and_then(|m| m.get("scheduler_handoff_command"))
                .and_then(|v| v.as_str())
                .map(String::from);
            (mode, command)
        };

        let Some(target_mode) = handoff_mode else {
            return;
        };

        self.consumed_handoffs.insert(session_id.to_string());

        // Switch to the target scheduler profile (e.g. "atlas").
        self.context
            .set_scheduler_profile(Some(target_mode.clone()));

        // Auto-dispatch /start-work by sending it as a prompt.
        let input = handoff_command.unwrap_or_else(|| "/start-work".to_string());
        self.dispatch_prompt_to_session(super::prompt_flow::PromptDispatchRequest {
            session_id,
            display_text: input.clone(),
            input,
            parts: None,
            agent: None,
            scheduler_profile: Some(target_mode),
            display_mode: Some("atlas".to_string()),
            model: self.selected_model_for_prompt(),
            variant: self.context.current_model_variant(),
            idempotency_key: Some(format!("tui_{}", uuid::Uuid::new_v4().simple())),
        });
    }

    pub(super) fn incremental_sync_anchor_id(&self, session_id: &str) -> Option<String> {
        let session_ctx = self.context.session.read();
        if !session_ctx.sessions.contains_key(session_id) {
            return None;
        }
        let messages = session_ctx.messages.get(session_id)?;
        if messages.len() >= 2 {
            messages.get(messages.len().saturating_sub(2))
        } else {
            messages.last()
        }
        .map(|message| message.id.clone())
    }

    pub(super) fn set_session_status(&mut self, session_id: &str, status: SessionStatus) {
        let mut session_ctx = self.context.session.write();
        session_ctx.set_status(session_id, status);
    }

    /// Apply a server-pushed session run status change: set the local
    /// SessionStatus, queue a telemetry refresh (unless this is a transient
    /// reconnecting state), and sync the prompt spinner.
    pub(super) fn apply_session_run_status_change(
        &mut self,
        session_id: &str,
        status: SessionStatus,
    ) {
        let skip_telemetry = matches!(status, SessionStatus::Reconnecting);
        self.set_session_status(session_id, status);
        if !skip_telemetry {
            self.queue_session_telemetry_refresh(session_id);
        }
        self.sync_prompt_spinner_state();
    }

    /// Route-gating: return true when a session-scoped event should surface
    /// on the current view. Events for non-active sessions surface when the
    /// user is on home/multi-session view; events for the active session
    /// always surface.
    pub(super) fn should_surface_event_for_session(&self, session_id: &str) -> bool {
        match self.context.current_route() {
            crate::router::Route::Session {
                session_id: active_session_id,
            } => active_session_id == session_id,
            _ => true,
        }
    }

    /// Apply an incoming OutputBlock from the server event stream.
    /// Handles incremental block insertion, scheduler-stage metadata, attached
    /// session refresh, and active status-dialog refresh.
    pub(super) fn apply_output_block_change(
        &mut self,
        session_id: &str,
        id: &Option<String>,
        payload: &serde_json::Value,
        live_identity: &Option<agendao_types::LiveMessagePartIdentity>,
    ) {
        let current_session = self.current_session_id();
        let is_active_session = current_session.as_deref() == Some(session_id);
        let current_is_parent_of_target = current_session.as_deref().is_some_and(|active| {
            let session_ctx = self.context.session.read();
            session_ctx
                .sessions
                .get(session_id)
                .and_then(|session| session.parent_id.as_deref())
                == Some(active)
        });

        {
            let mut session_ctx = self.context.session.write();
            session_ctx.apply_output_block_incremental(
                session_id,
                id.as_deref(),
                payload,
                live_identity.as_ref(),
            );
        }
        if payload.get("kind").and_then(|value| value.as_str()) == Some("scheduler_stage") {
            if let Ok(block) = serde_json::from_value::<agendao_output_blocks::SchedulerStageBlock>(
                payload.clone(),
            ) {
                self.context
                    .apply_scheduler_stage_summary(session_id, &block);
            }
        }

        if let crate::router::Route::Session { session_id: active } = self.context.current_route() {
            if active == *session_id {
                if payload.get("kind").and_then(|value| value.as_str()) == Some("scheduler_stage") {
                    self.refresh_attached_sessions();
                }
                self.event_caused_change = true;
            }
        }

        if current_is_parent_of_target {
            self.refresh_attached_sessions();
            self.event_caused_change = true;
        }

        if is_active_session && self.status_dialog.is_open() {
            self.refresh_active_status_dialog();
        }
    }
}
