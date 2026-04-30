use super::*;

pub(super) struct PromptDispatchRequest<'a> {
    pub session_id: &'a str,
    pub input: String,
    pub display_text: String,
    pub parts: Option<Vec<crate::api::PromptPart>>,
    pub agent: Option<String>,
    pub scheduler_profile: Option<String>,
    pub display_mode: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub idempotency_key: Option<String>,
}

impl App {
    pub(super) fn paste_clipboard_to_prompt(&mut self) {
        match Clipboard::read_text() {
            Ok(text) => {
                if !text.is_empty() {
                    self.prompt.insert_text(&text);
                }
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to read clipboard:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn paste_clipboard_to_provider_dialog(&mut self) {
        match Clipboard::read_text() {
            Ok(text) => {
                self.provider_dialog.set_input(text.trim().to_string());
            }
            Err(err) => {
                self.toast
                    .show(ToastVariant::Error, &format!("Paste failed: {}", err), 3000);
            }
        }
    }

    pub(super) fn copy_prompt_to_clipboard(&mut self) {
        let content = self.prompt.get_input().trim();
        if content.is_empty() {
            return;
        }
        if let Err(err) = Clipboard::write_text(content) {
            self.alert_dialog
                .set_message(&format!("Failed to write clipboard:\n{}", err));
            self.open_alert_dialog();
        }
    }

    pub(super) fn cut_prompt_to_clipboard(&mut self) {
        let content = self.prompt.get_input().trim();
        if content.is_empty() {
            return;
        }
        if let Err(err) = Clipboard::write_text(content) {
            self.alert_dialog
                .set_message(&format!("Failed to write clipboard:\n{}", err));
            self.open_alert_dialog();
            return;
        }
        self.prompt.clear();
    }

    /// Copy the current screen selection to clipboard and show a toast.
    pub(super) fn copy_selection(&mut self) {
        if !self.selection.is_active() {
            return;
        }
        let lines = self.screen_lines.clone();
        let mut text = self
            .selection
            .get_selected_text(|row| lines.get(row as usize).cloned());
        if matches!(self.context.current_route(), Route::Session { .. }) {
            text = strip_session_gutter(&text);
        }
        if !text.is_empty() {
            match Clipboard::write_text(&text) {
                Ok(()) => {
                    self.toast
                        .show(ToastVariant::Info, "Copied to clipboard", 2000);
                }
                Err(err) => {
                    self.toast
                        .show(ToastVariant::Error, &format!("Copy failed: {}", err), 3000);
                }
            }
        }
        self.selection.clear();
    }

    pub(super) fn submit_prompt(&mut self) -> anyhow::Result<()> {
        let shell_mode = self.prompt.is_shell_mode();
        let input = self.prompt.take_input();
        if input.trim().is_empty() {
            return Ok(());
        }

        if shell_mode {
            return self.submit_shell_command(input);
        }

        let ui_registry = CommandRegistry::new();
        if let Some(invocation) = ui_registry.resolve_ui_slash_input(&input) {
            self.execute_ui_action_invocation(&invocation)?;
            return Ok(());
        }

        if let Some(command) = parse_interactive_command(&input) {
            if self.execute_typed_interactive_command(command)? {
                return Ok(());
            }
        }

        self.submit_prompt_payload(input.clone(), input, None)
    }

    pub(super) fn submit_prompt_payload(
        &mut self,
        input: String,
        display_text: String,
        parts: Option<Vec<crate::api::PromptPart>>,
    ) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            eprintln!("API client not initialized");
            return Ok(());
        };

        let selected_mode = resolve_command_execution_mode(
            &self.context,
            &display_text,
            selected_execution_mode(&self.context),
        );
        let model = self.selected_model_for_prompt();
        let variant = self.context.current_model_variant();

        match self.context.current_route() {
            Route::Home => {
                let optimistic_session_id = self.create_optimistic_session();
                let opt_id = self.append_optimistic_user_message(
                    &optimistic_session_id,
                    &display_text,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                self.ensure_session_view(&optimistic_session_id);
                self.set_session_status(&optimistic_session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
                self.event_caused_change = true;
                let context = self.context.clone();
                let session_directory = self.context.directory.read().clone();
                thread::spawn(move || {
                    let (created_session, response, error) = match client.create_session(
                        None,
                        selected_mode.scheduler_profile.clone(),
                        Some(session_directory),
                    ) {
                        Ok(session) => match client.send_prompt(
                            &session.id,
                            input,
                            parts,
                            selected_mode.agent,
                            selected_mode.scheduler_profile,
                            model,
                            variant,
                            Some(format!("tui_{}", opt_id)),
                        ) {
                            Ok(response) => (Some(session), Some(response), None),
                            Err(err) => (Some(session), None, Some(err.to_string())),
                        },
                        Err(err) => (None, None, Some(err.to_string())),
                    };
                    let _ = context.emit_custom_event(CustomEvent::PromptDispatchHomeFinished {
                        optimistic_session_id,
                        optimistic_message_id: opt_id,
                        created_session: created_session.map(Box::new),
                        response,
                        error,
                    });
                });
            }
            Route::Session { session_id } => {
                if self.is_session_busy(&session_id) {
                    self.enqueue_prompt(
                        &session_id,
                        QueuedPrompt {
                            input,
                            display_text,
                            parts,
                            agent: selected_mode.agent,
                            scheduler_profile: selected_mode.scheduler_profile,
                            display_mode: selected_mode.display_mode,
                            model,
                            variant,
                            idempotency_key: Some(format!("tui_{}", uuid::Uuid::new_v4().simple())),
                        },
                    );
                    self.event_caused_change = true;
                    return Ok(());
                }
                self.dispatch_prompt_to_session(PromptDispatchRequest {
                    session_id: &session_id,
                    display_text,
                    input,
                    parts,
                    agent: selected_mode.agent,
                    scheduler_profile: selected_mode.scheduler_profile,
                    display_mode: selected_mode.display_mode,
                    model,
                    variant,
                    idempotency_key: None,
                });
            }
            _ => {}
        }

        Ok(())
    }

    pub(super) fn is_session_busy(&self, session_id: &str) -> bool {
        let session_ctx = self.context.session.read();
        !matches!(session_ctx.status(session_id), SessionStatus::Idle)
    }

    pub(super) fn enqueue_prompt(&mut self, session_id: &str, queued: QueuedPrompt) {
        let count = {
            let queue = self
                .prompt_runtime
                .pending_queue
                .entry(session_id.to_string())
                .or_default();
            queue.push_back(queued);
            queue.len()
        };
        self.context.set_queued_prompts(session_id, count);
        self.toast.show(
            ToastVariant::Info,
            &format!("Session busy, queued prompt ({})", count),
            2000,
        );
    }

    pub(super) fn dispatch_prompt_to_session(&mut self, request: PromptDispatchRequest<'_>) {
        let PromptDispatchRequest {
            session_id,
            input,
            display_text,
            parts,
            agent,
            scheduler_profile,
            display_mode,
            model,
            variant,
            idempotency_key,
        } = request;

        let Some(client) = self.context.get_api_client() else {
            eprintln!("API client not initialized");
            return;
        };

        // Optimistic: show user message immediately before network call.
        let opt_id = self.append_optimistic_user_message(
            session_id,
            &display_text,
            display_mode.clone(),
            model.clone(),
            variant.clone(),
        );
        self.set_session_status(session_id, SessionStatus::Running);
        self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
        self.prompt.set_spinner_active(true);
        self.ensure_session_view(session_id);
        self.event_caused_change = true;

        let context = self.context.clone();
        let session_id = session_id.to_string();
        thread::spawn(move || {
            let (response, error) = match client.send_prompt(
                &session_id,
                input,
                parts,
                agent,
                scheduler_profile,
                model,
                variant,
                idempotency_key.or_else(|| Some(format!("tui_{}", opt_id))),
            ) {
                Ok(response) => (Some(response), None),
                Err(err) => (None, Some(err.to_string())),
            };
            let _ = context.emit_custom_event(CustomEvent::PromptDispatchSessionFinished {
                session_id,
                optimistic_message_id: opt_id,
                response,
                error,
            });
        });
    }

    pub(super) fn dispatch_next_queued_prompt(&mut self, session_id: &str) -> bool {
        if self.is_session_busy(session_id) {
            return false;
        }

        let queued = {
            let Some(queue) = self.prompt_runtime.pending_queue.get_mut(session_id) else {
                self.context.set_queued_prompts(session_id, 0);
                return false;
            };
            let queued = queue.pop_front();
            let remaining = queue.len();
            self.context.set_queued_prompts(session_id, remaining);
            (queued, remaining == 0)
        };

        if queued.1 {
            self.prompt_runtime.pending_queue.remove(session_id);
        }

        if let Some(queued) = queued.0 {
            self.dispatch_prompt_to_session(PromptDispatchRequest {
                session_id,
                input: queued.input,
                display_text: queued.display_text,
                parts: queued.parts,
                agent: queued.agent,
                scheduler_profile: queued.scheduler_profile,
                display_mode: queued.display_mode,
                model: queued.model,
                variant: queued.variant,
                idempotency_key: queued.idempotency_key,
            });
            return true;
        }

        false
    }

    pub(super) fn submit_shell_command(&mut self, command: String) -> anyhow::Result<()> {
        let command = command.trim().to_string();
        if command.is_empty() {
            return Ok(());
        }

        let Some(client) = self.context.get_api_client() else {
            eprintln!("API client not initialized");
            return Ok(());
        };

        let user_line = format!("$ {}", command);
        let selected_mode = selected_execution_mode(&self.context);
        let model = self.selected_model_for_prompt();
        let variant = self.context.current_model_variant();

        match self.context.current_route() {
            Route::Home => {
                let optimistic_session_id = self.create_optimistic_session();
                let _opt_id = self.append_optimistic_user_message(
                    &optimistic_session_id,
                    &user_line,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                self.ensure_session_view(&optimistic_session_id);
                self.event_caused_change = true;

                let session_directory = self.context.directory.read().clone();
                let session = match client.create_session(
                    None,
                    selected_mode.scheduler_profile.clone(),
                    Some(session_directory),
                ) {
                    Ok(session) => session,
                    Err(err) => {
                        self.remove_optimistic_session(&optimistic_session_id);
                        self.context.navigate_home();
                        self.alert_dialog
                            .set_message(&format!("Failed to create session:\n{}", err));
                        self.open_alert_dialog();
                        return Ok(());
                    }
                };
                self.promote_optimistic_session(&optimistic_session_id, &session);
                self.context.navigate(Route::Session {
                    session_id: session.id.clone(),
                });
                self.ensure_session_view(&session.id);

                if let Err(err) = client.execute_shell(&session.id, command.clone(), None) {
                    self.alert_dialog
                        .set_message(&format!("Failed to execute shell command:\n{}", err));
                    self.open_alert_dialog();
                    return Ok(());
                }

                self.set_session_status(&session.id, SessionStatus::Idle);
                let _ = self.sync_session_from_server(&session.id);
            }
            Route::Session { session_id } => {
                let opt_id = self.append_optimistic_user_message(
                    &session_id,
                    &user_line,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.ensure_session_view(&session_id);
                self.event_caused_change = true;
                if let Err(err) = client.execute_shell(&session_id, command.clone(), None) {
                    self.remove_optimistic_message(&session_id, &opt_id);
                    self.alert_dialog
                        .set_message(&format!("Failed to execute shell command:\n{}", err));
                    self.open_alert_dialog();
                    return Ok(());
                }
                self.set_session_status(&session_id, SessionStatus::Idle);
                let _ = self.sync_session_from_server(&session_id);
            }
            _ => {}
        }

        self.sync_prompt_spinner_state();
        Ok(())
    }
}
