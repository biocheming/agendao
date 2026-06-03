use super::*;

impl App {
    fn pending_command_from_session(
        session: &crate::api::SessionInfo,
        question_id: &str,
    ) -> Option<crate::api::PendingCommandInvocation> {
        let metadata = session.metadata.as_ref()?;
        let pending = metadata.get("pending_command_invocation")?.clone();
        let pending =
            serde_json::from_value::<crate::api::PendingCommandInvocation>(pending).ok()?;
        if pending
            .question_id
            .as_deref()
            .is_some_and(|candidate| candidate != question_id)
        {
            return None;
        }
        Some(pending)
    }

    fn shell_quote_command_value(value: &str) -> String {
        if value.is_empty() {
            return "\"\"".to_string();
        }
        if value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '*' | ':'))
        {
            return value.to_string();
        }
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }

    fn split_repeatable_answer(answer: &str) -> Vec<String> {
        answer
            .split(|ch: char| matches!(ch, '\n' | ',' | '\t'))
            .flat_map(|segment| segment.split_whitespace())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn merge_pending_command_arguments(
        pending: &crate::api::PendingCommandInvocation,
        answers: &[Vec<String>],
    ) -> String {
        let mut parts = Vec::new();
        let raw = pending.raw_arguments.trim();
        if !raw.is_empty() {
            parts.push(raw.to_string());
        }

        for (index, field) in pending.missing_fields.iter().enumerate() {
            let values = answers
                .get(index)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .flat_map(|value| {
                    if value.contains('\n') || value.contains(',') {
                        Self::split_repeatable_answer(&value)
                    } else {
                        vec![value]
                    }
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if values.is_empty() {
                continue;
            }
            parts.push(format!("--{}", field));
            parts.extend(
                values
                    .iter()
                    .map(|value| Self::shell_quote_command_value(value)),
            );
        }

        parts.join(" ").trim().to_string()
    }

    pub(super) fn question_prompt_at(
        question: &QuestionInfo,
        index: usize,
    ) -> Option<QuestionRequest> {
        // Prefer full-fidelity `items` field; fallback to legacy `questions`/`options`.
        if let Some(item) = question.items.get(index) {
            let mut options: Vec<QuestionOption> = item
                .options
                .iter()
                .map(|o| QuestionOption {
                    id: o.label.clone(),
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect();
            if !options
                .iter()
                .any(|option| option.id.eq_ignore_ascii_case(OTHER_OPTION_ID))
            {
                options.push(QuestionOption {
                    id: OTHER_OPTION_ID.to_string(),
                    label: OTHER_OPTION_LABEL.to_string(),
                    description: None,
                });
            }
            let question_type = if options.is_empty() {
                QuestionType::Text
            } else if item.multiple {
                QuestionType::MultipleChoice
            } else {
                QuestionType::SingleChoice
            };
            return Some(QuestionRequest {
                id: question.id.clone(),
                question: item.question.clone(),
                question_type,
                options,
            });
        }

        // Legacy path: consume `questions`/`options` fields.
        let prompt_text = question.questions.get(index)?.clone();
        let option_labels = question
            .options
            .as_ref()
            .and_then(|all| all.get(index).cloned())
            .unwrap_or_default();
        let mut options = option_labels
            .into_iter()
            .map(|label| QuestionOption {
                id: label.clone(),
                label,
                description: None,
            })
            .collect::<Vec<_>>();
        if !options
            .iter()
            .any(|option| option.id.eq_ignore_ascii_case(OTHER_OPTION_ID))
        {
            options.push(QuestionOption {
                id: OTHER_OPTION_ID.to_string(),
                label: OTHER_OPTION_LABEL.to_string(),
                description: None,
            });
        }
        let question_type = if options.is_empty() {
            QuestionType::Text
        } else {
            QuestionType::SingleChoice
        };
        Some(QuestionRequest {
            id: question.id.clone(),
            question: prompt_text,
            question_type,
            options,
        })
    }

    pub(super) fn clear_question_tracking(&mut self, question_id: &str) {
        self.question_runtime.pending_ids.remove(question_id);
        self.question_runtime.pending_questions.remove(question_id);
        self.question_runtime
            .pending_queue
            .retain(|id| id != question_id);
        self.question_runtime.pending_drafts.remove(question_id);
    }

    pub(super) fn open_next_question_prompt(&mut self) -> bool {
        if self.question_prompt.is_open {
            return false;
        }

        while let Some(question_id) = self.question_runtime.pending_queue.pop_front() {
            let Some(question) = self
                .question_runtime
                .pending_questions
                .get::<str>(question_id.as_str())
                .cloned()
            else {
                continue;
            };
            let draft = self
                .question_runtime
                .pending_drafts
                .entry(question_id.clone())
                .or_default()
                .clone();
            if let Some(prompt) = Self::question_prompt_at(&question, draft.current_index) {
                self.question_prompt.ask(prompt);
                return true;
            }
            self.clear_question_tracking(&question_id);
        }
        false
    }

    pub(super) fn sync_question_requests(&mut self) -> bool {
        self.diagnostics.perf.question_sync = self.diagnostics.perf.question_sync.saturating_add(1);
        let Some(client) = self.context.get_api_client() else {
            return false;
        };

        let active_session = match self.context.current_route() {
            Route::Session { session_id } => Some(session_id),
            _ => None,
        };

        let mut questions = match client.list_questions() {
            Ok(items) => items,
            Err(err) => {
                tracing::debug!(%err, "failed to list pending questions");
                return false;
            }
        };

        if let Some(session_id) = active_session.as_deref() {
            questions.retain(|q| q.session_id == session_id);
        }
        questions.sort_by(|a, b| a.id.cmp(&b.id));

        let latest_ids = questions
            .iter()
            .map(|q| q.id.clone())
            .collect::<HashSet<_>>();
        let mut changed = latest_ids != self.question_runtime.pending_ids;

        for question in questions {
            let question_id = question.id.clone();
            self.question_runtime
                .pending_questions
                .insert(question_id.clone(), question);
            if self
                .question_runtime
                .pending_ids
                .insert(question_id.clone())
            {
                self.question_runtime.pending_queue.push_back(question_id);
                changed = true;
            }
        }

        self.question_runtime
            .pending_ids
            .retain(|id| latest_ids.contains(id));
        self.question_runtime
            .pending_questions
            .retain(|id, _| latest_ids.contains(id));
        self.question_runtime
            .pending_queue
            .retain(|id| latest_ids.contains(id));

        if let Some(current_id) = self.question_prompt.current().map(|q| q.id.clone()) {
            if !latest_ids.contains(&current_id) {
                self.question_prompt.close();
                changed = true;
            }
        }

        if self.open_next_question_prompt() {
            changed = true;
        }
        changed
    }

    fn resume_pending_command_after_question(
        &mut self,
        question_id: &str,
        answers: &[Vec<String>],
    ) -> anyhow::Result<()> {
        let Some(session_id) = self.current_session_id() else {
            return Ok(());
        };
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let session = client.get_session(&session_id)?;
        let Some(pending) = Self::pending_command_from_session(&session, question_id) else {
            return Ok(());
        };
        let arguments = Self::merge_pending_command_arguments(&pending, answers);
        let response = client.send_command_prompt(
            &session_id,
            pending.command.clone(),
            (!arguments.trim().is_empty()).then_some(arguments),
            self.selected_model_for_prompt(),
            self.context.current_model_variant(),
            Some(format!("tui_command_{}", uuid::Uuid::new_v4().simple())),
        )?;

        match response.status.as_str() {
            "accepted" => {
                self.set_session_status(&session_id, SessionStatus::Running);
                self.prompt.set_spinner_task_kind(TaskKind::LlmRequest);
                self.prompt.set_spinner_active(true);
                self.queue_session_telemetry_refresh(&session_id);
            }
            "awaiting_user" => {
                self.set_session_status(&session_id, SessionStatus::Idle);
                self.prompt.set_spinner_active(false);
                self.queue_session_telemetry_refresh(&session_id);
                self.sync_question_requests();
            }
            _ => {
                self.prompt.set_spinner_active(false);
            }
        }
        Ok(())
    }

    pub(super) fn submit_question_reply(&mut self, question_id: &str, answers: Vec<String>) {
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog
                .set_message("Cannot answer question: no API client");
            self.open_alert_dialog();
            return;
        };

        let question = self
            .question_runtime
            .pending_questions
            .get(question_id)
            .cloned();
        let normalized_answers = answers
            .into_iter()
            .map(|answer| answer.trim().to_string())
            .filter(|answer| !answer.is_empty())
            .collect::<Vec<_>>();
        let mut next_prompt = None;
        let answers = {
            let draft = self
                .question_runtime
                .pending_drafts
                .entry(question_id.to_string())
                .or_default();
            let current_index = draft.current_index;
            if draft.answers.len() <= current_index {
                draft.answers.resize(current_index + 1, Vec::new());
            }
            let mut question_answers = normalized_answers;
            if question_answers.is_empty() {
                if let Some(default_option) = question
                    .as_ref()
                    .and_then(|q| q.options.as_ref())
                    .and_then(|all| all.get(current_index))
                    .and_then(|opts| {
                        opts.iter()
                            .find(|option| !option.eq_ignore_ascii_case(OTHER_OPTION_LABEL))
                            .cloned()
                    })
                {
                    question_answers.push(default_option);
                }
            }
            draft.answers[current_index] = question_answers;

            let question_count = question
                .as_ref()
                .map(|q| q.items.len().max(q.questions.len()))
                .unwrap_or(1)
                .max(1);
            if draft.answers.len() < question_count {
                draft.answers.resize(question_count, Vec::new());
            }

            if current_index + 1 < question_count {
                draft.current_index += 1;
                if let Some(question) = question.as_ref() {
                    next_prompt = Self::question_prompt_at(question, draft.current_index);
                }
            }

            draft.answers.clone()
        };

        if let Some(prompt) = next_prompt {
            self.question_prompt.ask(prompt);
            return;
        }

        match client.reply_question(question_id, answers.clone()) {
            Ok(()) => {
                if let Err(error) =
                    self.resume_pending_command_after_question(question_id, &answers)
                {
                    self.alert_dialog
                        .set_message(&format!("Failed to resume pending command:\n{}", error));
                    self.open_alert_dialog();
                }
                self.clear_question_tracking(question_id);
                self.toast
                    .show(ToastVariant::Success, "Question answered", 2000);
                let _ = self.open_next_question_prompt();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to submit question response:\n{}", err));
                self.open_alert_dialog();
                let current_index = self
                    .question_runtime
                    .pending_drafts
                    .get(question_id)
                    .map(|draft| draft.current_index)
                    .unwrap_or(0);
                if let Some(prompt) = question
                    .as_ref()
                    .and_then(|question| Self::question_prompt_at(question, current_index))
                {
                    self.question_prompt.ask(prompt);
                }
            }
        }
    }

    pub(super) fn reject_question(&mut self, question_id: &str) {
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog
                .set_message("Cannot reject question: no API client");
            self.open_alert_dialog();
            return;
        };

        match client.reject_question(question_id) {
            Ok(()) => {
                self.clear_question_tracking(question_id);
                self.toast
                    .show(ToastVariant::Info, "Question rejected", 1500);
                let _ = self.open_next_question_prompt();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to reject question:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_question_prompt_key(&mut self, key: KeyEvent) -> bool {
        if !self.question_prompt.is_open {
            return false;
        }

        match key.code {
            KeyCode::Up | KeyCode::BackTab => self.question_prompt.move_up(),
            KeyCode::Down | KeyCode::Tab => self.question_prompt.move_down(),
            KeyCode::Char(' ') => self.question_prompt.handle_space(),
            KeyCode::Enter => {
                if let Some((question, answers)) = self.question_prompt.confirm() {
                    self.submit_question_reply(&question.id, answers);
                }
            }
            KeyCode::Esc => {
                if let Some(question) = self.question_prompt.current().cloned() {
                    self.reject_question(&question.id);
                }
                self.question_prompt.close();
            }
            KeyCode::Char(c) => self.question_prompt.type_char(c),
            KeyCode::Backspace => self.question_prompt.backspace(),
            _ => {}
        }

        true
    }

    pub(super) fn handle_question_prompt_mouse(&mut self, col: u16, row: u16) -> bool {
        if !self.question_prompt.is_open {
            return false;
        }

        self.question_prompt.handle_click(col, row);
        true
    }
}
