use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

use agendao_agent::{AgentInfo, AgentRegistry};
use agendao_config::Config as AppConfig;
use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::agent_loop::{
    AgentObservationContext, InteractionClock, ToolBackend, ToolCall, ToolExecution,
};
use tokio_util::sync::CancellationToken;

use crate::request_options::{resolve_compiled_execution_request, ExecutionResolutionContext};
use crate::{Result, ServerState};
use agendao_session::{MessageRole, PartType as SessionPartType, SessionMessage};
use agendao_types::{
    message_latest_compaction_summary, MemoryDetailView, MemoryEvidenceRef, MemoryRecordId,
    SessionContinuityPacket,
};

use super::messages::resolve_provider_and_model;
use super::prompt::SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY;

const SCHEDULER_CONTEXT_HYDRATE_TOOL: &str = "scheduler_context_hydrate";
const SCHEDULER_MEMORY_HYDRATE_TOOL: &str = "scheduler_memory_hydrate";
const SCHEDULER_CONTEXT_HYDRATE_DEFAULT_MESSAGE_LIMIT: usize = 2_000;
const SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT: usize = 8_000;
const SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES: usize = 12;
const SCHEDULER_MEMORY_HYDRATE_DEFAULT_RECORD_LIMIT: usize = 4_000;
const SCHEDULER_MEMORY_HYDRATE_MAX_RECORD_LIMIT: usize = 12_000;
const SCHEDULER_MEMORY_HYDRATE_MAX_RECORDS: usize = 8;

pub(crate) struct PromptRequestConfigInput<'a> {
    pub state: &'a Arc<ServerState>,
    pub config: &'a AppConfig,
    pub session_id: &'a str,
    pub requested_agent: Option<&'a str>,
    pub requested_scheduler: &'a agendao_orchestrator::selector::SchedulerChoice,
    pub request_model: Option<&'a str>,
    pub request_variant: Option<&'a str>,
    pub route: &'static str,
}

#[derive(Debug)]
enum SchedulerToolError {
    InvalidArguments(String),
    Execution(String),
}

impl std::fmt::Display for SchedulerToolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArguments(message) => write!(formatter, "Invalid arguments: {message}"),
            Self::Execution(message) => write!(formatter, "Execution error: {message}"),
        }
    }
}

struct SchedulerToolOutput {
    output: String,
    title: Option<String>,
    metadata: Option<serde_json::Value>,
    is_error: bool,
}

#[derive(Clone)]
pub(crate) struct SessionSchedulerToolExecutor {
    pub(super) state: Arc<ServerState>,
    pub(super) session_id: String,
    pub(super) message_id: String,
    pub(super) directory: String,
    pub(super) abort_token: CancellationToken,
    pub(super) tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    pub(super) execution_metadata: std::collections::HashMap<String, serde_json::Value>,
    pub(super) capability_allowed_tools_by_agent: BTreeMap<String, Vec<String>>,
    pub(super) interaction_clock: InteractionClock,
}

pub(crate) struct SessionSchedulerToolExecutorInput {
    pub session_id: String,
    pub message_id: String,
    pub directory: String,
    pub abort_token: CancellationToken,
    pub tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    pub execution_metadata: std::collections::HashMap<String, serde_json::Value>,
    pub capability_allowed_tools_by_agent: BTreeMap<String, Vec<String>>,
    pub interaction_clock: InteractionClock,
}

pub(super) fn resolve_effective_scheduler_choice(
    command_scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    request_scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    has_explicit_agent: bool,
) -> agendao_orchestrator::selector::SchedulerChoice {
    command_scheduler.or(request_scheduler).unwrap_or({
        if has_explicit_agent {
            agendao_orchestrator::selector::SchedulerChoice::Template {
                template: agendao_orchestrator::templates::TemplateId::Direct,
            }
        } else {
            agendao_orchestrator::selector::SchedulerChoice::Auto
        }
    })
}

impl SessionSchedulerToolExecutor {
    pub(crate) fn new(state: Arc<ServerState>, input: SessionSchedulerToolExecutorInput) -> Self {
        Self {
            state,
            session_id: input.session_id,
            message_id: input.message_id,
            directory: input.directory,
            abort_token: input.abort_token,
            tool_runtime_config: input.tool_runtime_config,
            execution_metadata: input.execution_metadata,
            capability_allowed_tools_by_agent: input.capability_allowed_tools_by_agent,
            interaction_clock: input.interaction_clock,
        }
    }

    fn native_execution_metadata(
        &self,
        call_id: Option<&str>,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut metadata = self.execution_metadata.clone();
        if let Some(call_id) = call_id {
            metadata.insert("call_id".to_string(), serde_json::json!(call_id));
        }
        metadata
    }

    async fn build_tool_context(
        &self,
        agent_name: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> agendao_tool::ToolContext {
        let sandbox_authority = self
            .state
            .sandbox_authority_for_session(&self.session_id)
            .await;
        let native_allowed =
            crate::ServerState::sandbox_native_allowed_for_mode(sandbox_authority.session_mode());
        let mut base_ctx = agendao_tool::ToolContext::new(
            self.session_id.clone(),
            self.message_id.clone(),
            self.directory.clone(),
        )
        .with_agent(agent_name.to_string())
        .with_abort(self.abort_token.clone())
        .with_config_store(self.state.config_store.clone())
        .with_tool_runtime_config(self.tool_runtime_config.clone())
        .with_registry(self.state.tool_registry.clone())
        // The authority and native hint are derived from one immutable
        // session-mode snapshot for this launch.
        .with_sandbox_execution_boundary(sandbox_authority)
        .with_sandbox_native_allowed(native_allowed)
        .with_ask_question({
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let abort = self.abort_token.clone();
            let interaction_clock = self.interaction_clock.clone();
            move |questions| {
                let state = state.clone();
                let session_id = session_id.clone();
                let abort = abort.clone();
                let interaction_clock = interaction_clock.clone();
                async move {
                    let _pause = interaction_clock.pause();
                    super::super::tui::request_question_answers_with_abort(
                        state, session_id, questions, abort,
                    )
                    .await
                }
            }
        })
        .with_ask({
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let abort = self.abort_token.clone();
            let interaction_clock = self.interaction_clock.clone();
            move |request| {
                let state = state.clone();
                let session_id = session_id.clone();
                let abort = abort.clone();
                let interaction_clock = interaction_clock.clone();
                async move {
                    let _pause = interaction_clock.pause();
                    super::super::permission::request_permission_with_abort(
                        state, session_id, request, abort,
                    )
                    .await
                }
            }
        })
        .with_todo_update({
            let state = self.state.clone();
            move |session_id, todos| {
                let state = state.clone();
                async move {
                    let todos = todos
                        .into_iter()
                        .map(|todo| agendao_types::TodoInfo {
                            content: todo.content,
                            status: todo.status,
                            priority: todo.priority,
                        })
                        .collect();
                    state.todo_manager.update(&session_id, todos).await;
                    Ok(())
                }
            }
        })
        .with_todo_get({
            let state = self.state.clone();
            move |session_id| {
                let state = state.clone();
                async move {
                    Ok(state
                        .todo_manager
                        .get(&session_id)
                        .await
                        .into_iter()
                        .map(|todo| agendao_tool::TodoItemData {
                            content: todo.content,
                            status: todo.status,
                            priority: todo.priority,
                        })
                        .collect())
                }
            }
        })
        .with_file_time_read({
            let tracker = self.state.file_time_tracker.clone();
            move |session_id, file_path| {
                let tracker = tracker.clone();
                async move { tracker.record(&session_id, &file_path) }
            }
        })
        .with_file_time_assert({
            let tracker = self.state.file_time_tracker.clone();
            move |session_id, file_path| {
                let tracker = tracker.clone();
                async move { tracker.assert_unchanged(&session_id, &file_path) }
            }
        });
        base_ctx.call_id = metadata
            .get("call_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        base_ctx.extra = metadata.clone();
        let mut available_tool_ids = self
            .capability_allowed_tools_by_agent
            .get(agent_name)
            .cloned()
            .unwrap_or_default();
        available_tool_ids.sort();
        available_tool_ids.dedup();
        let mut available_toolsets =
            agendao_skill::infer_toolsets_from_tools(available_tool_ids.iter().map(String::as_str))
                .into_iter()
                .collect::<Vec<_>>();
        available_toolsets.sort();
        base_ctx.extra.insert(
            "available_tool_ids".to_string(),
            serde_json::json!(available_tool_ids),
        );
        base_ctx.extra.insert(
            agendao_tool::tool_catalog::CAPABILITY_ALLOWED_TOOL_IDS_KEY.to_string(),
            serde_json::json!(available_tool_ids),
        );
        base_ctx.extra.insert(
            "available_toolsets".to_string(),
            serde_json::json!(available_toolsets),
        );
        base_ctx
    }

    async fn hydrate_scheduler_context(
        &self,
        arguments: serde_json::Value,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> std::result::Result<SchedulerToolOutput, SchedulerToolError> {
        let requested_ids = scheduler_context_hydrate_message_ids(&arguments)?;
        let allowed_ids = scheduler_context_allowed_message_ids(metadata);
        if allowed_ids.is_empty() {
            return Err(SchedulerToolError::InvalidArguments(
                "scheduler continuity packet is unavailable; no hydration anchors are authorized"
                    .to_string(),
            ));
        }
        let per_message_limit = scheduler_context_hydrate_message_limit(&arguments);
        let session = {
            let sessions = self.state.sessions.lock().await;
            sessions.get(&self.session_id).cloned()
        }
        .ok_or_else(|| {
            SchedulerToolError::Execution("session is no longer available".to_string())
        })?;

        let mut hydrated = Vec::new();
        let mut hydrated_ids = Vec::new();
        let mut rejected = Vec::new();
        let mut missing = Vec::new();
        for message_id in requested_ids {
            if !allowed_ids.contains(&message_id) {
                rejected.push(message_id);
                continue;
            }
            let Some(message) = session.get_message(&message_id) else {
                missing.push(message_id);
                continue;
            };
            if let Some(rendered) =
                render_scheduler_context_hydrated_message(message, per_message_limit)
            {
                hydrated.push(rendered);
                hydrated_ids.push(message_id);
            } else {
                missing.push(message_id);
            }
        }

        let mut sections = vec![
            "## Scheduler Context Hydration\nHydrated exact same-session sources authorized by the scheduler continuity packet."
                .to_string(),
        ];
        if !hydrated.is_empty() {
            sections.push(format!("## Hydrated Messages\n{}", hydrated.join("\n")));
        }
        if !rejected.is_empty() {
            sections.push(format!(
                "## Rejected Message IDs\n{}",
                rejected
                    .iter()
                    .map(|id| format!("- `{id}`: not present in scheduler continuity anchors"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !missing.is_empty() {
            sections.push(format!(
                "## Missing Message IDs\n{}",
                missing
                    .iter()
                    .map(|id| format!("- `{id}`: not found or no hydratable text"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        Ok(SchedulerToolOutput {
            output: sections.join("\n\n"),
            is_error: false,
            title: Some("Scheduler context hydrated".to_string()),
            metadata: Some(serde_json::json!({
                "hydrated_count": hydrated.len(),
                "rejected_count": rejected.len(),
                "missing_count": missing.len(),
                "hydrated_message_ids": hydrated_ids,
                "rejected_message_ids": rejected,
                "missing_message_ids": missing,
                "max_chars_per_message": per_message_limit,
            })),
        })
    }

    async fn hydrate_scheduler_memory(
        &self,
        arguments: serde_json::Value,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> std::result::Result<SchedulerToolOutput, SchedulerToolError> {
        let requested_ids = scheduler_memory_hydrate_record_ids(&arguments)?;
        let allowed_ids = scheduler_memory_allowed_record_ids(metadata);
        if allowed_ids.is_empty() {
            return Err(SchedulerToolError::InvalidArguments(
                "scheduler continuity packet is unavailable; no memory anchors are authorized"
                    .to_string(),
            ));
        }
        let per_record_limit = scheduler_memory_hydrate_record_limit(&arguments);
        let include_evidence = scheduler_memory_hydrate_include_evidence(&arguments);

        let mut hydrated = Vec::new();
        let mut hydrated_ids = Vec::new();
        let mut rejected = Vec::new();
        let mut missing = Vec::new();
        for record_id in requested_ids {
            if !allowed_ids.contains(&record_id) {
                rejected.push(record_id);
                continue;
            }
            let detail = self
                .state
                .runtime_memory
                .get_memory_detail(&MemoryRecordId(record_id.clone()))
                .await
                .map_err(|error| SchedulerToolError::Execution(error.to_string()))?;
            let Some(detail) = detail else {
                missing.push(record_id);
                continue;
            };
            hydrated.push(render_scheduler_memory_hydrated_record(
                &detail,
                include_evidence,
                per_record_limit,
            ));
            hydrated_ids.push(record_id);
        }

        let mut sections = vec![
            "## Scheduler Memory Hydration\nHydrated memory records authorized by the scheduler continuity packet."
                .to_string(),
        ];
        if !hydrated.is_empty() {
            sections.push(format!(
                "## Hydrated Memory Records\n{}",
                hydrated.join("\n")
            ));
        }
        if !rejected.is_empty() {
            sections.push(format!(
                "## Rejected Memory Record IDs\n{}",
                rejected
                    .iter()
                    .map(|id| format!("- `{id}`: not present in scheduler memory anchors"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !missing.is_empty() {
            sections.push(format!(
                "## Missing Memory Record IDs\n{}",
                missing
                    .iter()
                    .map(|id| format!("- `{id}`: not found or not visible in memory scope"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        Ok(SchedulerToolOutput {
            output: sections.join("\n\n"),
            is_error: false,
            title: Some("Scheduler memory hydrated".to_string()),
            metadata: Some(serde_json::json!({
                "hydrated_count": hydrated.len(),
                "rejected_count": rejected.len(),
                "missing_count": missing.len(),
                "hydrated_memory_record_ids": hydrated_ids,
                "rejected_memory_record_ids": rejected,
                "missing_memory_record_ids": missing,
                "max_chars_per_record": per_record_limit,
                "include_evidence": include_evidence,
            })),
        })
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        agent_name: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> std::result::Result<SchedulerToolOutput, SchedulerToolError> {
        if tool_name == SCHEDULER_CONTEXT_HYDRATE_TOOL {
            return self.hydrate_scheduler_context(arguments, metadata).await;
        }
        if tool_name == SCHEDULER_MEMORY_HYDRATE_TOOL {
            return self.hydrate_scheduler_memory(arguments, metadata).await;
        }

        let context = self.build_tool_context(agent_name, metadata).await;
        match self
            .state
            .tool_registry
            .execute(tool_name, arguments, context)
            .await
        {
            Ok(result) => Ok(SchedulerToolOutput {
                output: result.output,
                title: (!result.title.is_empty()).then_some(result.title),
                metadata: (!result.metadata.is_empty())
                    .then(|| serde_json::to_value(result.metadata).unwrap_or_default()),
                is_error: false,
            }),
            Err(error) => Ok(SchedulerToolOutput {
                output: error.to_string(),
                title: Some("Tool error".to_string()),
                metadata: None,
                is_error: true,
            }),
        }
    }
}

fn scheduler_context_hydrate_message_ids(
    arguments: &serde_json::Value,
) -> std::result::Result<Vec<String>, SchedulerToolError> {
    let Some(values) = arguments
        .get("message_ids")
        .and_then(|value| value.as_array())
    else {
        return Err(SchedulerToolError::InvalidArguments(
            "message_ids must be an array of scheduler continuity message ids".to_string(),
        ));
    };
    if values.is_empty() {
        return Err(SchedulerToolError::InvalidArguments(
            "message_ids must not be empty".to_string(),
        ));
    }
    if values.len() > SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES {
        return Err(SchedulerToolError::InvalidArguments(format!(
            "message_ids must contain at most {SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES} ids"
        )));
    }
    let mut ids = Vec::new();
    for value in values {
        let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) else {
            return Err(SchedulerToolError::InvalidArguments(
                "message_ids must only contain non-empty strings".to_string(),
            ));
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn scheduler_context_hydrate_message_limit(arguments: &serde_json::Value) -> usize {
    arguments
        .get("max_chars_per_message")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(SCHEDULER_CONTEXT_HYDRATE_DEFAULT_MESSAGE_LIMIT)
        .clamp(1, SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT)
}

fn scheduler_context_allowed_message_ids(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    metadata
        .get(SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY)
        .and_then(agendao_session::prompt::continuity_packet_allowed_message_ids)
        .unwrap_or_default()
}

fn scheduler_memory_hydrate_record_ids(
    arguments: &serde_json::Value,
) -> std::result::Result<Vec<String>, SchedulerToolError> {
    let Some(values) = arguments
        .get("record_ids")
        .and_then(|value| value.as_array())
    else {
        return Err(SchedulerToolError::InvalidArguments(
            "record_ids must be an array of scheduler memory anchor ids".to_string(),
        ));
    };
    if values.is_empty() {
        return Err(SchedulerToolError::InvalidArguments(
            "record_ids must not be empty".to_string(),
        ));
    }
    if values.len() > SCHEDULER_MEMORY_HYDRATE_MAX_RECORDS {
        return Err(SchedulerToolError::InvalidArguments(format!(
            "record_ids must contain at most {SCHEDULER_MEMORY_HYDRATE_MAX_RECORDS} ids"
        )));
    }
    let mut ids = Vec::new();
    for value in values {
        let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) else {
            return Err(SchedulerToolError::InvalidArguments(
                "record_ids must only contain non-empty strings".to_string(),
            ));
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

fn scheduler_memory_hydrate_record_limit(arguments: &serde_json::Value) -> usize {
    arguments
        .get("max_chars_per_record")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(SCHEDULER_MEMORY_HYDRATE_DEFAULT_RECORD_LIMIT)
        .clamp(1, SCHEDULER_MEMORY_HYDRATE_MAX_RECORD_LIMIT)
}

fn scheduler_memory_hydrate_include_evidence(arguments: &serde_json::Value) -> bool {
    arguments
        .get("include_evidence")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn scheduler_memory_allowed_record_ids(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    metadata
        .get(SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY)
        .and_then(SessionContinuityPacket::from_value)
        .map(|packet| packet.allowed_memory_record_ids())
        .unwrap_or_default()
}

fn render_scheduler_context_hydrated_message(
    message: &SessionMessage,
    per_message_limit: usize,
) -> Option<String> {
    let text = scheduler_context_hydratable_text(message)?;
    let text = truncate_scheduler_context_hydration(&text, per_message_limit);
    Some(format!(
        "- {} `{}`:\n{}",
        scheduler_context_role_label(&message.role),
        message.id,
        indent_scheduler_context_hydration(&text)
    ))
}

fn scheduler_context_hydratable_text(message: &SessionMessage) -> Option<String> {
    let mut parts = Vec::new();
    let text = message.get_text();
    let text = text.trim();
    if !text.is_empty() {
        parts.push(text.to_string());
    }
    if let Some(summary) = message_latest_compaction_summary(&message.metadata, &message.id, None) {
        parts.push(format!(
            "[continuity compaction summary]\n{}",
            summary.summary.trim()
        ));
        return (!parts.is_empty()).then(|| parts.join("\n\n"));
    }
    for part in &message.parts {
        if let SessionPartType::Compaction { summary } = &part.part_type {
            let summary = summary.trim();
            if !summary.is_empty() {
                parts.push(format!("[continuity compaction summary]\n{summary}"));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

fn scheduler_context_role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

fn indent_scheduler_context_hydration(text: &str) -> String {
    text.lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_scheduler_context_hydration(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(limit.saturating_sub(24))
        .collect::<String>();
    truncated.push_str("\n...[truncated]...");
    truncated
}

fn render_scheduler_memory_hydrated_record(
    detail: &MemoryDetailView,
    include_evidence: bool,
    per_record_limit: usize,
) -> String {
    let record = &detail.record;
    let mut lines = vec![
        format!(
            "- memory `{}` [{} / {} / {} / validation:{}]: {}",
            record.id.0,
            scheduler_memory_label(&record.kind),
            scheduler_memory_label(&record.scope),
            scheduler_memory_label(&record.status),
            scheduler_memory_label(&record.validation_status),
            record.title.trim()
        ),
        format!("  summary: {}", record.summary.trim()),
    ];
    if let Some(confidence) = record.confidence {
        lines.push(format!("  confidence: {confidence:.2}"));
    }
    if let Some(source_session_id) = record.source_session_id.as_deref() {
        lines.push(format!("  source_session_id: `{source_session_id}`"));
    }
    if let Some(workspace_identity) = record.workspace_identity.as_deref() {
        lines.push(format!("  workspace_identity: `{workspace_identity}`"));
    }
    if !record.trigger_conditions.is_empty() {
        lines.push("  trigger_conditions:".to_string());
        lines.extend(render_scheduler_memory_list(&record.trigger_conditions));
    }
    if !record.normalized_facts.is_empty() {
        lines.push("  normalized_facts:".to_string());
        lines.extend(render_scheduler_memory_list(&record.normalized_facts));
    }
    if !record.boundaries.is_empty() {
        lines.push("  boundaries:".to_string());
        lines.extend(render_scheduler_memory_list(&record.boundaries));
    }
    if include_evidence && !record.evidence_refs.is_empty() {
        lines.push("  evidence_refs:".to_string());
        lines.extend(
            record
                .evidence_refs
                .iter()
                .map(render_scheduler_memory_evidence_ref),
        );
    }
    if let Some(derived_skill_name) = record.derived_skill_name.as_deref() {
        lines.push(format!("  derived_skill_name: `{derived_skill_name}`"));
    }
    if let Some(linked_skill_name) = record.linked_skill_name.as_deref() {
        lines.push(format!("  linked_skill_name: `{linked_skill_name}`"));
    }
    truncate_scheduler_context_hydration(&lines.join("\n"), per_record_limit)
}

fn scheduler_memory_label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn render_scheduler_memory_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| format!("    - {}", value.trim()))
        .collect()
}

fn render_scheduler_memory_evidence_ref(evidence: &MemoryEvidenceRef) -> String {
    let mut parts = Vec::new();
    if let Some(session_id) = evidence.session_id.as_deref() {
        parts.push(format!("session_id=`{session_id}`"));
    }
    if let Some(message_id) = evidence.message_id.as_deref() {
        parts.push(format!("message_id=`{message_id}`"));
    }
    if let Some(tool_call_id) = evidence.tool_call_id.as_deref() {
        parts.push(format!("tool_call_id=`{tool_call_id}`"));
    }
    if let Some(stage_id) = evidence.stage_id.as_deref() {
        parts.push(format!("stage_id=`{stage_id}`"));
    }
    if let Some(note) = evidence.note.as_deref() {
        parts.push(format!("note={}", note.trim()));
    }
    if parts.is_empty() {
        "    - evidence reference with no details".to_string()
    } else {
        format!("    - {}", parts.join("; "))
    }
}

fn scheduler_context_hydrate_tool_definition() -> agendao_provider::ToolDefinition {
    agendao_provider::ToolDefinition {
        name: SCHEDULER_CONTEXT_HYDRATE_TOOL.to_string(),
        description: Some(
            "Hydrate exact same-session messages identified by Scheduler Continuity Source Anchors. Use only when the current task needs prior context that is truncated, summarized, or ambiguous."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "required": ["message_ids"],
            "properties": {
                "message_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGES,
                    "items": {"type": "string"},
                    "description": "Message ids from the Scheduler Continuity Source Anchors."
                },
                "max_chars_per_message": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT,
                    "description": "Maximum characters to return per hydrated message."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn scheduler_memory_hydrate_tool_definition() -> agendao_provider::ToolDefinition {
    agendao_provider::ToolDefinition {
        name: SCHEDULER_MEMORY_HYDRATE_TOOL.to_string(),
        description: Some(
            "Hydrate memory records identified by Scheduler Continuity Memory Anchors. Use only for exact cross-session memory details authorized by the current continuity packet."
                .to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "required": ["record_ids"],
            "properties": {
                "record_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": SCHEDULER_MEMORY_HYDRATE_MAX_RECORDS,
                    "items": {"type": "string"},
                    "description": "Memory record ids from the Scheduler Continuity Memory Anchors."
                },
                "include_evidence": {
                    "type": "boolean",
                    "description": "Whether to include provenance evidence refs for each hydrated memory record."
                },
                "max_chars_per_record": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": SCHEDULER_MEMORY_HYDRATE_MAX_RECORD_LIMIT,
                    "description": "Maximum characters to return per hydrated memory record."
                }
            },
            "additionalProperties": false
        }),
    }
}

pub(crate) fn scheduler_host_tool_definitions() -> Vec<agendao_provider::ToolDefinition> {
    vec![
        scheduler_context_hydrate_tool_definition(),
        scheduler_memory_hydrate_tool_definition(),
    ]
}

#[async_trait]
impl ToolBackend for SessionSchedulerToolExecutor {
    async fn execute(
        &self,
        observation: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> std::result::Result<ToolExecution, String> {
        let metadata = self.native_execution_metadata(Some(&call.id));
        match self
            .execute_tool(
                call.tool.as_str(),
                call.arguments.clone(),
                observation.agent.as_str(),
                &metadata,
            )
            .await
        {
            Ok(output) => Ok(ToolExecution {
                output: output.output,
                title: output.title,
                metadata: output.metadata,
                is_error: output.is_error,
            }),
            Err(error) => Ok(ToolExecution {
                output: error.to_string(),
                title: Some("Tool error".to_string()),
                metadata: None,
                is_error: true,
            }),
        }
    }
}

pub(crate) fn resolve_config_default_agent_name(config: &AppConfig) -> String {
    config
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("build")
        .to_string()
}

pub(crate) struct ResolvedPromptRequestConfig {
    pub scheduler_choice: agendao_orchestrator::selector::SchedulerChoice,
    pub resolved_agent: Option<AgentInfo>,
    pub provider: Arc<dyn agendao_provider::Provider>,
    pub provider_id: String,
    pub model_id: String,
    pub compiled_request: CompiledExecutionRequest,
}

pub(crate) fn apply_scheduler_selection_session_metadata(
    session: &mut agendao_session::Session,
    resolved: &ResolvedPromptRequestConfig,
) {
    prepare_scheduler_blueprint_lock(session, &resolved.scheduler_choice);
}

fn prepare_scheduler_blueprint_lock(
    session: &mut agendao_session::Session,
    choice: &agendao_orchestrator::selector::SchedulerChoice,
) {
    if !matches!(
        choice,
        agendao_orchestrator::selector::SchedulerChoice::Auto
    ) {
        clear_scheduler_blueprint_lock(session);
    }
}

fn clear_scheduler_blueprint_lock(session: &mut agendao_session::Session) {
    for key in [
        crate::scheduler_runner::BLUEPRINT_LOCK_METADATA_KEY,
        crate::scheduler_runner::BLUEPRINT_FINGERPRINT_METADATA_KEY,
        crate::scheduler_runner::SELECTION_SOURCE_METADATA_KEY,
        crate::scheduler_runner::GENERATED_AGENTS_METADATA_KEY,
    ] {
        session.remove_metadata(key);
    }
}

pub(super) fn resolve_request_model_inputs(
    agent_model: Option<&str>,
    request_model: Option<&str>,
    config_model: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    (
        request_model.map(str::to_string),
        agent_model.or(config_model).map(str::to_string),
        None,
    )
}

fn build_execution_resolution_context(
    session_id: &str,
    provider_id: &str,
    model_id: &str,
    request_variant: Option<&str>,
    resolved_agent: Option<&AgentInfo>,
) -> ExecutionResolutionContext {
    ExecutionResolutionContext {
        session_id: session_id.to_string(),
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        max_tokens: resolved_agent.and_then(|agent| agent.max_tokens),
        temperature: resolved_agent.and_then(|agent| agent.temperature),
        top_p: resolved_agent.and_then(|agent| agent.top_p),
        variant: request_variant
            .map(str::to_string)
            .or_else(|| resolved_agent.and_then(|agent| agent.variant.clone())),
    }
}

pub(crate) async fn resolve_prompt_request_config(
    input: PromptRequestConfigInput<'_>,
) -> Result<ResolvedPromptRequestConfig> {
    let PromptRequestConfigInput {
        state,
        config,
        session_id,
        requested_agent,
        requested_scheduler,
        request_model,
        request_variant,
        route,
    } = input;

    let scheduler_choice = requested_scheduler.clone();
    let default_agent_name = requested_agent
        .is_none()
        .then(|| resolve_config_default_agent_name(config));

    let agent_registry = AgentRegistry::from_config(config);
    let selected_agent_name = requested_agent.or(default_agent_name.as_deref());
    let resolved_agent = selected_agent_name.and_then(|name| agent_registry.get(name).cloned());
    if let Some(requested_agent) = requested_agent {
        if resolved_agent.is_none() {
            return Err(crate::error::ApiError::BadRequest(format!(
                "unknown agent '{requested_agent}'"
            )));
        }
    }

    let agent_model = resolved_agent
        .as_ref()
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    let (request_model_input, config_model_input, config_provider_input) =
        resolve_request_model_inputs(
            agent_model.as_deref(),
            request_model,
            config.model.as_deref(),
        );
    let (provider, provider_id, model_id) = resolve_provider_and_model(
        state,
        request_model_input.as_deref(),
        config_model_input.as_deref(),
        config_provider_input.as_deref(),
    )
    .await?;

    let compiled_request = resolve_compiled_execution_request(
        config,
        &build_execution_resolution_context(
            session_id,
            &provider_id,
            &model_id,
            request_variant,
            resolved_agent.as_ref(),
        ),
    )
    .await;
    tracing::info!(
        route,
        requested_agent = ?requested_agent,
        default_agent = ?default_agent_name,
        resolved_agent = ?resolved_agent.as_ref().map(|agent| agent.name.as_str()),
        agent_model = ?agent_model,
        request_model_input = ?request_model_input,
        config_model_input = ?config_model_input,
        config_provider_input = ?config_provider_input,
        "resolved request prompt agent configuration"
    );

    Ok(ResolvedPromptRequestConfig {
        scheduler_choice,
        resolved_agent,
        provider,
        provider_id,
        model_id,
        compiled_request,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn session_with_blueprint_lock() -> agendao_session::Session {
        let mut session = agendao_session::Session::new("project", "/tmp/project");
        for key in [
            crate::scheduler_runner::BLUEPRINT_LOCK_METADATA_KEY,
            crate::scheduler_runner::BLUEPRINT_FINGERPRINT_METADATA_KEY,
            crate::scheduler_runner::SELECTION_SOURCE_METADATA_KEY,
            crate::scheduler_runner::GENERATED_AGENTS_METADATA_KEY,
        ] {
            session.insert_metadata(key, serde_json::json!("locked"));
        }
        session
    }

    #[test]
    fn auto_request_preserves_the_previous_blueprint_lock() {
        let mut session = session_with_blueprint_lock();
        prepare_scheduler_blueprint_lock(
            &mut session,
            &agendao_orchestrator::selector::SchedulerChoice::Auto,
        );
        assert_eq!(
            session
                .record()
                .metadata
                .get(crate::scheduler_runner::SELECTION_SOURCE_METADATA_KEY),
            Some(&serde_json::json!("locked"))
        );
    }

    #[test]
    fn explicit_request_clears_the_previous_blueprint_lock() {
        let mut session = session_with_blueprint_lock();
        prepare_scheduler_blueprint_lock(
            &mut session,
            &agendao_orchestrator::selector::SchedulerChoice::Template {
                template: agendao_orchestrator::templates::TemplateId::Direct,
            },
        );
        for key in [
            crate::scheduler_runner::BLUEPRINT_LOCK_METADATA_KEY,
            crate::scheduler_runner::BLUEPRINT_FINGERPRINT_METADATA_KEY,
            crate::scheduler_runner::SELECTION_SOURCE_METADATA_KEY,
            crate::scheduler_runner::GENERATED_AGENTS_METADATA_KEY,
        ] {
            assert!(!session.record().metadata.contains_key(key));
        }
    }

    #[test]
    fn scheduler_context_hydrate_only_allows_packet_anchors() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "exact_recent_tail": [
                    {"message_id": "msg_user", "role": "user"},
                    {"message_id": "msg_assistant", "role": "assistant"}
                ],
                "latest_compaction_summary": {"message_id": "msg_compaction"}
            }),
        )]);

        let allowed = scheduler_context_allowed_message_ids(&metadata);

        assert_eq!(
            allowed,
            vec![
                "msg_assistant".to_string(),
                "msg_compaction".to_string(),
                "msg_user".to_string()
            ]
        );
    }

    #[test]
    fn scheduler_context_hydrate_rejects_unknown_packet_version() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 99,
                "exact_recent_tail": [
                    {"message_id": "msg_user", "role": "user"}
                ]
            }),
        )]);

        assert!(scheduler_context_allowed_message_ids(&metadata).is_empty());
    }

    #[test]
    fn scheduler_context_hydrate_arguments_validate_and_dedupe_ids() {
        let ids = scheduler_context_hydrate_message_ids(&serde_json::json!({
            "message_ids": ["msg_1", "msg_1", "msg_2"]
        }))
        .expect("valid message ids should parse");

        assert_eq!(ids, vec!["msg_1".to_string(), "msg_2".to_string()]);
        assert!(scheduler_context_hydrate_message_ids(&serde_json::json!({
            "message_ids": []
        }))
        .is_err());
        assert_eq!(
            scheduler_context_hydrate_message_limit(&serde_json::json!({
                "max_chars_per_message": 99_999
            })),
            SCHEDULER_CONTEXT_HYDRATE_MAX_MESSAGE_LIMIT
        );
    }

    #[test]
    fn scheduler_context_hydrate_renders_text_and_compaction_parts() {
        let mut message = SessionMessage::assistant("session");
        message.id = "msg_compaction".to_string();
        message.add_text("visible text");
        message.parts.push(agendao_session::MessagePart {
            id: "part_compaction".to_string(),
            part_type: SessionPartType::Compaction {
                summary: "older findings".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: Some(message.id.clone()),
        });

        let rendered = render_scheduler_context_hydrated_message(&message, 4_000)
            .expect("message should hydrate");

        assert!(rendered.contains("assistant `msg_compaction`"));
        assert!(rendered.contains("visible text"));
        assert!(rendered.contains("[continuity compaction summary]"));
        assert!(rendered.contains("older findings"));
    }

    #[test]
    fn scheduler_context_hydrate_prefers_packet_summary_text() {
        let mut message = SessionMessage::assistant("session");
        message.id = "msg_compaction_packet".to_string();
        message.metadata.insert(
            "context_compaction_continuity_packet".to_string(),
            serde_json::json!({
                "version": 1,
                "latest_compaction_summary": {
                    "message_id": "msg_compaction_packet",
                    "summary": "packet owned continuity summary"
                }
            }),
        );
        message.parts.push(agendao_session::MessagePart {
            id: "part_compaction_packet".to_string(),
            part_type: SessionPartType::Compaction {
                summary: "older raw summary".to_string(),
            },
            created_at: chrono::Utc::now(),
            message_id: Some(message.id.clone()),
        });

        let rendered = render_scheduler_context_hydrated_message(&message, 4_000)
            .expect("message should hydrate");

        assert!(rendered.contains("[continuity compaction summary]"));
        assert!(rendered.contains("packet owned continuity summary"));
        assert!(!rendered.contains("older raw summary"));
    }

    #[test]
    fn scheduler_memory_hydrate_only_allows_packet_anchors() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 1,
                "memory_anchors": [
                    {"record_id": "mem_b", "title": "B", "kind": "Pattern", "status": "Validated"},
                    {"record_id": "mem_a", "title": "A", "kind": "Lesson", "status": "Consolidated"}
                ]
            }),
        )]);

        let allowed = scheduler_memory_allowed_record_ids(&metadata);

        assert_eq!(allowed, vec!["mem_a".to_string(), "mem_b".to_string()]);
    }

    #[test]
    fn scheduler_memory_hydrate_rejects_unknown_packet_version() {
        let metadata = HashMap::from([(
            SCHEDULER_SESSION_CONTEXT_PACKET_METADATA_KEY.to_string(),
            serde_json::json!({
                "version": 99,
                "memory_anchors": [
                    {"record_id": "mem_a", "title": "A", "kind": "Lesson", "status": "Validated"}
                ]
            }),
        )]);

        assert!(scheduler_memory_allowed_record_ids(&metadata).is_empty());
    }

    #[test]
    fn scheduler_memory_hydrate_arguments_validate_and_dedupe_ids() {
        let ids = scheduler_memory_hydrate_record_ids(&serde_json::json!({
            "record_ids": ["mem_1", "mem_1", "mem_2"]
        }))
        .expect("valid record ids should parse");

        assert_eq!(ids, vec!["mem_1".to_string(), "mem_2".to_string()]);
        assert!(scheduler_memory_hydrate_record_ids(&serde_json::json!({
            "record_ids": []
        }))
        .is_err());
        assert_eq!(
            scheduler_memory_hydrate_record_limit(&serde_json::json!({
                "max_chars_per_record": 99_999
            })),
            SCHEDULER_MEMORY_HYDRATE_MAX_RECORD_LIMIT
        );
        assert!(scheduler_memory_hydrate_include_evidence(
            &serde_json::json!({
                "include_evidence": true
            })
        ));
    }

    #[test]
    fn scheduler_memory_hydrate_renders_detail_and_optional_evidence() {
        let detail = MemoryDetailView {
            record: agendao_types::MemoryRecord {
                id: MemoryRecordId("mem_123".to_string()),
                kind: agendao_types::MemoryKind::Lesson,
                scope: agendao_types::MemoryScope::WorkspaceShared,
                status: agendao_types::MemoryStatus::Validated,
                title: "Audit hydration boundary".to_string(),
                summary: "Use anchor-gated hydration for scheduler memory recall.".to_string(),
                trigger_conditions: vec!["scheduler continuity".to_string()],
                normalized_facts: vec!["hydration_scope=memory_anchor".to_string()],
                boundaries: vec!["Do not hydrate ids outside packet anchors.".to_string()],
                confidence: Some(0.9),
                evidence_refs: vec![MemoryEvidenceRef {
                    session_id: Some("session".to_string()),
                    message_id: Some("msg_a".to_string()),
                    tool_call_id: Some("tool_a".to_string()),
                    stage_id: Some("stage_a".to_string()),
                    note: Some("test evidence".to_string()),
                }],
                source_session_id: Some("session".to_string()),
                workspace_identity: Some("workspace:test".to_string()),
                created_at: 1,
                updated_at: 2,
                last_validated_at: None,
                expires_at: None,
                derived_skill_name: None,
                linked_skill_name: None,
                validation_status: agendao_types::MemoryValidationStatus::Passed,
            },
        };

        let without_evidence = render_scheduler_memory_hydrated_record(&detail, false, 4_000);
        assert!(without_evidence.contains("memory `mem_123`"));
        assert!(without_evidence.contains("lesson / workspace_shared / validated"));
        assert!(without_evidence.contains("hydration_scope=memory_anchor"));
        assert!(!without_evidence.contains("evidence_refs"));

        let with_evidence = render_scheduler_memory_hydrated_record(&detail, true, 4_000);
        assert!(with_evidence.contains("evidence_refs"));
        assert!(with_evidence.contains("message_id=`msg_a`"));
    }
}
