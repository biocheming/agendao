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

#[derive(Clone, Debug)]
pub(super) struct PromptDispatchStarted {
    pub optimistic_message_id: String,
}

impl App {
    pub(super) fn paste_clipboard_to_prompt(&mut self) {
        match Clipboard::read() {
            Ok(content) => {
                if content.mime.starts_with("image/") {
                    self.queue_clipboard_image_attachment(content);
                    return;
                }
                if !content.data.is_empty() {
                    self.prompt.insert_text(&content.data);
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
        let lines = self.screen_lines.lock().unwrap_or_else(|e| e.into_inner()).clone();
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
        let has_pending_parts = self.prompt_draft.has_attachments();
        if input.trim().is_empty() && !has_pending_parts {
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

        let parts = self.prompt_draft.take_attachments();
        self.sync_prompt_draft_hint();
        self.submit_prompt_payload(input.clone(), input, parts)
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
                if self.local_direct {
                    let session_directory = self.context.directory.read().clone();
                    let session = match client.create_session(
                        selected_mode.scheduler_profile.clone(),
                        Some(session_directory),
                    ) {
                        Ok(session) => session,
                        Err(err) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to create session:\n{}", err));
                            self.open_alert_dialog();
                            self.event_caused_change = true;
                            return Ok(());
                        }
                    };
                    self.cache_session_from_api(&session);
                    self.context.navigate(Route::Session {
                        session_id: session.id.clone(),
                    });
                    self.sse_session_filter
                        .send_replace(Some(session.id.clone()));
                    self.dispatch_prompt_to_session(PromptDispatchRequest {
                        session_id: &session.id,
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
                    return Ok(());
                }

                let optimistic_session_id = self.create_optimistic_session();
                let started = self.begin_prompt_dispatch(
                    &optimistic_session_id,
                    &display_text,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                let context = self.context.clone();
                let session_directory = self.context.directory.read().clone();
                crate::app::app_impl::support::spawn_background_task(async move {
                    let (created_session, response, error) = match client.create_session(
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
                            Some(format!("tui_{}", started.optimistic_message_id)),
                        ) {
                            Ok(response) => (Some(session), Some(response), None),
                            Err(err) => (Some(session), None, Some(err.to_string())),
                        },
                        Err(err) => (None, None, Some(err.to_string())),
                    };
                    let _ = context.emit_custom_event(CustomEvent::PromptDispatchHomeFinished {
                        optimistic_session_id,
                        optimistic_message_id: started.optimistic_message_id,
                        created_session: created_session.map(Box::new),
                        response,
                        error,
                    });
                });
            }
            Route::Session { session_id } => {
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

        let started = self.begin_prompt_dispatch(
            session_id,
            &display_text,
            display_mode.clone(),
            model.clone(),
            variant.clone(),
        );

        let context = self.context.clone();
        let session_id = session_id.to_string();
        crate::app::app_impl::support::spawn_background_task(async move {
            let (response, error) = match client.send_prompt(
                &session_id,
                input,
                parts,
                agent,
                scheduler_profile,
                model,
                variant,
                idempotency_key.or_else(|| Some(format!("tui_{}", started.optimistic_message_id))),
            ) {
                Ok(response) => (Some(response), None),
                Err(err) => (None, Some(err.to_string())),
            };
            let _ = context.emit_custom_event(CustomEvent::PromptDispatchSessionFinished {
                session_id,
                optimistic_message_id: started.optimistic_message_id,
                response,
                error,
            });
        });
    }

    pub(super) fn begin_prompt_dispatch(
        &mut self,
        session_id: &str,
        display_text: &str,
        display_mode: Option<String>,
        model: Option<String>,
        variant: Option<String>,
    ) -> PromptDispatchStarted {
        let optimistic_message_id = self.append_optimistic_user_message(
            session_id,
            display_text,
            display_mode,
            model,
            variant,
        );
        self.ensure_session_view(session_id);
        self.set_session_status(session_id, SessionStatus::Running);
        self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
        self.prompt.set_spinner_active(true);
        self.event_caused_change = true;
        PromptDispatchStarted {
            optimistic_message_id,
        }
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
                if self.local_direct {
                    let session_directory = self.context.directory.read().clone();
                    let session = match client.create_session(
                        selected_mode.scheduler_profile.clone(),
                        Some(session_directory),
                    ) {
                        Ok(session) => session,
                        Err(err) => {
                            self.alert_dialog
                                .set_message(&format!("Failed to create session:\n{}", err));
                            self.open_alert_dialog();
                            self.event_caused_change = true;
                            return Ok(());
                        }
                    };
                    self.cache_session_from_api(&session);
                    self.context.navigate(Route::Session {
                        session_id: session.id.clone(),
                    });
                    self.sse_session_filter
                        .send_replace(Some(session.id.clone()));
                    let started = self.begin_prompt_dispatch(
                        &session.id,
                        &user_line,
                        selected_mode.display_mode.clone(),
                        model.clone(),
                        variant.clone(),
                    );
                    self.pending_shell_dispatch = Some(PendingShellDispatch {
                        session_id: session.id.clone(),
                        optimistic_message_id: started.optimistic_message_id.clone(),
                    });

                    let context = self.context.clone();
                    let session_id = session.id.clone();
                    crate::app::app_impl::support::spawn_background_task(async move {
                        let result = client.execute_shell(&session_id, command.clone(), None);
                        let _ = context.emit_custom_event(CustomEvent::ShellDispatchFinished {
                            optimistic_session_id: session_id.clone(),
                            session_id: session_id.clone(),
                            optimistic_message_id: started.optimistic_message_id,
                            created_session: None,
                            cancelled: false,
                            error: result.err().map(|e| e.to_string()),
                        });
                    });
                    return Ok(());
                }

                let optimistic_session_id = self.create_optimistic_session();
                let started = self.begin_prompt_dispatch(
                    &optimistic_session_id,
                    &user_line,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.context.navigate(Route::Session {
                    session_id: optimistic_session_id.clone(),
                });
                self.pending_shell_dispatch = Some(PendingShellDispatch {
                    session_id: optimistic_session_id.clone(),
                    optimistic_message_id: started.optimistic_message_id.clone(),
                });

                let context = self.context.clone();
                let session_directory = self.context.directory.read().clone();
                let opt_session_id = optimistic_session_id.clone();
                crate::app::app_impl::support::spawn_background_task(async move {
                    let (real_session_id, created_session, error) = match client.create_session(
                        selected_mode.scheduler_profile.clone(),
                        Some(session_directory),
                    ) {
                        Ok(session) => {
                            let sid = session.id.clone();
                            match client.execute_shell(&session.id, command.clone(), None) {
                                Ok(_) => (sid, Some(session), None),
                                Err(err) => (sid, Some(session), Some(err.to_string())),
                            }
                        }
                        Err(err) => (String::new(), None, Some(err.to_string())),
                    };
                    let _ = context.emit_custom_event(CustomEvent::ShellDispatchFinished {
                        optimistic_session_id: opt_session_id,
                        session_id: real_session_id,
                        optimistic_message_id: started.optimistic_message_id,
                        created_session: created_session.map(Box::new),
                        cancelled: false,
                        error,
                    });
                });
            }
            Route::Session { session_id } => {
                let started = self.begin_prompt_dispatch(
                    &session_id,
                    &user_line,
                    selected_mode.display_mode.clone(),
                    model.clone(),
                    variant.clone(),
                );
                self.pending_shell_dispatch = Some(PendingShellDispatch {
                    session_id: session_id.clone(),
                    optimistic_message_id: started.optimistic_message_id.clone(),
                });

                let context = self.context.clone();
                let session_id = session_id.to_string();
                crate::app::app_impl::support::spawn_background_task(async move {
                    let result = client.execute_shell(&session_id, command.clone(), None);
                    let _ = context.emit_custom_event(CustomEvent::ShellDispatchFinished {
                        optimistic_session_id: session_id.clone(),
                        session_id: session_id.clone(),
                        optimistic_message_id: started.optimistic_message_id,
                        created_session: None,
                        cancelled: false,
                        error: result.err().map(|e| e.to_string()),
                    });
                });
            }
            _ => {}
        }

        Ok(())
    }

    pub(super) fn settle_shell_dispatch(
        &mut self,
        session_id: &str,
        optimistic_message_id: Option<&str>,
        outcome: ShellDispatchOutcome,
    ) {
        self.pending_shell_dispatch = None;
        match outcome {
            ShellDispatchOutcome::Failed => {
                if let Some(msg_id) = optimistic_message_id {
                    self.remove_optimistic_message(session_id, msg_id);
                }
                self.set_session_status(session_id, SessionStatus::Idle);
                self.sync_prompt_spinner_state();
            }
            ShellDispatchOutcome::Sent => {
                self.set_session_status(session_id, SessionStatus::Idle);
                self.sync_prompt_spinner_state();
            }
            ShellDispatchOutcome::Cancelled => {
                if let Some(msg_id) = optimistic_message_id {
                    self.remove_optimistic_message(session_id, msg_id);
                }
                self.set_session_status(session_id, SessionStatus::Idle);
                self.sync_prompt_spinner_state();
            }
        }
    }
}

pub(super) enum ShellDispatchOutcome {
    Sent,
    Failed,
    Cancelled,
}
