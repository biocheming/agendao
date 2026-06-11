use super::*;

impl App {
    pub(super) fn set_active_model_selection(
        &mut self,
        model_ref: String,
        provider: Option<String>,
    ) {
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        let resolved_provider = provider.or_else(|| provider_from_model(&model_key));
        self.context
            .set_model_selection(model_key.clone(), resolved_provider);
        if let Some(variant) = explicit_variant {
            self.model_variant_selection
                .insert(model_key.clone(), Some(variant.clone()));
            self.context.set_model_variant(Some(variant));
            return;
        }
        let variant = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten();
        self.context.set_model_variant(variant);
    }

    pub(super) fn sync_current_model_variant(&mut self) {
        let Some(model_ref) = self.context.current_model() else {
            self.context.set_model_variant(None);
            return;
        };
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        if let Some(explicit) = explicit_variant {
            self.model_variant_selection
                .insert(model_key.clone(), Some(explicit.clone()));
            self.context.set_model_variant(Some(explicit));
            return;
        }
        let selected = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten();
        let available = self.model_variants.get(&model_key);
        let valid = selected.filter(|value| {
            available
                .map(|items| items.iter().any(|item| item == value))
                .unwrap_or(false)
        });
        if valid.is_none() {
            self.model_variant_selection.insert(model_key, None);
        }
        self.context.set_model_variant(valid);
    }

    pub(super) fn cycle_model_variant(&mut self) {
        let Some(model_ref) = self.context.current_model() else {
            return;
        };
        let (model_key, explicit_variant) =
            parse_model_ref_selection(&model_ref, &self.available_models, &self.model_variants);
        let Some(variants) = self.model_variants.get(&model_key).cloned() else {
            self.model_variant_selection.remove(&model_key);
            self.context.set_model_variant(None);
            return;
        };
        if variants.is_empty() {
            self.model_variant_selection.insert(model_key, None);
            self.context.set_model_variant(None);
            return;
        }

        let current = self
            .model_variant_selection
            .get(&model_key)
            .cloned()
            .flatten()
            .or(explicit_variant);
        let next = match current {
            None => Some(variants[0].clone()),
            Some(current_value) => {
                let index = variants.iter().position(|item| item == &current_value);
                match index {
                    Some(idx) if idx + 1 < variants.len() => Some(variants[idx + 1].clone()),
                    _ => None,
                }
            }
        };
        self.model_variant_selection.insert(model_key, next.clone());
        self.context.set_model_variant(next);
    }

    pub(super) fn current_model_label(&self) -> String {
        let Some(model) = self.context.current_model() else {
            return "(not selected)".to_string();
        };
        let (base_model, _) =
            parse_model_ref_selection(&model, &self.available_models, &self.model_variants);
        if let Some(variant) = self.context.current_model_variant() {
            return format!("{base_model} ({variant})");
        }
        base_model
    }

    pub(super) fn selected_model_for_prompt(&self) -> Option<String> {
        let model = self.context.current_model()?;
        let (base, inline_variant) =
            parse_model_ref_selection(&model, &self.available_models, &self.model_variants);
        let variant = self.context.current_model_variant();

        let resolved = if let Some(variant) = variant {
            let candidate = format!("{base}/{variant}");
            if self.available_models.contains(&candidate) {
                candidate
            } else {
                model.clone()
            }
        } else if inline_variant.is_some() && self.available_models.contains(&base) {
            base
        } else {
            model.clone()
        };

        Some(resolved)
    }

    /// Fetch the provider list from the server and populate the dialog.
    pub(super) fn populate_provider_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        if let Ok(schema) = client.get_provider_connect_schema() {
            self.provider_dialog.populate_from_connect_schema(schema);
        } else if let Ok(resp) = client.get_known_providers() {
            self.provider_dialog.populate_from_known(resp.providers);
            self.provider_dialog.clear_resolution();
        }

        if let Ok(config) = client.get_config() {
            let mut overrides = config
                .provider
                .unwrap_or_default()
                .into_iter()
                .flat_map(|(provider_id, provider)| {
                    provider.models.unwrap_or_default().into_iter().map(
                        move |(model_key, config)| crate::components::ProviderModelOverride {
                            provider_id: provider_id.clone(),
                            model_key,
                            config,
                        },
                    )
                })
                .collect::<Vec<_>>();
            overrides.sort_by(|a, b| {
                a.provider_id
                    .cmp(&b.provider_id)
                    .then_with(|| a.model_key.cmp(&b.model_key))
            });
            self.provider_dialog.set_model_overrides(overrides);
        }
    }

    pub(super) fn resolve_provider_dialog_search(&mut self) {
        let query = self.provider_dialog.search_query.trim().to_string();
        if query.is_empty() {
            self.provider_dialog.clear_resolution();
            return;
        }

        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_resolve_error("No API connection".to_string());
            return;
        };

        match client.resolve_provider_connect(&query) {
            Ok(response) => self.provider_dialog.apply_resolve_response(response),
            Err(error) => self.provider_dialog.set_resolve_error(error.to_string()),
        }
    }

    pub(super) fn quick_connect_provider_dialog_selection(&mut self) {
        let query = self
            .provider_dialog
            .selected_provider()
            .map(|provider| provider.id)
            .or_else(|| {
                let query = self.provider_dialog.search_query.trim();
                (!query.is_empty()).then_some(query.to_string())
            });

        let Some(query) = query else {
            return;
        };

        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_resolve_error("No API connection".to_string());
            return;
        };

        match client.resolve_provider_connect(&query) {
            Ok(response) => {
                self.provider_dialog
                    .apply_resolve_response(response.clone());
                match response.draft.mode {
                    crate::api::ProviderConnectDraftMode::Known => {
                        let mut provider =
                            crate::components::provider_from_connect_draft(&response.draft);
                        if let Ok(descriptor) = client.get_provider_descriptor(&provider.id) {
                            provider.descriptor_candidate = descriptor.descriptor_candidate;
                            provider.descriptor_candidate_error =
                                descriptor.descriptor_candidate_error;
                        }
                        self.provider_dialog.enter_input_mode_for_provider(provider);
                    }
                    crate::api::ProviderConnectDraftMode::Custom => {
                        self.provider_dialog.start_custom_flow_with_prefill(
                            response.custom_draft.provider_id,
                            response.custom_draft.base_url.unwrap_or_default(),
                            response
                                .custom_draft
                                .protocol
                                .unwrap_or_else(|| "openai".to_string()),
                        );
                    }
                }
            }
            Err(error) => self.provider_dialog.set_resolve_error(error.to_string()),
        }
    }

    pub(super) fn start_advanced_provider_dialog_selection(&mut self) {
        let query = self
            .provider_dialog
            .selected_provider()
            .map(|provider| provider.id)
            .or_else(|| {
                let query = self.provider_dialog.search_query.trim();
                (!query.is_empty()).then_some(query.to_string())
            });

        let Some(query) = query else {
            return;
        };

        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_resolve_error("No API connection".to_string());
            return;
        };

        match client.resolve_provider_connect(&query) {
            Ok(response) => {
                self.provider_dialog
                    .apply_resolve_response(response.clone());
                let draft = match response.draft.mode {
                    crate::api::ProviderConnectDraftMode::Known => response.draft,
                    crate::api::ProviderConnectDraftMode::Custom => response.custom_draft,
                };
                self.provider_dialog.start_custom_flow_with_prefill(
                    draft.provider_id,
                    draft.base_url.unwrap_or_default(),
                    draft.protocol.unwrap_or_else(|| "openai".to_string()),
                );
            }
            Err(error) => self.provider_dialog.set_resolve_error(error.to_string()),
        }
    }

    /// Submit an API key for a provider and update the dialog state.
    pub(super) fn submit_provider_auth(&mut self, provider_id: &str, api_key: &str) {
        use crate::components::SubmitResult;
        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_submit_result(SubmitResult::Failed("No API connection".to_string()));
            return;
        };
        match client.set_auth(provider_id, api_key) {
            Ok(()) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Success);
                self.toast.show(
                    crate::components::ToastVariant::Success,
                    &format!("Connected to {}", provider_id),
                    3000,
                );
                self.populate_provider_dialog();
                // Also refresh the model list so the new provider's models appear
                self.refresh_model_dialog();
            }
            Err(e) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Failed(e.to_string()));
            }
        }
    }

    /// Submit a custom provider registration and update the dialog state.
    pub(super) fn submit_custom_provider_auth(
        &mut self,
        provider_id: &str,
        base_url: &str,
        protocol: &str,
        api_key: &str,
    ) {
        use crate::components::SubmitResult;
        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_submit_result(SubmitResult::Failed("No API connection".to_string()));
            return;
        };
        match client.register_custom_provider(provider_id, base_url, protocol, api_key) {
            Ok(()) => {
                self.provider_dialog.exit_custom_flow();
                self.provider_dialog
                    .set_submit_result(SubmitResult::Success);
                self.toast.show(
                    crate::components::ToastVariant::Success,
                    &format!("Connected to {}", provider_id),
                    3000,
                );
                self.populate_provider_dialog();
                // Also refresh the model list so the new provider's models appear
                self.refresh_model_dialog();
            }
            Err(e) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Failed(e.to_string()));
            }
        }
    }

    pub(super) fn submit_provider_model_override(
        &mut self,
        provider_id: &str,
        model_key: &str,
        model: &agendao_config::ModelConfig,
    ) {
        use crate::components::SubmitResult;
        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_submit_result(SubmitResult::Failed("No API connection".to_string()));
            return;
        };
        match client.put_provider_model_config(provider_id, model_key, model) {
            Ok(_) => {
                self.provider_dialog.exit_model_override_flow();
                self.provider_dialog
                    .set_submit_result(SubmitResult::Success);
                self.toast.show(
                    crate::components::ToastVariant::Success,
                    &format!("Saved model override {}/{}", provider_id, model_key),
                    3000,
                );
                self.populate_provider_dialog();
                self.refresh_model_dialog();
            }
            Err(error) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Failed(error.to_string()));
            }
        }
    }

    pub(super) fn delete_selected_provider_model_override(&mut self) {
        use crate::components::SubmitResult;
        let Some(client) = self.context.get_api_client() else {
            self.provider_dialog
                .set_submit_result(SubmitResult::Failed("No API connection".to_string()));
            return;
        };
        let Some(selected) = self.provider_dialog.selected_model_override() else {
            return;
        };
        match client.delete_provider_model_config(&selected.provider_id, &selected.model_key) {
            Ok(_) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Success);
                self.toast.show(
                    crate::components::ToastVariant::Success,
                    &format!(
                        "Removed model override {}/{}",
                        selected.provider_id, selected.model_key
                    ),
                    3000,
                );
                self.populate_provider_dialog();
                self.refresh_model_dialog();
            }
            Err(error) => {
                self.provider_dialog
                    .set_submit_result(SubmitResult::Failed(error.to_string()));
            }
        }
    }

    pub(super) fn sync_prompt_spinner_style(&mut self) {
        let theme = self.context.theme.read().clone();
        let mode_name = current_mode_label(&self.context).unwrap_or_default();
        self.prompt
            .set_spinner_color(agent_color_from_name(&theme, &mode_name));
    }

    pub(super) fn sync_prompt_spinner_state(&mut self) -> bool {
        let before_active = self.prompt.spinner_active();
        let before_kind = self.prompt.spinner_task_kind();

        let Route::Session { session_id } = self.context.current_route() else {
            self.prompt.set_spinner_active(false);
            self.prompt.clear_interrupt_confirmation();
            return before_active != self.prompt.spinner_active()
                || before_kind != self.prompt.spinner_task_kind();
        };

        let status = {
            let session_ctx = self.context.session.read();
            session_ctx.status(&session_id).clone()
        };
        let is_active = self.session_status_requires_spinner(&session_id, &status);
        self.prompt.set_spinner_active(is_active);
        if !is_active {
            self.prompt.clear_interrupt_confirmation();
            return before_active != self.prompt.spinner_active()
                || before_kind != self.prompt.spinner_task_kind();
        }

        let task_kind = self.infer_spinner_task_kind(&session_id, &status);
        if self.prompt.spinner_task_kind() != task_kind {
            self.prompt.set_spinner_task_kind(task_kind);
        }

        before_active != self.prompt.spinner_active()
            || before_kind != self.prompt.spinner_task_kind()
    }

    fn session_status_requires_spinner(&self, session_id: &str, status: &SessionStatus) -> bool {
        // Early exit: Idle status is authoritative
        if matches!(status, SessionStatus::Idle) {
            return false;
        }

        // Runtime is the canonical authority for session running state
        if let Some(runtime) = self.context.session_runtime() {
            if runtime.session_id == session_id {
                return runtime.pending_permission.is_some()
                    || runtime.pending_question.is_some()
                    || runtime.active_stage_id.is_some()
                    || runtime.active_stage_count > 0
                    || !runtime.active_tools.is_empty()
                    || !matches!(runtime.run_status, crate::api::SessionRunStatusKind::Idle);
            }
        }

        // Fallback: check pending UI-level permission/question
        if self.context.get_pending_permission().is_some() || self.context.has_pending_question() {
            return true;
        }

        // No spinner needed - runtime authority says idle
        false
    }

    pub(super) fn infer_spinner_task_kind(
        &self,
        session_id: &str,
        status: &SessionStatus,
    ) -> TaskKind {
        if matches!(
            status,
            SessionStatus::Retrying { .. } | SessionStatus::Reconnecting
        ) {
            return TaskKind::LlmResponse;
        }

        let session_ctx = self.context.session.read();
        let Some(messages) = session_ctx.messages.get(session_id) else {
            return TaskKind::LlmRequest;
        };
        let Some(last_message) = messages.last() else {
            return TaskKind::LlmRequest;
        };

        match last_message.role {
            MessageRole::User => TaskKind::LlmRequest,
            MessageRole::Assistant => infer_task_kind_from_message(last_message),
            MessageRole::System => TaskKind::LlmResponse,
            MessageRole::Tool => TaskKind::ToolCall,
        }
    }

    pub(super) fn matches_keybind(&self, keybind_name: &str, key: KeyEvent) -> bool {
        self.context
            .keybind
            .read()
            .match_key(keybind_name, key.code, key.modifiers)
    }

    pub(super) fn sync_command_palette_labels(&mut self) {
        let prefs = self.context.ui_preferences();
        self.command_palette
            .sync_visibility_labels(crate::components::VisibilityLabels {
                show_thinking: prefs.show_thinking,
                show_tool_details: prefs.show_tool_details,
                density: prefs.message_density,
                semantic_highlight: prefs.semantic_highlight,
                show_header: prefs.show_header,
                show_scrollbar: prefs.show_scrollbar,
                tips_hidden: prefs.tips_hidden,
            });
    }
}
