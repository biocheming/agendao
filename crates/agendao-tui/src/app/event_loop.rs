use super::*;
use crate::components::SessionLeftMouseDownOutcome;

impl App {
    fn handle_session_interrupt_request(&mut self) -> anyhow::Result<()> {
        let current_sid = self.current_session_id();
        if let Some(ref pending) = self.pending_shell_dispatch {
            if current_sid.as_deref() == Some(&pending.session_id) {
                let session_id = pending.session_id.clone();
                let msg_id = pending.optimistic_message_id.clone();
                if let Some(client) = self.context.get_api_client() {
                    let _ = client.abort_session(&session_id);
                }
                self.settle_shell_dispatch(
                    &session_id,
                    Some(&msg_id),
                    self::prompt_flow::ShellDispatchOutcome::Cancelled,
                );
                if self.prompt.is_shell_mode() {
                    self.prompt.exit_shell_mode();
                }
                self.prompt.clear_interrupt_confirmation();
                return Ok(());
            }
        }
        if self.prompt.is_shell_mode() {
            self.prompt.exit_shell_mode();
            self.prompt.clear_interrupt_confirmation();
            return Ok(());
        }
        if let Route::Session { session_id } = self.context.current_route() {
            let status = {
                let session_ctx = self.context.session.read();
                session_ctx.status(&session_id).clone()
            };
            if !matches!(status, SessionStatus::Idle) {
                if !self.prompt.register_interrupt_keypress() {
                    return Ok(());
                }
                if let Some(client) = self.context.get_api_client() {
                    let _ = client.abort_session(&session_id);
                }
                self.prompt.clear_interrupt_confirmation();
                self.set_session_status(&session_id, SessionStatus::Idle);
                self.sync_prompt_spinner_state();
                return Ok(());
            }
        }
        self.prompt.clear_interrupt_confirmation();
        Ok(())
    }

    fn handle_mouse_down(
        &mut self,
        button: crossterm::event::MouseButton,
        col: u16,
        row: u16,
        mouse_event: &crossterm::event::MouseEvent,
    ) -> anyhow::Result<bool> {
        if button == crossterm::event::MouseButton::Right {
            if self.selection.is_active() {
                self.copy_selection();
            }
            self.suppress_current_terminal_event_for_reratui();
            return Ok(true);
        }

        if self.handle_permission_prompt_mouse(col, row) {
            self.suppress_current_terminal_event_for_reratui();
            return Ok(true);
        }

        if self.handle_question_prompt_mouse(col, row) {
            self.suppress_current_terminal_event_for_reratui();
            return Ok(true);
        }

        if self.handle_status_dialog_mouse(button, col, row) {
            self.suppress_current_terminal_event_for_reratui();
            return Ok(true);
        }

        if self.handle_dialog_mouse(mouse_event)? {
            self.suppress_current_terminal_event_for_reratui();
            return Ok(true);
        }

        if button == crossterm::event::MouseButton::Left {
            if let Route::Session { .. } = self.context.current_route() {
                if let Some(sv) = self.context.session_view_handle() {
                    match sv.left_mouse_down_outcome(col, row) {
                        SessionLeftMouseDownOutcome::Consumed => {
                            self.selection.clear();
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(true);
                        }
                        SessionLeftMouseDownOutcome::BeginSelection { area } => {
                            self.selection.start_scoped(row, col, Some(area));
                        }
                        SessionLeftMouseDownOutcome::ClearSelection => {
                            self.selection.clear();
                        }
                    }
                }
            } else {
                self.selection.start(row, col);
            }
        }

        Ok(false)
    }

    pub(super) fn handle_event(&mut self, event: &Event) -> anyhow::Result<()> {
        self.event_caused_change = true;

        match event {
            Event::Key(key) => {
                if !is_primary_key_event(*key) {
                    return Ok(());
                }
                let key = normalize_key_event(*key);

                if self.handle_permission_prompt_key(key) {
                    self.suppress_current_terminal_event_for_reratui();
                    return Ok(());
                }

                if self.handle_question_prompt_key(key) {
                    self.suppress_current_terminal_event_for_reratui();
                    return Ok(());
                }

                if self.handle_dialog_key(key)? {
                    self.suppress_current_terminal_event_for_reratui();
                    return Ok(());
                }

                if self.leader_state.active {
                    if self.leader_state.check_timeout() {
                    } else {
                        let action = match key.code {
                            KeyCode::Char('n') => Some(UiActionId::NewSession),
                            KeyCode::Char('l') => Some(UiActionId::OpenSessionList),
                            KeyCode::Char('m') => Some(UiActionId::OpenModelList),
                            KeyCode::Char('a') => Some(UiActionId::OpenAgentList),
                            KeyCode::Char('t') => Some(UiActionId::OpenThemeList),
                            KeyCode::Char('b') => Some(UiActionId::ToggleSidebar),
                            KeyCode::Char('s') => Some(UiActionId::ViewStatus),
                            KeyCode::Char('q') => Some(UiActionId::Exit),
                            KeyCode::Char('u') => Some(UiActionId::Undo),
                            KeyCode::Char('r') => Some(UiActionId::Redo),
                            _ => None,
                        };
                        self.leader_state.reset();
                        if let Some(action) = action {
                            self.execute_ui_action(action)?;
                        }
                        self.suppress_current_terminal_event_for_reratui();
                        return Ok(());
                    }
                }

                if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
                    self.leader_state.start(KeyCode::Char('x'));
                    self.suppress_current_terminal_event_for_reratui();
                    return Ok(());
                }

                if (key.code == KeyCode::Char('C') || key.code == KeyCode::Char('c'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    if !self.selection.is_active() && self.can_render_reactive_route() {
                        return Ok(());
                    }
                    self.copy_selection();
                    self.suppress_current_terminal_event_for_reratui();
                    return Ok(());
                }

                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    if self.selection.is_active() {
                        self.copy_selection();
                        self.suppress_current_terminal_event_for_reratui();
                        return Ok(());
                    }
                    if self.can_render_reactive_route() {
                        return Ok(());
                    }
                    self.execute_ui_action(UiActionId::Exit)?;
                    return Ok(());
                }

                if key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL {
                    if self.can_render_reactive_route() {
                        return Ok(());
                    }
                    self.request_abort_execution();
                    return Ok(());
                }

                if key.code == KeyCode::Esc {
                    if self.selection.is_active() {
                        self.selection.clear();
                        self.suppress_current_terminal_event_for_reratui();
                        return Ok(());
                    }
                }

                if self.matches_keybind("session_interrupt", key) {
                    if self.can_render_reactive_route() {
                        return Ok(());
                    }
                    self.handle_session_interrupt_request()?;
                    return Ok(());
                }

                if self.matches_keybind("input_paste", key)
                    && !self.can_render_reactive_route()
                {
                    self.paste_clipboard_to_prompt();
                    return Ok(());
                }
                if self.matches_keybind("input_copy", key)
                    && !self.can_render_reactive_route()
                {
                    self.copy_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("input_cut", key)
                    && !self.can_render_reactive_route()
                {
                    self.cut_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("command_palette", key)
                    && !self.can_render_reactive_route()
                {
                    self.sync_command_palette_labels();
                    self.open_command_palette_dialog();
                    return Ok(());
                }
                if self.matches_keybind("model_cycle", key)
                    && !self.can_render_reactive_route()
                {
                    self.refresh_model_dialog();
                    self.open_model_select_dialog();
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle", key)
                    && !self.can_render_reactive_route()
                {
                    self.cycle_agent(1);
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle_reverse", key)
                    && !self.can_render_reactive_route()
                {
                    self.cycle_agent(-1);
                    return Ok(());
                }
                if self.matches_keybind("variant_cycle", key)
                    && !self.can_render_reactive_route()
                {
                    self.cycle_model_variant();
                    return Ok(());
                }
                if self.matches_keybind("display_thinking", key)
                    && !self.can_render_reactive_route()
                {
                    self.context.toggle_thinking();
                    return Ok(());
                }
                if self.matches_keybind("tool_details", key)
                    && !self.can_render_reactive_route()
                {
                    self.context.toggle_tool_details();
                    return Ok(());
                }
                if self.matches_keybind("input_clear", key)
                    && !self.can_render_reactive_route()
                {
                    self.discard_prompt_draft();
                    return Ok(());
                }
                if self.matches_keybind("help_toggle", key)
                    && !self.can_render_reactive_route()
                {
                    self.open_help_dialog();
                    return Ok(());
                }

                let route = self.context.current_route();
                match route {
                    Route::Home | Route::Session { .. } => {
                        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                            if self.can_render_reactive_route() {
                                return Ok(());
                            }
                            self.submit_prompt()?;
                        }
                    }
                    _ => {}
                }
            }
            Event::Resize(width, height) => {
                self.viewport_area = Rect::new(0, 0, *width, *height);
            }
            Event::Mouse(mouse_event) => {
                use crossterm::event::MouseEventKind;
                match mouse_event.kind {
                    MouseEventKind::Down(button) => {
                        let col = mouse_event.column;
                        let row = mouse_event.row;
                        if self.handle_mouse_down(button, col, row, mouse_event)? {
                            return Ok(());
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                    }
                    MouseEventKind::Drag(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.update(mouse_event.row, mouse_event.column);
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                        if let Route::Session { .. } = self.context.current_route() {
                            if let Some(sv) = self.context.session_view_handle() {
                                if sv.consumes_left_drag(mouse_event.column, mouse_event.row) {
                                    self.suppress_current_terminal_event_for_reratui();
                                    return Ok(());
                                }
                            }
                        }
                        let col = mouse_event.column;
                        let row = mouse_event.row;
                        self.selection.update(row, col);
                    }
                    MouseEventKind::Moved => {
                        if self.status_dialog.is_open() {
                            self.event_caused_change = false;
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                        if self.handle_dialog_mouse(mouse_event)? {
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                        self.event_caused_change = false;
                    }
                    MouseEventKind::Up(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.finalize();
                            self.suppress_current_terminal_event_for_reratui();
                            return Ok(());
                        }
                        if let Route::Session { .. } = self.context.current_route() {
                            if let Some(sv) = self.context.session_view_handle() {
                                if sv.consumes_left_mouse_up() {
                                    self.suppress_current_terminal_event_for_reratui();
                                    return Ok(());
                                }
                            }
                        }
                        self.selection.finalize();
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                // Terminal paste is now reserved for legacy dialog text fields.
                // Reactive prompt paste flows through PromptComponent -> PromptEdited.
                if !text.is_empty() {
                    if self.provider_dialog.is_open() && self.provider_dialog.accepts_text_input() {
                        for c in text.chars() {
                            self.provider_dialog.push_char(c);
                        }
                    }
                }
            }
            Event::Custom(event) => match event.as_ref() {
                CustomEvent::PromptDispatchHomeFinished {
                    optimistic_session_id,
                    optimistic_message_id,
                    created_session,
                    response,
                    error,
                } => {
                    if let Some(session) = created_session.as_deref() {
                        self.promote_optimistic_session(optimistic_session_id, session);

                        if let Route::Session { session_id: active } = self.context.current_route()
                        {
                            if active == *optimistic_session_id {
                                self.context.navigate(Route::Session {
                                    session_id: session.id.clone(),
                                });
                            }
                        }
                        self.ensure_session_view(&session.id);

                        if let Some(err) = error {
                            self.remove_optimistic_message(&session.id, optimistic_message_id);
                            self.set_session_status(&session.id, SessionStatus::Idle);
                            self.sync_prompt_spinner_state();
                            self.alert_dialog
                                .set_message(&format!("Failed to send prompt:\n{}", err));
                            self.open_alert_dialog();
                        } else {
                            match response.as_ref().map(|response| response.status.as_str()) {
                                Some("awaiting_user") => {
                                    self.set_session_status(&session.id, SessionStatus::Idle);
                                    self.prompt.set_spinner_active(false);
                                    self.queue_session_telemetry_refresh(&session.id);
                                    if !self.local_direct {
                                        self.sync_question_requests();
                                    }
                                }
                                Some("queued") => {
                                    self.set_session_status(&session.id, SessionStatus::Running);
                                    self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                                    self.prompt.set_spinner_active(true);
                                    self.queue_session_telemetry_refresh(&session.id);
                                }
                                _ => {
                                    self.set_session_status(&session.id, SessionStatus::Running);
                                    self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                                    self.prompt.set_spinner_active(true);
                                }
                            }
                        }
                    } else {
                        self.remove_optimistic_session(optimistic_session_id);
                        if let Route::Session { session_id: active } = self.context.current_route()
                        {
                            if active == *optimistic_session_id {
                                self.context.navigate_home();
                            }
                        }
                        self.prompt.set_spinner_active(false);
                        if let Some(err) = error {
                            self.alert_dialog
                                .set_message(&format!("Failed to create session:\n{}", err));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::PromptDispatchSessionFinished {
                    session_id,
                    optimistic_message_id,
                    response,
                    error,
                } => {
                    self.settle_prompt_dispatch(
                        session_id,
                        optimistic_message_id,
                        response.as_ref(),
                        error.as_deref(),
                    );
                    self.event_caused_change = true;
                }
                CustomEvent::ShellDispatchFinished {
                    optimistic_session_id,
                    session_id,
                    optimistic_message_id,
                    created_session,
                    cancelled,
                    error,
                } => {
                    let already_cancelled_locally = self
                        .pending_shell_dispatch
                        .as_ref()
                        .map_or(true, |pending| {
                            pending.optimistic_message_id != *optimistic_message_id
                        });
                    if already_cancelled_locally {
                        self.event_caused_change = true;
                    } else if session_id.is_empty() {
                        self.remove_optimistic_session(optimistic_session_id);
                        if self
                            .current_session_id()
                            .as_deref()
                            .map_or(false, |id| id == optimistic_session_id.as_str())
                        {
                            self.navigate_home_with_prompt_cleanup();
                        }
                        self.pending_shell_dispatch = None;
                        self.sync_prompt_spinner_state();
                        if let Some(err) = error {
                            self.alert_dialog
                                .set_message(&format!("Shell command failed:\n{}", err));
                            self.open_alert_dialog();
                        }
                        self.event_caused_change = true;
                    } else {
                        if *optimistic_session_id != *session_id {
                            if let Some(session) = created_session {
                                self.promote_optimistic_session(&optimistic_session_id, &session);
                            }
                            self.handle_prompt_route_change();
                            self.context.navigate(Route::Session {
                                session_id: session_id.to_string(),
                            });
                            self.ensure_session_view(&session_id);
                        }

                        let outcome = if *cancelled {
                            self::prompt_flow::ShellDispatchOutcome::Cancelled
                        } else if error.is_some() {
                            self::prompt_flow::ShellDispatchOutcome::Failed
                        } else {
                            self::prompt_flow::ShellDispatchOutcome::Sent
                        };
                        self.settle_shell_dispatch(
                            &session_id,
                            Some(&optimistic_message_id),
                            outcome,
                        );
                        if let Some(err) = error {
                            self.alert_dialog
                                .set_message(&format!("Shell command failed:\n{}", err));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::PermissionReplyFinished {
                    permission_id,
                    outcome,
                } => {
                    self.permission_runtime.last_submit_completed_at =
                        Some(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                    match outcome {
                        crate::event::PermissionReplyOutcome::Succeeded => {
                            self.permission_runtime.last_submit_error = None;
                            self.clear_permission_request(&permission_id);
                            self.context
                                .set_pending_permissions(self.permission_prompt.pending_count());
                            self.toast.show(
                                crate::components::ToastVariant::Success,
                                "Permission reply sent",
                                2000,
                            );
                        }
                        crate::event::PermissionReplyOutcome::Failed { message } => {
                            self.permission_runtime.last_submit_error = Some(message.clone());
                            self.permission_prompt
                                .mark_submit_failed(permission_id, message.clone());
                            self.alert_dialog.set_message(&format!(
                                "Failed to submit permission response:\n{}",
                                message
                            ));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::SessionDeleteFinished {
                    session_id,
                    outcome,
                } => {
                    match outcome {
                        SessionDeleteOutcome::Succeeded => {
                            if self.current_session_id().as_deref() == Some(session_id.as_str()) {
                                self.context.navigate_home();
                            }
                            self.refresh_session_list_dialog();
                            self.toast.show(
                                crate::components::ToastVariant::Success,
                                &format!("Session deleted: {}", session_id),
                                2200,
                            );
                        }
                        SessionDeleteOutcome::Failed { message } => {
                            self.alert_dialog.set_message(&format!(
                                "Failed to delete session `{}`:\n{}",
                                session_id, message
                            ));
                            self.open_alert_dialog();
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::SessionTelemetryRefreshFinished {
                    session_id,
                    telemetry,
                } => {
                    self.sync_runtime.session_telemetry_sync_inflight = false;

                    if self.current_session_id().as_deref() == Some(session_id.as_str()) {
                        if let Some(telemetry) = telemetry.as_deref() {
                            self.context
                                .apply_session_telemetry_snapshot(telemetry.clone());
                            self.reconcile_pending_user_inputs_from_runtime();
                            self.refresh_attached_sessions();
                            if self.status_dialog.is_open() {
                                self.refresh_active_status_dialog();
                            }
                            self.event_caused_change = true;
                        }
                    }
                }
                CustomEvent::SessionNavigationIntent { kind } => {
                    match kind {
                        crate::event::SessionNavigationIntentKind::Parent => {
                            self.navigate_to_parent_session();
                        }
                        crate::event::SessionNavigationIntentKind::Attached => {
                            self.navigate_to_attached_session();
                        }
                        crate::event::SessionNavigationIntentKind::Session(session_id) => {
                            self.navigate_session_with_prompt_cleanup(session_id.clone());
                            self.ensure_session_view(session_id);
                            let _ = self.sync_session_from_server(session_id);
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::SessionSidebarIntent { kind } => {
                    match kind {
                        crate::event::SessionSidebarIntentKind::KillSelectedProcess => {
                            if let Some(sv) = self
                                .context
                                .session_view_handle()
                                .filter(|sv| sv.sidebar_process_focus())
                            {
                                let procs = self.context.processes.read().clone();
                                if let Some(proc) = procs.get(sv.sidebar_process_selected()) {
                                    let _ = agendao_orchestrator::global_lifecycle()
                                        .kill_process(proc.pid);
                                    *self.context.processes.write() =
                                        agendao_core::process_registry::global_registry().list();
                                    sv.clamp_sidebar_process_selection(
                                        self.context.processes.read().len(),
                                    );
                                }
                            }
                        }
                    }
                    self.event_caused_change = true;
                }
                CustomEvent::SlashPopupIntent { kind } => {
                    if self.slash_popup.is_open() {
                        match kind {
                            crate::event::SlashPopupIntentKind::Close => {
                                self.close_slash_popup_dialog();
                            }
                            crate::event::SlashPopupIntentKind::MoveUp => {
                                self.slash_popup.move_up();
                            }
                            crate::event::SlashPopupIntentKind::MoveDown => {
                                self.slash_popup.move_down();
                            }
                            crate::event::SlashPopupIntentKind::SelectCurrent => {
                                self.slash_popup.select_current();
                                if let Some(action) = self.slash_popup.take_action() {
                                    self.execute_ui_action(action)?;
                                }
                            }
                        }
                        self.event_caused_change = true;
                    }
                }
                CustomEvent::SessionInterruptRequested => {
                    self.handle_session_interrupt_request()?;
                    self.event_caused_change = true;
                }
                CustomEvent::PromptEdited { prompt } => {
                    self.prompt = (*prompt.clone()).clone();
                    self.sync_slash_popup_from_prompt();
                    self.event_caused_change = true;
                }
                CustomEvent::PromptSubmitRequested { prompt } => {
                    self.prompt = (*prompt.clone()).clone();
                    self.close_slash_popup_dialog();
                    self.submit_prompt()?;
                    self.event_caused_change = true;
                }
                CustomEvent::PromptPasteText { text } => {
                    self.prompt.insert_text(text);
                    self.sync_slash_popup_from_prompt();
                    self.event_caused_change = true;
                }
                CustomEvent::UiActionRequested { action } => {
                    self.execute_ui_action(*action)?;
                    self.event_caused_change = true;
                }
                CustomEvent::SessionUpdated { session_id, source } => {
                    if source.as_deref() == Some("stream.reconnected") {
                        self.queue_session_scoped_repair(session_id);
                    }
                    self.handle_session_updated_reconcile(session_id, source.as_deref());
                }
                CustomEvent::SessionStatusReconnecting { session_id } => {
                    self.apply_session_run_status_change(session_id, SessionStatus::Reconnecting);
                }
                CustomEvent::FrontendEvent(event) => self.apply_frontend_event(event),
                _ => {}
            },
            Event::Tick => {
                let now = Instant::now();
                let delta_ms = now
                    .saturating_duration_since(self.sync_runtime.last_tick_at)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64;
                self.sync_runtime.last_tick_at = now;
                let mut tick_changed = false;
                tick_changed |= self.toast.tick(delta_ms);
                tick_changed |= self.prompt.tick_spinner(delta_ms);
                tick_changed |= self.sync_prompt_spinner_state();

                if self.sync_runtime.pending_initial_submit
                    && !self.prompt.get_input().trim().is_empty()
                {
                    self.sync_runtime.pending_initial_submit = false;
                    self.submit_prompt()?;
                    tick_changed = true;
                }

                self.spawn_queued_session_telemetry_refresh();

                let route = self.context.current_route();
                if let Route::Session { session_id } = &route {
                    let should_sync_pending = self.sync_runtime.pending_session_sync.as_deref()
                        == Some(session_id.as_str())
                        && self
                            .sync_runtime
                            .pending_session_sync_due_at
                            .map(|due| Instant::now() >= due)
                            .unwrap_or(false);
                    if should_sync_pending {
                        let sync_result = self
                            .sync_session_from_server_with_mode(
                                session_id,
                                SessionSyncMode::Incremental,
                            )
                            .or_else(|_| {
                                self.sync_session_from_server_with_mode(
                                    session_id,
                                    SessionSyncMode::Full,
                                )
                            });
                        if sync_result.is_ok() {
                            tick_changed = true;
                            self.check_scheduler_handoff(session_id);
                            self.refresh_attached_sessions();
                            if self.status_dialog.is_open() {
                                self.refresh_active_status_dialog();
                            }
                        }
                        self.sync_runtime.pending_session_sync = None;
                        self.sync_runtime.pending_session_sync_due_at = None;
                    }
                }
                if matches!(route, Route::Session { .. }) {
                    if self
                        .sync_runtime
                        .pending_question_sync_due_at
                        .map(|due| Instant::now() >= due)
                        .unwrap_or(false)
                    {
                        tick_changed |= self.sync_question_requests();
                        self.sync_runtime.last_question_sync = Instant::now();
                        self.sync_runtime.pending_question_sync_due_at = None;
                    }
                    if self
                        .sync_runtime
                        .pending_permission_sync_due_at
                        .map(|due| Instant::now() >= due)
                        .unwrap_or(false)
                    {
                        tick_changed |= self.sync_permission_requests();
                        self.sync_runtime.last_permission_sync = Instant::now();
                        self.sync_runtime.pending_permission_sync_due_at = None;
                    }
                } else {
                    self.sync_runtime.pending_question_sync_due_at = None;
                    self.sync_runtime.pending_permission_sync_due_at = None;
                }
                if matches!(route, Route::Session { .. })
                    && self.session_sidebar_visible()
                    && !self.local_direct_idle_session()
                    && self.sync_runtime.last_process_refresh.elapsed() >= Duration::from_secs(2)
                    && self.sync_runtime.pending_process_refresh_due_at.is_none()
                {
                    self.queue_process_refresh();
                }
                if self
                    .sync_runtime
                    .pending_process_refresh_due_at
                    .map(|due| Instant::now() >= due)
                    .unwrap_or(false)
                {
                    let should_refresh_processes =
                        matches!(route, Route::Session { .. }) && self.session_sidebar_visible();
                    if should_refresh_processes {
                        agendao_core::process_registry::global_registry().refresh_stats();
                        *self.context.processes.write() =
                            agendao_core::process_registry::global_registry().list();
                        tick_changed = true;
                    }
                    self.sync_runtime.last_process_refresh = Instant::now();
                    self.sync_runtime.pending_process_refresh_due_at = None;
                }
                tick_changed |= self.sync_ui_bridge_health();
                self.maybe_log_perf_snapshot();
                self.event_caused_change = tick_changed;
            }
            _ => {}
        }

        Ok(())
    }
}
