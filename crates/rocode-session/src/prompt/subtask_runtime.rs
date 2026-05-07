use std::collections::HashMap;
use std::sync::Arc;

use rocode_provider::Provider;
use rocode_types::{SubsessionHandoffFieldKind, SubsessionHandoffPacket};

use crate::{MessageRole, PartType, Session, SessionMessage};

use super::{
    tool_execution, PersistedSubsession, PersistedSubsessionTurn, PromptHooks, SessionPrompt,
};

#[derive(Debug, Clone)]
pub(super) struct PendingSubtask {
    pub(super) part_index: usize,
    pub(super) subtask_id: String,
    pub(super) agent: String,
    pub(super) prompt: String,
    pub(super) description: String,
}

impl SessionPrompt {
    fn collect_pending_subtasks(message: &SessionMessage) -> Vec<PendingSubtask> {
        let metadata_by_id: HashMap<String, (String, String, String)> = message
            .metadata
            .get("pending_subtasks")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("id").and_then(|v| v.as_str())?.to_string();
                        let agent = item
                            .get("agent")
                            .and_then(|v| v.as_str())
                            .unwrap_or("general")
                            .to_string();
                        let prompt = item
                            .get("prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let description = item
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some((id, (agent, prompt, description)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        message
            .parts
            .iter()
            .enumerate()
            .filter_map(|(part_index, part)| match &part.part_type {
                PartType::Subtask {
                    id,
                    description,
                    status,
                } if status == "pending" => {
                    let (agent, prompt, meta_description) = metadata_by_id
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| (id.clone(), description.clone(), description.clone()));
                    let description = if meta_description.is_empty() {
                        description.clone()
                    } else {
                        meta_description
                    };
                    let prompt = if prompt.trim().is_empty() {
                        description.clone()
                    } else {
                        prompt
                    };
                    Some(PendingSubtask {
                        part_index,
                        subtask_id: id.clone(),
                        agent,
                        prompt,
                        description,
                    })
                }
                _ => None,
            })
            .collect()
    }

    pub(super) async fn process_pending_subtasks(
        &self,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        provider_id: &str,
        model_id: &str,
        hooks: &PromptHooks,
    ) -> anyhow::Result<bool> {
        let last_user_idx = session
            .messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User));
        let Some(last_user_idx) = last_user_idx else {
            return Ok(false);
        };

        let pending = Self::collect_pending_subtasks(&session.messages[last_user_idx]);
        if pending.is_empty() {
            return Ok(false);
        }

        let mut results: Vec<(usize, String, bool, String, String)> = Vec::new();
        let tool_registry = Arc::new(rocode_tool::create_default_registry().await);
        let mut persisted = Self::load_persisted_subsessions(session);
        let default_model = format!("{}:{}", provider_id, model_id);
        let user_text = session.messages[last_user_idx].get_text();

        for subtask in &pending {
            let mut handoff = SubsessionHandoffPacket::bounded_goal(subtask.prompt.clone());
            if !user_text.trim().is_empty() {
                handoff.push_titled_text(
                    SubsessionHandoffFieldKind::SupportingContext,
                    "parent request context",
                    user_text.clone(),
                );
            }
            let subsession_id = format!("task_subtask_{}", subtask.subtask_id);
            persisted
                .entry(subsession_id.clone())
                .or_insert_with(|| PersistedSubsession {
                    kind: rocode_types::SessionContextKind::DelegatedSubsession,
                    agent: subtask.agent.clone(),
                    model: Some(default_model.clone()),
                    directory: Some(session.directory.clone()),
                    disabled_tools: Vec::new(),
                    history: Vec::new(),
                });
            let state_snapshot =
                persisted
                    .get(&subsession_id)
                    .cloned()
                    .unwrap_or(PersistedSubsession {
                        kind: rocode_types::SessionContextKind::DelegatedSubsession,
                        agent: subtask.agent.clone(),
                        model: Some(default_model.clone()),
                        directory: Some(session.directory.clone()),
                        disabled_tools: Vec::new(),
                        history: Vec::new(),
                    });

            match Self::execute_persisted_subsession_prompt(
                &state_snapshot,
                &handoff,
                provider.clone(),
                tool_registry.clone(),
                tool_execution::PersistedSubsessionPromptOptions {
                    default_model: default_model.clone(),
                    fallback_directory: Some(session.directory.clone()),
                    hooks: PromptHooks {
                        agent_lookup: hooks.agent_lookup.clone(),
                        ask_question_hook: hooks.ask_question_hook.clone(),
                        ask_permission_hook: hooks.ask_permission_hook.clone(),
                        ..Default::default()
                    },
                    question_session_id: Some(session.id.clone()),
                    abort: None,
                    tool_runtime_config: self.tool_runtime_config.clone(),
                    config_store: self.config_store.clone(),
                },
            )
            .await
            {
                Ok(output) => {
                    if let Some(existing) = persisted.get_mut(&subsession_id) {
                        existing.history.push(PersistedSubsessionTurn {
                            handoff: Some(handoff),
                            result: Some(output.clone()),
                            prompt: None,
                            output: None,
                        });
                    }
                    results.push((
                        subtask.part_index,
                        subtask.subtask_id.clone(),
                        false,
                        subtask.description.clone(),
                        output.text,
                    ));
                }
                Err(error) => {
                    results.push((
                        subtask.part_index,
                        subtask.subtask_id.clone(),
                        true,
                        subtask.description.clone(),
                        error.to_string(),
                    ));
                }
            }
        }

        for (part_index, subtask_id, is_error, description, output) in results {
            if let Some(message) = session.messages_mut().get_mut(last_user_idx) {
                if let Some(part) = message.parts.get_mut(part_index) {
                    if let PartType::Subtask { status, .. } = &mut part.part_type {
                        *status = if is_error {
                            "error".to_string()
                        } else {
                            "completed".to_string()
                        };
                    }
                }
            }

            let assistant = session.add_assistant_message();
            assistant
                .metadata
                .insert("subtask_id".to_string(), serde_json::json!(subtask_id));
            assistant.metadata.insert(
                "subtask_status".to_string(),
                serde_json::json!(if is_error { "error" } else { "completed" }),
            );
            assistant.add_text(format!(
                "Subtask `{}` {}:\n{}",
                description,
                if is_error { "failed" } else { "completed" },
                output
            ));
        }

        Self::save_persisted_subsessions(session, &persisted);

        Ok(true)
    }
}
