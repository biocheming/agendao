use axum::{
    extract::{Path, State},
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::path::Path as FsPath;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;

use crate::session_runtime::events::{
    broadcast_session_updated, send_sse_server_event, sse_output_block_hook, ServerEvent,
};
use crate::{ApiError, ServerState};
use rocode_agent::{AgentInfo, AgentRegistry};
use rocode_provider::ToolDefinition;
use rocode_session::{MessageRole as SessionMessageRole, Session};

use super::permission::request_permission;
use super::provider_diagnostics::attach_provider_diagnostic_from_error;
use super::session::{
    apply_scheduler_selection_session_metadata, resolve_prompt_request_config,
    resolved_session_directory, to_task_agent_info, PromptRequestSchedulerProfileSource,
    SendMessageRequest,
};
use super::tui::{request_question_answers_with_hook, QuestionEventHook};

pub(crate) async fn send_stream_error_event(
    state: &Arc<ServerState>,
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    session_id: &str,
    message_id: Option<String>,
    done: Option<bool>,
    error: String,
) {
    if let Some(event) = state
        .runtime_telemetry
        .record_session_error(session_id, message_id.as_deref(), done, &error)
        .await
    {
        send_sse_server_event(tx, &event).await;
    }
}

pub(crate) async fn send_stream_usage_event(
    state: &Arc<ServerState>,
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    session_id: &str,
    message_id: Option<String>,
    usage: rocode_session::SessionUsage,
) {
    if let Some(event) = state
        .runtime_telemetry
        .record_session_usage(session_id, message_id.as_deref(), usage)
        .await
    {
        send_sse_server_event(tx, &event).await;
    }
}

async fn emit_latest_assistant_usage(
    state: &Arc<ServerState>,
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    session_id: &str,
    session: &Session,
) {
    let latest_usage = session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, SessionMessageRole::Assistant))
        .and_then(|message| {
            message
                .usage
                .as_ref()
                .map(|usage| (message.id.clone(), usage.clone()))
        });

    let Some((message_id, usage)) = latest_usage else {
        return;
    };

    send_stream_usage_event(
        state,
        tx,
        session_id,
        Some(message_id),
        rocode_session::SessionUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_miss_tokens: usage.cache_miss_tokens,
            context_tokens: usage.context_tokens,
            total_cost: usage.total_cost,
        },
    )
    .await;
}

fn filtered_tool_definitions(
    mut tool_defs: Vec<ToolDefinition>,
    agent: Option<&AgentInfo>,
) -> Vec<ToolDefinition> {
    if let Some(agent) = agent {
        tool_defs.retain(|tool| agent.is_tool_allowed(&tool.name));
    }
    rocode_session::prioritize_tool_definitions(&mut tool_defs);
    tool_defs
}

fn stream_ingress_envelope(
    session_id: &str,
    text: &str,
    idempotency_key: Option<String>,
    ingress_source: Option<&str>,
) -> rocode_session::prompt::IngressTurnEnvelope {
    let now = chrono::Utc::now().timestamp_millis();
    let turn_id = idempotency_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("ingress_{}", value.trim()))
        .unwrap_or_else(|| format!("ingress_{}", uuid::Uuid::new_v4().simple()));
    let mut envelope = rocode_session::prompt::IngressTurnEnvelope::new_text(
        session_id.to_string(),
        rocode_session::prompt::normalize_ingress_source(ingress_source),
        turn_id,
        now,
        text.to_string(),
    );
    envelope.context_key = Some("stream".to_string());
    envelope.idempotency_key = idempotency_key.filter(|value| !value.trim().is_empty());
    envelope.stabilization.policy =
        rocode_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY.to_string();
    envelope
}

pub(crate) async fn stream_message(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    if req.agent.is_some() && req.scheduler_profile.is_some() {
        return Err(ApiError::BadRequest(
            "`agent` and `scheduler_profile` are mutually exclusive".to_string(),
        ));
    }

    let request_variant = {
        let sessions = state.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
        req.variant.clone().or_else(|| {
            session
                .metadata
                .get("model_variant")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
    };

    let config = state.config_store.config();
    let known_agents = AgentRegistry::from_config(&config)
        .list_all()
        .into_iter()
        .map(|agent| agent.name.clone())
        .collect::<Vec<_>>();
    let request_config = resolve_prompt_request_config(super::session::PromptRequestConfigInput {
        state: &state,
        config: &config,
        session_id: &session_id,
        requested_agent: req.agent.as_deref(),
        requested_scheduler_profile: req.scheduler_profile.as_deref(),
        requested_scheduler_profile_source: req
            .scheduler_profile
            .as_deref()
            .map(|_| PromptRequestSchedulerProfileSource::ExplicitRequest),
        scheduler_profile_override: None,
        request_model: req.model.as_deref(),
        request_variant: request_variant.as_deref(),
        route: "stream",
    })
    .await?;
    let scheduler_applied = request_config.scheduler_applied;
    let scheduler_profile_name = request_config.scheduler_profile_name.clone();
    let scheduler_root_agent = request_config.scheduler_root_agent.clone();
    let scheduler_skill_tree_applied = request_config.scheduler_skill_tree_applied;
    let resolved_agent = request_config.resolved_agent.clone();
    let provider = request_config.provider.clone();
    let provider_id = request_config.provider_id.clone();
    let model_id = request_config.model_id.clone();
    let agent_system_prompt = request_config.agent_system_prompt.clone();
    let compiled_request = request_config.compiled_request.clone();

    let (selected_variant, stream_session) = {
        let mut sessions = state.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

        let normalized_directory =
            resolved_session_directory(session.record().directory.as_str(), &state.project_root());
        if session.record().directory != normalized_directory {
            session.set_directory(normalized_directory);
        }

        let selected_variant = request_variant.clone();
        if let Some(variant) = selected_variant.as_deref() {
            session.insert_metadata("model_variant", serde_json::json!(variant));
        } else {
            session.remove_metadata("model_variant");
        }
        session.insert_metadata("model_provider", serde_json::json!(provider_id.clone()));
        session.insert_metadata("model_id", serde_json::json!(model_id.clone()));
        if let Some(agent) = resolved_agent.as_ref().map(|agent| agent.name.as_str()) {
            session.insert_metadata("agent", serde_json::json!(agent));
        } else {
            session.remove_metadata("agent");
        }
        session.insert_metadata("scheduler_applied", serde_json::json!(scheduler_applied));
        session.insert_metadata(
            "scheduler_skill_tree_applied",
            serde_json::json!(scheduler_skill_tree_applied),
        );
        if let Some(profile) = scheduler_profile_name.as_deref() {
            session.insert_metadata("scheduler_profile", serde_json::json!(profile));
        } else {
            session.remove_metadata("scheduler_profile");
        }
        if let Some(root_agent) = scheduler_root_agent.as_deref() {
            session.insert_metadata("scheduler_root_agent", serde_json::json!(root_agent));
        } else {
            session.remove_metadata("scheduler_root_agent");
        }
        apply_scheduler_selection_session_metadata(session, &request_config);
        session.touch();

        (selected_variant, session.clone())
    };

    broadcast_session_updated(state.as_ref(), session_id.clone(), "stream.request");

    let resolved_tool_surface =
        rocode_session::resolve_tool_surface(state.tool_registry.as_ref()).await;
    let tool_defs = filtered_tool_definitions(resolved_tool_surface.tools, resolved_agent.as_ref());
    let tool_source_digests = if resolved_agent.is_some() {
        Vec::new()
    } else {
        resolved_tool_surface.source_digests
    };

    let (tx, rx) = mpsc::channel::<std::result::Result<Event, Infallible>>(128);
    let stream_state = state.clone();
    let stream_session_id = session_id.clone();
    let stream_config = config.clone();
    let stream_session = stream_session.clone();
    let stream_ingress = stream_ingress_envelope(
        &stream_session_id,
        &req.content,
        req.idempotency_key.clone(),
        req.ingress_source.as_deref(),
    );
    let stream_parts = if let Some(parts) = req.parts.clone().filter(|parts| !parts.is_empty()) {
        parts
    } else {
        rocode_session::resolve_prompt_parts(
            &req.content,
            FsPath::new(stream_session.record().directory.as_str()),
            &known_agents,
        )
        .await
    };
    let stream_variant = selected_variant.clone();
    let stream_provider = provider.clone();
    let stream_provider_id = provider_id.clone();
    let stream_model_id = model_id.clone();
    let stream_agent = resolved_agent.clone();
    let stream_system_prompt = agent_system_prompt.clone();

    tokio::spawn(async move {
        let stream_tx = tx;
        let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel::<Session>();
        let update_state = stream_state.clone();
        let mut update_task = tokio::spawn(async move {
            while let Some(snapshot) = update_rx.recv().await {
                {
                    let mut sessions = update_state.sessions.lock().await;
                    sessions.update(snapshot.clone());
                }
            }
        });

        let update_hook_tx = update_tx.clone();
        let update_hook: rocode_session::SessionUpdateHook = Arc::new(move |snapshot| {
            let _ = update_hook_tx.send(snapshot.clone());
        });

        let prompt_runner = rocode_session::SessionPrompt::new(Arc::new(RwLock::new(
            rocode_session::SessionStateManager::new(),
        )))
        .with_config_store(stream_state.config_store.clone())
        .with_tool_runtime_config(rocode_tool::ToolRuntimeConfig::from_config(&stream_config));
        let mut session = stream_session;
        super::permission::PERMISSION_ENGINE
            .lock()
            .await
            .clear_turn(&stream_session_id);
        let input = rocode_session::PromptInput {
            session_id: stream_session_id.clone(),
            message_id: None,
            model: Some(rocode_session::prompt::ModelRef {
                provider_id: stream_provider_id.clone(),
                model_id: stream_model_id.clone(),
            }),
            agent: stream_agent.as_ref().map(|agent| agent.name.clone()),
            no_reply: false,
            system: None,
            variant: stream_variant.clone(),
            parts: stream_parts.clone(),
            tools: None,
            ingress: Some(stream_ingress.clone()),
        };

        let agent_registry = AgentRegistry::from_config(&stream_config);
        let agent_lookup: Option<rocode_session::prompt::AgentLookup> =
            Some(Arc::new(move |name: &str| {
                agent_registry.get(name).map(to_task_agent_info)
            }));

        let ask_question_hook: Option<rocode_session::prompt::AskQuestionHook> = {
            let state = stream_state.clone();
            let sse_tx = stream_tx.clone();
            Some(Arc::new(move |session_id, questions| {
                let state = state.clone();
                let sse_tx = sse_tx.clone();
                let event_hook: QuestionEventHook = Arc::new(move |payload| {
                    let sse_tx = sse_tx.clone();
                    let event_name = payload
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("question.event")
                        .to_string();
                    tokio::spawn(async move {
                        if let Ok(event) = Event::default().event(&event_name).json_data(payload) {
                            if let Err(error) = sse_tx.send(Ok(event)).await {
                                tracing::debug!(
                                    event_name,
                                    error = %error,
                                    "Failed to forward question SSE event to stream client"
                                );
                            }
                        }
                    });
                });
                Box::pin(async move {
                    request_question_answers_with_hook(
                        state,
                        session_id,
                        questions,
                        Some(event_hook),
                    )
                    .await
                })
            }))
        };
        let ask_permission_hook: Option<rocode_session::prompt::AskPermissionHook> = {
            let state = stream_state.clone();
            Some(Arc::new(move |session_id, request| {
                let state = state.clone();
                Box::pin(async move { request_permission(state, session_id, request).await })
            }))
        };

        let event_broadcast: Option<rocode_session::prompt::EventBroadcastHook> = {
            let state = stream_state.clone();
            Some(Arc::new(move |event| {
                if let Ok(server_event) = serde_json::from_value::<ServerEvent>(event) {
                    if let Some(payload) = server_event.to_json_string() {
                        state.broadcast(&payload);
                    } else {
                        tracing::warn!(
                            "failed to serialize ServerEvent from stream event_broadcast"
                        );
                    }
                } else {
                    tracing::warn!("ignored non-ServerEvent payload in stream event_broadcast");
                }
            }))
        };
        let output_block_hook: Option<rocode_session::prompt::OutputBlockHook> =
            { Some(sse_output_block_hook(stream_tx.clone())) };

        if let Err(error) = prompt_runner
            .prompt_with_update_hook(
                input,
                &mut session,
                rocode_session::prompt::PromptRequestContext {
                    provider: stream_provider,
                    system_prompt: stream_system_prompt.clone(),
                    memory_prefetch: None,
                    tools: tool_defs,
                    tool_source_digests,
                    compiled_request: compiled_request.clone(),
                    hooks: rocode_session::prompt::PromptHooks {
                        update_hook: Some(update_hook),
                        event_broadcast,
                        compaction_lifecycle_hook: None,
                        output_block_hook,
                        agent_lookup,
                        ask_question_hook,
                        ask_permission_hook,
                        publish_bus_hook: None,
                        steering_boundary_hook: None,
                    },
                },
            )
            .await
        {
            tracing::warn!(
                session_id = %stream_session_id,
                provider_id = %stream_provider_id,
                model_id = %stream_model_id,
                %error,
                "web stream prompt failed"
            );
            let assistant = session.add_assistant_message();
            assistant.finish = Some("error".to_string());
            assistant
                .metadata
                .insert("error".to_string(), serde_json::json!(error.to_string()));
            assistant
                .metadata
                .insert("finish_reason".to_string(), serde_json::json!("error"));
            assistant.metadata.insert(
                "model_provider".to_string(),
                serde_json::json!(&stream_provider_id),
            );
            assistant
                .metadata
                .insert("model_id".to_string(), serde_json::json!(&stream_model_id));
            if let Some(agent) = stream_agent.as_ref().map(|agent| agent.name.as_str()) {
                assistant
                    .metadata
                    .insert("agent".to_string(), serde_json::json!(agent));
            }
            attach_provider_diagnostic_from_error(
                assistant,
                &error,
                &stream_provider_id,
                Some(&stream_model_id),
            );
            assistant.add_text(format!("Provider error: {}", error));
            let _ = update_tx.send(session.clone());
            let message_id = session
                .messages
                .iter()
                .rev()
                .find(|message| matches!(message.role, SessionMessageRole::Assistant))
                .map(|message| message.id.clone());
            send_stream_error_event(
                &stream_state,
                &stream_tx,
                &stream_session_id,
                message_id,
                Some(true),
                error.to_string(),
            )
            .await;
        }

        drop(update_tx);
        match tokio::time::timeout(std::time::Duration::from_secs(1), &mut update_task).await {
            Ok(joined) => {
                let _ = joined;
            }
            Err(_) => {
                update_task.abort();
                tracing::warn!(
                    session_id = %stream_session_id,
                    "timed out waiting for web stream update task shutdown; aborted task"
                );
            }
        }

        {
            let mut sessions = stream_state.sessions.lock().await;
            sessions.update(session.clone());
        }
        emit_latest_assistant_usage(&stream_state, &stream_tx, &stream_session_id, &session).await;
        broadcast_session_updated(
            stream_state.as_ref(),
            stream_session_id.clone(),
            "stream.final",
        );

        if let Err(err) = stream_state
            .flush_session_to_storage(&stream_session_id)
            .await
        {
            tracing::error!(
                session_id = %stream_session_id,
                %err,
                "failed to flush session to storage after stream"
            );
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)))
}

#[cfg(test)]
mod tests {
    use crate::session_runtime::scheduler_stage_block_from_message;
    use chrono::Utc;
    use rocode_command::governance_fixtures::canonical_scheduler_stage_fixture;
    use rocode_session::prompt::IngressSource;
    use rocode_session::{MessagePart, MessageRole, PartType, SessionMessage};

    use super::stream_ingress_envelope;

    #[test]
    fn scheduler_stage_message_projects_canonical_governance_block() {
        let fixture = canonical_scheduler_stage_fixture();
        let message = SessionMessage {
            id: "msg-stage".to_string(),
            session_id: "session-1".to_string(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart {
                id: "part-text".to_string(),
                part_type: PartType::Text {
                    text: fixture.message_text.clone(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: Utc::now(),
                message_id: Some("msg-stage".to_string()),
            }],
            created_at: Utc::now(),
            metadata: fixture.metadata,
            usage: None,
            finish: None,
        };

        let block = scheduler_stage_block_from_message(&message).expect("scheduler stage block");
        assert_eq!(block, fixture.block);
    }

    #[test]
    fn scheduler_stage_message_falls_back_to_metadata_title_when_heading_missing() {
        let fixture = canonical_scheduler_stage_fixture();
        let message = SessionMessage {
            id: "msg-stage".to_string(),
            session_id: "session-1".to_string(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart {
                id: "part-text".to_string(),
                part_type: PartType::Text {
                    text: fixture.block.text.clone(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: Utc::now(),
                message_id: Some("msg-stage".to_string()),
            }],
            created_at: Utc::now(),
            metadata: fixture.metadata,
            usage: None,
            finish: None,
        };

        let block = scheduler_stage_block_from_message(&message).expect("scheduler stage block");
        assert_eq!(block.title, "Atlas · Coordination Gate");
        assert_eq!(
            block.text,
            "Decision pending on the unresolved task ledger."
        );
    }

    #[test]
    fn stream_ingress_source_defaults_to_api_and_preserves_known_sources() {
        let defaulted = stream_ingress_envelope("ses_1", "hello", None, None);
        assert_eq!(defaulted.source, IngressSource::Api);
        assert_eq!(defaulted.context_key.as_deref(), Some("stream"));
        assert_eq!(
            defaulted.stabilization.policy,
            rocode_session::prompt::INGRESS_POLICY_ENTRY_METADATA_ONLY
        );

        let blank = stream_ingress_envelope("ses_1", "hello", None, Some(""));
        assert_eq!(blank.source, IngressSource::Api);

        assert_eq!(
            stream_ingress_envelope("ses_1", "hello", None, Some("cli")).source,
            IngressSource::Cli
        );
        assert_eq!(
            stream_ingress_envelope("ses_1", "hello", None, Some("TUI")).source,
            IngressSource::Tui
        );
        assert_eq!(
            stream_ingress_envelope("ses_1", "hello", None, Some("web")).source,
            IngressSource::Web
        );
        assert_eq!(
            stream_ingress_envelope("ses_1", "hello", None, Some("scheduler")).source,
            IngressSource::Scheduler
        );
        assert_eq!(
            stream_ingress_envelope("ses_1", "hello", None, Some("feishu")).source,
            IngressSource::Other("feishu".to_string())
        );
    }
}
