use super::*;

impl App {
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
            return Ok(true);
        }

        if self.handle_permission_prompt_mouse(col, row) {
            return Ok(true);
        }

        if self.handle_question_prompt_mouse(col, row) {
            return Ok(true);
        }

        if self.handle_status_dialog_mouse(button, col, row) {
            return Ok(true);
        }

        if self.handle_dialog_mouse(mouse_event)? {
            return Ok(true);
        }

        if button == crossterm::event::MouseButton::Left {
            if let Route::Session { .. } = self.context.current_route() {
                if let Some(sv) = self.context.session_view_handle() {
                    if sv.handle_sidebar_click(&self.context, col, row) {
                        if let Some(session_id) = sv.take_pending_navigate_session() {
                            self.navigate_session_with_prompt_cleanup(session_id.clone());
                            self.ensure_session_view(&session_id);
                            let _ = self.sync_session_from_server(&session_id);
                        }
                        if let Some(cs_idx) = sv.take_pending_navigate_attached() {
                            let sessions = self.context.attached_sessions();
                            if let Some(child) = sessions.get(cs_idx) {
                                let attached_id = child.session_id.clone();
                                self.navigate_session_with_prompt_cleanup(attached_id.clone());
                                self.ensure_session_view(&attached_id);
                                let _ = self.sync_session_from_server(&attached_id);
                            }
                        }
                        if sv.take_pending_navigate_parent() {
                            self.navigate_to_parent_session();
                        }
                        return Ok(true);
                    }
                    if sv.is_point_in_sidebar(col, row) {
                        return Ok(true);
                    }
                    if sv.handle_scrollbar_click(col, row) {
                        return Ok(true);
                    }
                    if sv.handle_click(col, row) {
                        return Ok(true);
                    }
                }
            }
            if let Route::Session { .. } = self.context.current_route() {
                if let Some(sv) = self.context.session_view_handle() {
                    if let Some(area) = sv.selection_area() {
                        if col >= area.x
                            && col < area.x.saturating_add(area.width)
                            && row >= area.y
                            && row < area.y.saturating_add(area.height)
                        {
                            self.selection.start_scoped(row, col, Some(area));
                        } else {
                            self.selection.clear();
                        }
                    } else {
                        self.selection.clear();
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
                    return Ok(());
                }

                if self.handle_question_prompt_key(key) {
                    return Ok(());
                }

                if self.handle_dialog_key(key)? {
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
                        return Ok(());
                    }
                }

                if key.code == KeyCode::Char('x') && key.modifiers == KeyModifiers::CONTROL {
                    self.leader_state.start(KeyCode::Char('x'));
                    return Ok(());
                }

                if (key.code == KeyCode::Char('C') || key.code == KeyCode::Char('c'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    self.copy_selection();
                    return Ok(());
                }

                if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
                    if self.selection.is_active() {
                        self.copy_selection();
                        return Ok(());
                    }
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                if key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL {
                    tracing::info!("Ctrl+K pressed");
                    if let Some(session_id) = self.current_session_id() {
                        let active_tool_calls = self.context.get_active_tool_calls();
                        let tool_call_count = active_tool_calls.len();
                        tracing::info!(
                            "Active session: {}, tool call count: {}",
                            session_id,
                            tool_call_count
                        );

                        if tool_call_count > 1 {
                            let items: Vec<ToolCallItem> = active_tool_calls
                                .values()
                                .map(|info| ToolCallItem {
                                    id: info.id.clone(),
                                    tool_name: info.tool_name.clone(),
                                })
                                .collect();
                            self.open_tool_call_cancel_dialog_modal(items);
                        } else if tool_call_count == 1 {
                            if let Some(api) = self.context.get_api_client() {
                                let tool_call_id = active_tool_calls.keys().next().unwrap().clone();
                                if let Err(e) = api.cancel_tool_call(&session_id, &tool_call_id) {
                                    self.toast.show(
                                        ToastVariant::Error,
                                        &format!("Failed to cancel tool: {}", e),
                                        3000,
                                    );
                                } else {
                                    self.toast.show(
                                        ToastVariant::Info,
                                        "Tool cancellation requested",
                                        3000,
                                    );
                                }
                            }
                        } else if let Some(api) = self.context.get_api_client() {
                            match api.abort_session(&session_id) {
                                Err(e) => {
                                    self.toast.show(
                                        ToastVariant::Error,
                                        &format!("Failed to cancel session: {}", e),
                                        3000,
                                    );
                                }
                                Ok(value) => {
                                    let message = value
                                        .get("target")
                                        .and_then(|value| value.as_str())
                                        .map(|target| match target {
                                            "stage" => {
                                                let stage = value
                                                    .get("stage")
                                                    .and_then(|value| value.as_str())
                                                    .unwrap_or("current stage");
                                                format!("Stage cancellation requested: {}", stage)
                                            }
                                            _ => "Run cancellation requested".to_string(),
                                        })
                                        .unwrap_or_else(|| {
                                            "Run cancellation requested".to_string()
                                        });
                                    self.toast.show(ToastVariant::Info, &message, 3000);
                                }
                            }
                        }
                    }
                    return Ok(());
                }

                if key.code == KeyCode::Esc {
                    if let Some(sv) = self.context.session_view_handle() {
                        if sv.clear_sidebar_focus() {
                            return Ok(());
                        }
                    }
                    if self.selection.is_active() {
                        self.selection.clear();
                        return Ok(());
                    }
                }

                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_process_focus())
                {
                    let proc_count = self.context.processes.read().len();
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_process_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            sv.move_sidebar_process_selection_down(proc_count);
                            return Ok(());
                        }
                        KeyCode::Char('d') | KeyCode::Delete => {
                            let procs = self.context.processes.read().clone();
                            if let Some(proc) = procs.get(sv.sidebar_process_selected()) {
                                let _ =
                                    agendao_orchestrator::global_lifecycle().kill_process(proc.pid);
                                *self.context.processes.write() =
                                    agendao_core::process_registry::global_registry().list();
                                sv.clamp_sidebar_process_selection(
                                    self.context.processes.read().len(),
                                );
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_workspace_focus())
                {
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_workspace_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            let count = sv.sidebar_workspace_node_count();
                            sv.move_sidebar_workspace_selection_down(count);
                            return Ok(());
                        }
                        KeyCode::Left => {
                            if sv.collapse_sidebar_workspace_selection() {
                                return Ok(());
                            }
                        }
                        KeyCode::Right => {
                            if sv.expand_sidebar_workspace_selection() {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(sv) = self
                    .context
                    .session_view_handle()
                    .filter(|sv| sv.sidebar_attached_session_focus())
                {
                    match key.code {
                        KeyCode::Up => {
                            sv.move_sidebar_attached_session_selection_up();
                            return Ok(());
                        }
                        KeyCode::Down => {
                            let count = self.context.attached_sessions().len();
                            sv.move_sidebar_attached_session_selection_down(count);
                            return Ok(());
                        }
                        KeyCode::Enter => {
                            let sessions = self.context.attached_sessions();
                            if let Some(child) =
                                sessions.get(sv.sidebar_attached_session_selected())
                            {
                                let attached_id = child.session_id.clone();
                                self.context.navigate_session(attached_id.clone());
                                self.ensure_session_view(&attached_id);
                                let _ = self.sync_session_from_server(&attached_id);
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                if key.code == KeyCode::Char('p') && key.modifiers.is_empty() {
                    if let Some(sv) = self.context.session_view_handle() {
                        if sv.toggle_sidebar_process_focus(self.terminal_width()) {
                            return Ok(());
                        }
                    }
                }

                if key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
                    self.state = AppState::Exiting;
                    return Ok(());
                }

                if self.matches_keybind("session_interrupt", key) {
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
                    return Ok(());
                }

                if self.matches_keybind("input_paste", key) {
                    self.paste_clipboard_to_prompt();
                    return Ok(());
                }
                if self.matches_keybind("input_copy", key) {
                    self.copy_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("input_cut", key) {
                    self.cut_prompt_to_clipboard();
                    return Ok(());
                }
                if self.matches_keybind("history_previous", key) {
                    self.prompt.history_previous_entry();
                    return Ok(());
                }
                if self.matches_keybind("history_next", key) {
                    self.prompt.history_next_entry();
                    return Ok(());
                }
                if self.matches_keybind("page_up", key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(sv) = self.context.session_view_handle() {
                            sv.scroll_page_up();
                            return Ok(());
                        }
                    }
                }
                if self.matches_keybind("page_down", key) {
                    if let Route::Session { .. } = self.context.current_route() {
                        if let Some(sv) = self.context.session_view_handle() {
                            sv.scroll_page_down();
                            return Ok(());
                        }
                    }
                }

                if self.matches_keybind("command_palette", key) {
                    self.sync_command_palette_labels();
                    self.open_command_palette_dialog();
                    return Ok(());
                }
                if self.matches_keybind("model_cycle", key) {
                    self.refresh_model_dialog();
                    self.open_model_select_dialog();
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle", key) {
                    self.cycle_agent(1);
                    return Ok(());
                }
                if self.matches_keybind("agent_cycle_reverse", key) {
                    self.cycle_agent(-1);
                    return Ok(());
                }
                if self.matches_keybind("variant_cycle", key) {
                    self.cycle_model_variant();
                    return Ok(());
                }
                if self.matches_keybind("session_parent", key) {
                    self.navigate_to_parent_session();
                    return Ok(());
                }
                if self.matches_keybind("session_attached_open", key) {
                    self.navigate_to_attached_session();
                    return Ok(());
                }
                if self.matches_keybind("session_attached_focus", key) {
                    if let Some(sv) = self.context.session_view_handle() {
                        let _ = sv.toggle_sidebar_attached_session_focus(self.terminal_width());
                    }
                    return Ok(());
                }
                if self.matches_keybind("session_workspace_focus", key) {
                    if let Some(sv) = self.context.session_view_handle() {
                        let _ = sv.toggle_sidebar_workspace_focus(self.terminal_width());
                    }
                    return Ok(());
                }
                if self.matches_keybind("sidebar_toggle", key) {
                    self.toggle_session_sidebar();
                    return Ok(());
                }
                if self.matches_keybind("display_thinking", key) {
                    self.context.toggle_thinking();
                    return Ok(());
                }
                if self.matches_keybind("tool_details", key) {
                    self.context.toggle_tool_details();
                    return Ok(());
                }
                if self.matches_keybind("input_clear", key) {
                    self.discard_prompt_draft();
                    return Ok(());
                }
                if self.matches_keybind("input_newline", key) {
                    let route = self.context.current_route();
                    if matches!(route, Route::Home | Route::Session { .. }) {
                        self.prompt.insert_text("\n");
                        return Ok(());
                    }
                }
                if self.matches_keybind("help_toggle", key) {
                    self.open_help_dialog();
                    return Ok(());
                }

                let route = self.context.current_route();
                match route {
                    Route::Home | Route::Session { .. } => {
                        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                            self.submit_prompt()?;
                        } else if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(key);
                        }
                    }
                    _ => {
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.prompt.handle_key(key);
                        }
                    }
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
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if !sv.scroll_sidebar_up_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_up_mouse();
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if !sv.scroll_sidebar_down_at(mouse_event.column, mouse_event.row) {
                                sv.scroll_down_mouse();
                            }
                        }
                    }
                    MouseEventKind::Drag(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.update(mouse_event.row, mouse_event.column);
                            return Ok(());
                        }
                        let col = mouse_event.column;
                        let row = mouse_event.row;
                        if let Some(sv) = self.context.session_view_handle() {
                            if sv.handle_scrollbar_drag(col, row) {
                                return Ok(());
                            }
                        }
                        self.selection.update(row, col);
                    }
                    MouseEventKind::Moved => {
                        if self.status_dialog.is_open() {
                            self.event_caused_change = false;
                            return Ok(());
                        }
                        if self.handle_dialog_mouse(mouse_event)? {
                            return Ok(());
                        }
                        self.event_caused_change = self
                            .context
                            .session_view_handle()
                            .map(|sv| sv.handle_mouse_move(mouse_event.column, mouse_event.row))
                            .unwrap_or(false);
                    }
                    MouseEventKind::Up(_) => {
                        if self.status_dialog.is_open() {
                            self.selection.finalize();
                            return Ok(());
                        }
                        if let Some(sv) = self.context.session_view_handle() {
                            if sv.stop_scrollbar_drag() {
                                return Ok(());
                            }
                        }
                        self.selection.finalize();
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                if !text.is_empty() {
                    if self.provider_dialog.is_open() && self.provider_dialog.accepts_text_input() {
                        for c in text.chars() {
                            self.provider_dialog.push_char(c);
                        }
                    } else {
                        self.prompt.insert_text(text);
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
                                    self.sync_question_requests();
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
                            self.queue_permission_sync();
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
                            self.queue_permission_sync();
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
                CustomEvent::StateChanged(change) => self.handle_state_change(change),
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
                    if !self.local_direct_idle_session()
                        && self.sync_runtime.last_full_session_sync.elapsed()
                        >= Duration::from_secs(SESSION_FULL_SYNC_INTERVAL_SECS)
                        && self
                            .sync_session_from_server_with_mode(session_id, SessionSyncMode::Full)
                            .is_ok()
                    {
                        tick_changed = true;
                        self.refresh_attached_sessions();
                        if self.status_dialog.is_open() {
                            self.refresh_active_status_dialog();
                        }
                    }
                }
                if matches!(route, Route::Session { .. }) {
                    if (!self.local_direct_idle_session() || self.question_prompt.is_open)
                        && self.sync_runtime.last_question_sync.elapsed()
                        >= Duration::from_secs(QUESTION_SYNC_FALLBACK_SECS)
                        && self.sync_runtime.pending_question_sync_due_at.is_none()
                    {
                        self.queue_question_sync();
                    }
                    if (!self.local_direct_idle_session() || self.permission_prompt.is_open)
                        && self.sync_runtime.last_permission_sync.elapsed()
                        >= self.permission_sync_interval()
                        && self.sync_runtime.pending_permission_sync_due_at.is_none()
                    {
                        self.queue_permission_sync();
                    }
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
                if self.sync_runtime.last_aux_sync.elapsed() >= self.aux_sync_interval() {
                    if self.session_list_dialog.is_open() {
                        self.refresh_session_list_dialog();
                    }
                    if self.skill_list_dialog.is_open() {
                        let _ = self.refresh_skill_list_dialog();
                    }
                    if !self.local_direct {
                        let _ = self.refresh_lsp_status();
                        let _ = self.refresh_mcp_dialog();
                    }
                    self.sync_runtime.last_aux_sync = Instant::now();
                    tick_changed = true;
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
