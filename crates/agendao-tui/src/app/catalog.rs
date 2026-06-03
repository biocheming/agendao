use super::*;

impl App {
    pub(super) fn submit_skill_guard_run(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(skill_name) = self
            .skill_list_dialog
            .selected_skill()
            .map(|value| value.to_string())
        else {
            anyhow::bail!("Select a skill before running guard.");
        };
        let response = client.run_skill_hub_guard(&crate::api::SkillHubGuardRunRequest {
            skill_name: Some(skill_name.clone()),
            source: None,
        })?;
        let report_count = response.reports.len();
        let violation_count = response
            .reports
            .iter()
            .map(|report| report.violations.len())
            .sum::<usize>();
        self.skill_list_dialog
            .set_guard_reports(format!("skill `{}`", skill_name), response.reports);
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Guard scanned skill {} ({} report{}, {} total violations).",
            skill_name,
            report_count,
            if report_count == 1 { "" } else { "s" },
            violation_count
        ))
    }

    pub(super) fn submit_skill_source_guard_run(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let response = client.run_skill_hub_guard(&crate::api::SkillHubGuardRunRequest {
            skill_name: None,
            source: Some(source.clone()),
        })?;
        let report_count = response.reports.len();
        let violation_count = response
            .reports
            .iter()
            .map(|report| report.violations.len())
            .sum::<usize>();
        self.skill_list_dialog
            .set_guard_reports(format!("source `{}`", source.source_id), response.reports);
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Guard scanned source {} ({} report{}, {} total violations).",
            source.source_id,
            report_count,
            if report_count == 1 { "" } else { "s" },
            violation_count
        ))
    }

    pub(super) fn submit_skill_hub_index_refresh(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let response =
            client.refresh_skill_hub_index(&crate::api::SkillHubIndexRefreshRequest { source })?;
        let source_id = response.snapshot.source.source_id.clone();
        let skill_count = response.snapshot.entries.len();
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Refreshed source index for {} ({} entries).",
            source_id, skill_count
        ))
    }

    pub(super) fn refresh_skill_hub_state(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let managed = client.list_skill_hub_managed()?;
        let index = client.list_skill_hub_index()?;
        let distributions = client.list_skill_hub_distributions()?;
        let artifact_cache = client.list_skill_hub_artifact_cache()?;
        let policy = client.list_skill_hub_policy()?;
        let lifecycle = client.list_skill_hub_lifecycle()?;
        let audit = client.list_skill_hub_audit()?;
        let timeline = client.list_skill_hub_timeline(&crate::api::SkillHubTimelineQuery {
            skill_name: None,
            source_id: None,
            limit: Some(120),
        })?;
        self.skill_list_dialog.set_hub_state(
            managed.managed_skills,
            index.source_indices,
            audit.audit_events,
        );
        self.skill_list_dialog.set_remote_hub_state(
            distributions.distributions,
            artifact_cache.artifact_cache,
            policy.policy,
            lifecycle.lifecycle,
        );
        self.skill_list_dialog
            .set_governance_timeline(timeline.entries);
        Ok(())
    }

    pub(super) fn submit_skill_hub_sync_plan(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let response =
            client.plan_skill_hub_sync(&crate::api::SkillHubSyncPlanRequest { source })?;
        let entry_count = response.plan.entries.len();
        let source_id = response.plan.source_id.clone();
        self.skill_list_dialog.set_hub_plan(response.plan);
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Built hub sync plan for {} ({} entries).",
            source_id, entry_count
        ))
    }

    pub(super) fn submit_skill_hub_sync_apply(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before applying skill hub sync.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let response = client
            .apply_skill_hub_sync(&crate::api::SkillHubSyncApplyRequest { session_id, source })?;
        let entry_count = response.plan.entries.len();
        let source_id = response.plan.source_id.clone();
        let guard_count = response.guard_reports.len();
        self.skill_list_dialog.set_hub_plan(response.plan);
        self.refresh_skill_list_dialog()?;
        Ok(if guard_count > 0 {
            format!(
                "Applied hub sync for {} ({} planned entries, {} guard warnings).",
                source_id, entry_count, guard_count
            )
        } else {
            format!(
                "Applied hub sync for {} ({} planned entries).",
                source_id, entry_count
            )
        })
    }

    pub(super) fn submit_skill_hub_remote_install_plan(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response = client.plan_skill_hub_remote_install(
            &crate::api::SkillHubRemoteInstallPlanRequest {
                source,
                skill_name: skill_name.clone(),
            },
        )?;
        let action = response.entry.action.clone();
        self.skill_list_dialog.set_remote_install_plan(response);
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Built remote install plan for {} ({:?}).",
            skill_name, action
        ))
    }

    pub(super) fn submit_skill_hub_remote_install_apply(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before applying remote installs.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response = client.apply_skill_hub_remote_install(
            &crate::api::SkillHubRemoteInstallApplyRequest {
                session_id,
                source,
                skill_name: skill_name.clone(),
            },
        )?;
        let action = response.plan.entry.action.clone();
        let guard_warnings = response
            .guard_report
            .as_ref()
            .map(|report| report.violations.len())
            .unwrap_or_default();
        self.skill_list_dialog
            .set_remote_install_plan(response.plan);
        self.refresh_skill_list_dialog()?;
        Ok(if guard_warnings > 0 {
            format!(
                "Applied remote {:?} for {} ({} guard warnings).",
                action, skill_name, guard_warnings
            )
        } else {
            format!("Applied remote {:?} for {}.", action, skill_name)
        })
    }

    pub(super) fn submit_skill_hub_remote_update_plan(&mut self) -> anyhow::Result<String> {
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response =
            client.plan_skill_hub_remote_update(&crate::api::SkillHubRemoteUpdatePlanRequest {
                source,
                skill_name: skill_name.clone(),
            })?;
        let action = response.entry.action.clone();
        self.skill_list_dialog.set_remote_install_plan(response);
        self.refresh_skill_hub_state()?;
        Ok(format!(
            "Built remote update plan for {} ({:?}).",
            skill_name, action
        ))
    }

    pub(super) fn submit_skill_hub_remote_update_apply(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before applying remote updates.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response = client.apply_skill_hub_remote_update(
            &crate::api::SkillHubRemoteUpdateApplyRequest {
                session_id,
                source,
                skill_name: skill_name.clone(),
            },
        )?;
        let guard_warnings = response
            .guard_report
            .as_ref()
            .map(|report| report.violations.len())
            .unwrap_or_default();
        self.skill_list_dialog
            .set_remote_install_plan(response.plan.clone());
        self.refresh_skill_list_dialog()?;
        Ok(if guard_warnings > 0 {
            format!(
                "Applied remote {:?} for {} ({} guard warnings).",
                response.plan.entry.action, skill_name, guard_warnings
            )
        } else {
            format!(
                "Applied remote {:?} for {}.",
                response.plan.entry.action, skill_name
            )
        })
    }

    pub(super) fn submit_skill_hub_managed_detach(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before detaching managed skills.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response =
            client.detach_skill_hub_managed(&crate::api::SkillHubManagedDetachRequest {
                session_id,
                source,
                skill_name: skill_name.clone(),
            })?;
        self.refresh_skill_list_dialog()?;
        Ok(format!(
            "Detached managed skill {} ({:?}).",
            skill_name, response.lifecycle.state
        ))
    }

    pub(super) fn submit_skill_hub_managed_remove(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before removing managed skills.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(source) = self.skill_list_dialog.selected_hub_source().cloned() else {
            anyhow::bail!("No indexed skill hub source is available yet.");
        };
        let Some(skill_name) = self.skill_list_dialog.resolve_remote_install_skill_name() else {
            anyhow::bail!(
                "No remote skill candidate resolved. Select a source and type a matching query or select an installed skill with the same name."
            );
        };
        let response =
            client.remove_skill_hub_managed(&crate::api::SkillHubManagedRemoveRequest {
                session_id,
                source,
                skill_name: skill_name.clone(),
            })?;
        self.refresh_skill_list_dialog()?;
        Ok(if response.deleted_from_workspace {
            format!(
                "Removed managed skill {} and deleted workspace copy.",
                skill_name
            )
        } else {
            format!(
                "Removed managed skill {} without deleting workspace copy.",
                skill_name
            )
        })
    }

    pub(super) fn submit_skill_create(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before managing skills.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(payload) = self.skill_list_dialog.create_payload() else {
            anyhow::bail!("Skill create draft is not active.");
        };
        let payload = payload.map_err(anyhow::Error::msg)?;

        let response = match payload {
            crate::components::SkillCreatePayload::Raw {
                name,
                description,
                category,
                body,
            } => {
                if name.trim().is_empty() || description.trim().is_empty() || body.trim().is_empty()
                {
                    anyhow::bail!("Name, description, and body are required.");
                }
                client.manage_skill(&crate::api::SkillManageRequest {
                    session_id,
                    action: crate::api::SkillManageAction::Create,
                    name: Some(name.clone()),
                    new_name: None,
                    description: Some(description),
                    body: Some(body),
                    methodology: None,
                    content: None,
                    category,
                    directory_name: None,
                    file_path: None,
                })?
            }
            crate::components::SkillCreatePayload::Methodology {
                name,
                description,
                category,
                methodology,
            } => {
                if name.trim().is_empty() || description.trim().is_empty() {
                    anyhow::bail!("Name and description are required.");
                }
                client.manage_skill(&crate::api::SkillManageRequest {
                    session_id,
                    action: crate::api::SkillManageAction::Create,
                    name: Some(name.clone()),
                    new_name: None,
                    description: Some(description),
                    body: None,
                    methodology: Some(methodology),
                    content: None,
                    category,
                    directory_name: None,
                    file_path: None,
                })?
            }
        };
        self.skill_list_dialog.cancel_manage_mode();
        self.refresh_skill_list_dialog()?;
        Ok(skill_manage_message(
            format!("Created skill {}.", response.result.skill_name),
            response.guard_report.as_ref(),
        ))
    }

    pub(super) fn submit_skill_edit(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before managing skills.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(payload) = self.skill_list_dialog.edit_payload() else {
            anyhow::bail!("Skill edit draft is not active.");
        };
        let payload = payload.map_err(anyhow::Error::msg)?;

        let response = match payload {
            crate::components::SkillEditPayload::Raw { name, content } => {
                if content.trim().is_empty() {
                    anyhow::bail!("Skill source cannot be empty.");
                }
                client.manage_skill(&crate::api::SkillManageRequest {
                    session_id,
                    action: crate::api::SkillManageAction::Edit,
                    name: Some(name.clone()),
                    new_name: None,
                    description: None,
                    body: None,
                    methodology: None,
                    content: Some(content),
                    category: None,
                    directory_name: None,
                    file_path: None,
                })?
            }
            crate::components::SkillEditPayload::Methodology {
                name,
                description,
                methodology,
            } => {
                if description.trim().is_empty() {
                    anyhow::bail!("Description is required for methodology patch mode.");
                }
                client.manage_skill(&crate::api::SkillManageRequest {
                    session_id,
                    action: crate::api::SkillManageAction::Patch,
                    name: Some(name.clone()),
                    new_name: None,
                    description: Some(description),
                    body: None,
                    methodology: Some(methodology),
                    content: None,
                    category: None,
                    directory_name: None,
                    file_path: None,
                })?
            }
        };
        self.skill_list_dialog.cancel_manage_mode();
        self.refresh_skill_list_dialog()?;
        Ok(skill_manage_message(
            format!("Saved skill {}.", response.result.skill_name),
            response.guard_report.as_ref(),
        ))
    }

    pub(super) fn submit_skill_delete(&mut self) -> anyhow::Result<String> {
        let Some(session_id) = self.current_session_id() else {
            anyhow::bail!("Select or create a session before managing skills.");
        };
        let Some(client) = self.context.get_api_client() else {
            anyhow::bail!("No active API client.");
        };
        let Some(name) = self.skill_list_dialog.delete_payload() else {
            anyhow::bail!("Skill delete confirmation is not active.");
        };

        let response = client.manage_skill(&crate::api::SkillManageRequest {
            session_id,
            action: crate::api::SkillManageAction::Delete,
            name: Some(name),
            new_name: None,
            description: None,
            body: None,
            methodology: None,
            content: None,
            category: None,
            directory_name: None,
            file_path: None,
        })?;
        self.skill_list_dialog.cancel_manage_mode();
        self.refresh_skill_list_dialog()?;
        Ok(format!("Deleted skill {}.", response.result.skill_name))
    }

    pub(super) fn refresh_skill_list_detail(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            self.skill_list_dialog.clear_detail();
            return Ok(());
        };
        let Some(skill_name) = self
            .skill_list_dialog
            .selected_skill()
            .map(|value| value.to_string())
        else {
            self.skill_list_dialog.clear_detail();
            return Ok(());
        };

        match client.get_skill_detail(&crate::api::SkillDetailQuery {
            name: skill_name.clone(),
            session_id: self.current_session_id(),
            ..Default::default()
        }) {
            Ok(detail) => {
                self.skill_list_dialog.set_skill_detail(detail);
                Ok(())
            }
            Err(error) => {
                self.skill_list_dialog
                    .set_skill_detail_error(format!("Failed to load `{}`: {}", skill_name, error));
                Err(error)
            }
        }
    }

    pub(super) fn refresh_model_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            self.context.set_has_connected_provider(false);
            return;
        };

        let Ok(providers) = client.get_all_providers() else {
            self.context.set_has_connected_provider(false);
            return;
        };
        let connected_provider_ids = providers.connected.iter().cloned().collect::<HashSet<_>>();
        let has_connected_provider = !connected_provider_ids.is_empty();
        self.context
            .set_has_connected_provider(has_connected_provider);

        let mut available_models = HashSet::new();
        let mut variant_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut models = Vec::new();
        let mut context_providers = Vec::new();
        for provider in providers.all {
            let provider_id = provider.id.clone();
            let provider_name = provider.name.clone();
            let mut provider_models = Vec::new();
            for model in provider.models {
                let model_id = model.id;
                let model_name = model.name;
                let model_context_window = model.context_window.unwrap_or(0);
                let model_ref = format!("{}/{}", provider_id, model_id);
                available_models.insert(model_ref.clone());
                let entry = variant_map.entry(model_ref).or_default();
                for variant in model.variants {
                    if !entry.iter().any(|value| value == &variant) {
                        entry.push(variant);
                    }
                }
                models.push(Model {
                    id: model_id.clone(),
                    name: model_name.clone(),
                    provider: provider_id.clone(),
                    context_window: model_context_window,
                });
                provider_models.push(crate::context::ModelInfo {
                    id: format!("{}/{}", provider_id, model_id),
                    name: model_name,
                    context_window: model_context_window,
                    max_output_tokens: model.max_output_tokens.unwrap_or(0),
                    supports_vision: false,
                    supports_tools: true,
                    cost_per_million_input: model.cost_per_million_input,
                    cost_per_million_output: model.cost_per_million_output,
                });
            }
            context_providers.push(crate::context::ProviderInfo {
                id: provider_id,
                name: provider_name,
                models: provider_models,
            });
        }
        *self.context.providers.write() = context_providers;
        models.sort_by(|a, b| {
            a.provider
                .cmp(&b.provider)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        for variants in variant_map.values_mut() {
            variants.sort();
        }
        self.model_select.set_models(models);
        let current_key = self.context.current_model().as_ref().map(|m| {
            let provider = self.context.current_provider();
            if let Some(ref p) = provider {
                format!("{}/{}", p, m)
            } else {
                m.clone()
            }
        });
        self.model_select.set_current_model(current_key);
        self.model_select
            .set_recent(self.context.load_recent_models());
        self.available_models = available_models;
        self.model_variants = variant_map;
        self.model_variant_selection.retain(|model_key, variant| {
            let Some(available) = self.model_variants.get(model_key) else {
                return false;
            };
            match variant {
                Some(value) => available.iter().any(|item| item == value),
                None => true,
            }
        });
        self.sync_current_model_variant();

        let model_missing = self.context.current_model().is_none();
        if model_missing {
            let restored = self
                .model_select
                .recent()
                .iter()
                .find_map(|(provider, model_id)| {
                    let key = format!("{}/{}", provider, model_id);
                    if self.available_models.contains(&key) {
                        Some((key, provider.clone()))
                    } else {
                        None
                    }
                });
            if let Some((model_ref, provider)) = restored {
                self.set_active_model_selection(model_ref, Some(provider));
            } else if let Some((provider, model_id)) = providers.default_model.iter().next() {
                self.set_active_model_selection(
                    format!("{}/{}", provider, model_id),
                    Some(provider.clone()),
                );
            }
        }
    }

    pub(super) fn refresh_agent_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            return;
        };

        let agents = match client.list_execution_modes() {
            Ok(agents) => agents,
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Failed to load modes: {}", error),
                    3000,
                );
                return;
            }
        };

        if agents.is_empty() {
            return;
        }

        let theme = self.context.theme.read().clone();
        let mut mode_names = Vec::new();
        let mapped = agents
            .into_iter()
            .filter(|mode| {
                if mode.kind == "agent" {
                    let is_subagent = mode.mode.as_deref() == Some("subagent");
                    let is_hidden = mode.hidden.unwrap_or(false);
                    !is_subagent && !is_hidden
                } else {
                    true
                }
            })
            .enumerate()
            .map(|(idx, mode)| {
                mode_names.push(mode.id.clone());
                map_execution_mode_to_dialog_option(&theme, idx, mode)
            })
            .collect::<Vec<_>>();
        self.agent_select.set_agents(mapped);
        let current = current_mode_label(&self.context).unwrap_or_default();
        if !current.trim().is_empty() && !mode_names.iter().any(|name| name == &current) {
            if let Some(first) = self.agent_select.agents().first() {
                apply_selected_mode(&self.context, first);
                self.sync_prompt_spinner_style();
            }
        }
        self.prompt.set_agent_suggestions(mode_names);
    }

    pub(super) fn cycle_agent(&mut self, direction: i8) {
        self.refresh_agent_dialog();

        let agents = self.agent_select.agents();
        if agents.is_empty() {
            return;
        }

        let current = current_mode_label(&self.context).unwrap_or_default();
        let current_index = agents
            .iter()
            .position(|agent| agent.name == current)
            .unwrap_or(0);

        let len = agents.len();
        let next_index = if direction >= 0 {
            (current_index + 1) % len
        } else if current_index == 0 {
            len - 1
        } else {
            current_index - 1
        };
        let next_agent = agents[next_index].clone();

        apply_selected_mode(&self.context, &next_agent);
        self.sync_prompt_spinner_style();
    }

    pub(super) fn refresh_session_list_dialog(&mut self) {
        let Some(client) = self.context.get_api_client() else {
            return;
        };

        let query = self.session_list_dialog.query().trim().to_string();
        let sessions_result = if query.is_empty() {
            client.list_sessions()
        } else {
            client.list_sessions_filtered(Some(&query), Some(30))
        };
        let Ok(sessions) = sessions_result else {
            return;
        };
        let status_map = client.get_session_status().unwrap_or_default();
        {
            let mut session_ctx = self.context.session.write();
            for session in &sessions {
                if let Some(status) = status_map.get(&session.id) {
                    session_ctx.set_status(&session.id, map_api_run_status(status));
                }
            }
        }

        let items = sessions
            .into_iter()
            .map(|session| SessionItem {
                is_busy: status_map.get(&session.id).map(|s| s.busy).unwrap_or(false),
                id: session.id,
                title: session.title,
                directory: session.directory,
                parent_id: session.parent_id,
                updated_at: session.time.updated,
            })
            .collect::<Vec<_>>();
        self.session_list_dialog.set_sessions(items);
    }

    pub(super) fn refresh_theme_list_dialog(&mut self) {
        let options = self
            .context
            .available_theme_names()
            .into_iter()
            .map(|name| ThemeOption {
                id: name.clone(),
                name: format_theme_option_label(&name),
            })
            .collect::<Vec<_>>();
        self.theme_list_dialog.set_options(options);
    }

    pub(super) fn refresh_skill_list_dialog(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            self.skill_list_dialog.clear_detail();
            return Ok(());
        };
        let query = crate::api::SkillCatalogQuery {
            session_id: self.current_session_id(),
            ..Default::default()
        };
        let skills = client.list_skills(Some(&query))?;
        let suggestions = skills.iter().map(|skill| skill.name.clone()).collect();
        self.skill_list_dialog.set_skills(skills);
        let _ = self.refresh_skill_hub_state();
        self.prompt.set_skill_suggestions(suggestions);
        let _ = self.refresh_skill_list_detail();
        Ok(())
    }

    pub(super) fn refresh_lsp_status(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let servers = client.get_lsp_servers()?;
        let statuses = servers
            .into_iter()
            .map(|id| crate::context::LspStatus {
                id,
                root: "-".to_string(),
                status: crate::context::LspConnectionStatus::Connected,
            })
            .collect::<Vec<_>>();
        *self.context.lsp_status.write() = statuses;
        Ok(())
    }

    pub(super) fn refresh_mcp_dialog(&mut self) -> anyhow::Result<()> {
        let Some(client) = self.context.get_api_client() else {
            return Ok(());
        };
        let servers = client.get_mcp_status()?;

        let mcp_items = servers
            .iter()
            .map(|server| McpItem {
                name: server.name.clone(),
                status: server.status.clone(),
                tools: server.tools,
                resources: server.resources,
                error: server.error.clone(),
            })
            .collect::<Vec<_>>();
        self.mcp_dialog.set_items(mcp_items);

        let statuses = servers
            .into_iter()
            .map(|server| {
                let status = map_mcp_status(&server);
                McpServerStatus {
                    name: server.name,
                    status,
                    error: server.error,
                }
            })
            .collect::<Vec<_>>();
        *self.context.mcp_servers.write() = statuses;
        Ok(())
    }
}

fn skill_manage_message(
    base: String,
    guard_report: Option<&crate::api::SkillGuardReport>,
) -> String {
    if let Some(report) = guard_report {
        return format!(
            "{} Guard: {:?} ({} violations).",
            base,
            report.status,
            report.violations.len()
        );
    }
    base
}
