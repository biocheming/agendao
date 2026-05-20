use super::*;

impl App {
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
        self.context.ensure_session_view_handle(session_id);

        // Update the SSE session filter so the listener reconnects
        // with server-side filtering for this session.
        if let Ok(mut filter) = self.sse_session_filter.lock() {
            *filter = Some(session_id.to_string());
        }

        // Fetch the initial telemetry snapshot for the new session.
        self.refresh_session_telemetry(session_id);
    }

    /// Refresh the cached telemetry snapshot when the server notifies us of a topology change.
    pub(super) fn handle_topology_changed(&mut self, session_id: &str) {
        let current = self.current_session_id();
        if current.as_deref() != Some(session_id) {
            return;
        }
        self.refresh_session_telemetry(session_id);
    }

    /// Refresh the aggregated session telemetry snapshot from the server.
    ///
    /// Called on first load and runtime-related SSE events so the TUI keeps
    /// one authority-backed snapshot instead of separately fetching runtime
    /// and topology.
    pub(super) fn refresh_session_telemetry(&mut self, session_id: &str) {
        let current = self.current_session_id();
        if current.as_deref() != Some(session_id) {
            return;
        }
        let client = self.context.api_client.read();
        let Some(client) = client.as_ref() else {
            return;
        };
        match client.get_session_telemetry(session_id) {
            Ok(telemetry) => {
                self.context.apply_session_telemetry_snapshot(telemetry);
            }
            Err(err) => {
                tracing::debug!(%err, session_id, "failed to fetch session telemetry");
            }
        }
    }

    /// Navigate to the attached session of the currently active scheduler stage.
    ///
    /// Scans the current session's messages (most recent first) for one that
    /// carries `scheduler_stage_attached_session_id` metadata and navigates to it.
    pub(super) fn navigate_to_attached_session(&mut self) {
        let session_id = match self.current_session_id() {
            Some(id) => id,
            None => return,
        };
        let attached_id = {
            let session_ctx = self.context.session.read();
            session_ctx.messages.get(&session_id).and_then(|msgs| {
                msgs.iter().rev().find_map(|msg| {
                    msg.metadata
                        .as_ref()
                        .and_then(|m| m.get("scheduler_stage_attached_session_id"))
                        .and_then(serde_json::Value::as_str)
                        .map(String::from)
                })
            })
        };
        if let Some(attached_id) = attached_id {
            self.context.navigate_session(attached_id.clone());
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
            self.context.navigate_session(parent_id.clone());
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
                self.context.navigate_home();
            }
        }
    }

    pub(super) fn refresh_attached_sessions(&self) {
        let session_id = match self.current_session_id() {
            Some(id) => id,
            None => return,
        };
        let session_ctx = self.context.session.read();
        let graph_root_id = session_ctx
            .sessions
            .get(&session_id)
            .and_then(|session| session.parent_id.clone())
            .unwrap_or(session_id);
        let children = match session_ctx.messages.get(&graph_root_id) {
            Some(msgs) => collect_attached_sessions(msgs),
            None => return,
        };
        drop(session_ctx);
        self.context.set_attached_sessions(children);
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
        self.sync_runtime.last_full_session_sync = self.sync_runtime.last_session_sync;
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
}
