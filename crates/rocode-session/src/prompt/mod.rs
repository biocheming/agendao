pub mod compaction_helpers;
mod file_parts;
pub(crate) mod hooks;
pub mod ingress;
mod loop_lifecycle;
mod message_building;
mod runtime_step;
pub mod shell;
mod skill_reflection;
pub mod subtask;
mod subtask_runtime;
mod surface_contract;
#[cfg(test)]
mod tests;
mod tool_calls;
mod tool_execution;
pub mod tools_and_output;

pub const PROMPT_SURFACE_RUNTIME_SNAPSHOT_METADATA_KEY: &str = "prompt_surface_runtime_snapshot";
pub const PROMPT_SURFACE_SNAPSHOT_INVALIDATION_METADATA_KEY: &str =
    "prompt_surface_snapshot_invalidation";

pub fn sanctioned_model_context_summary(message: &SessionMessage) -> Option<&str> {
    surface_contract::sanctioned_model_context_projection_for_message(message)
        .map(|projection| projection.summary)
}

pub use compaction_helpers::{should_compact, trigger_compaction};
pub(crate) use hooks::{
    apply_chat_message_hook_outputs, apply_chat_messages_hook_outputs, session_message_hook_payload,
};
pub use ingress::{
    normalize_ingress_source, stabilize_ingress_turns, IngressAttachmentRef, IngressSource,
    IngressStabilizationMetadata, IngressTurnEnvelope, INGRESS_POLICY_ENTRY_METADATA_ONLY,
    INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH, INGRESS_POLICY_SCHEDULER_METADATA_ONLY,
    INGRESS_POLICY_UNSPECIFIED,
};
#[cfg(test)]
pub(crate) use shell::resolve_shell_invocation;
pub use shell::{resolve_command_template, shell_exec, CommandInput, ShellInput};
pub use subtask::{tool_definitions_from_schemas, SubtaskExecutor, ToolSchema};
use surface_contract::HiddenRuntimeHint;
pub use tools_and_output::{
    compose_session_title_source, create_structured_output_tool, extract_structured_output,
    generate_session_title, generate_session_title_for_session, generate_session_title_llm,
    insert_reminders, max_steps_for_agent, merge_tool_definitions, prioritize_tool_definitions,
    resolve_tool_surface, resolve_tool_surface_with_mcp, resolve_tools, resolve_tools_with_mcp,
    resolve_tools_with_mcp_registry, sanitize_session_title_source,
    structured_output_system_prompt, was_plan_agent, ResolvedTool, ResolvedToolSurface,
    StructuredOutputConfig,
};

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use rocode_content::output_blocks::OutputBlock;
use rocode_execution_types::CompiledExecutionRequest;
use rocode_provider::{Provider, ToolDefinition};
use rocode_skill::RuntimeInstructionSource;
use rocode_types::MemoryRetrievalPacket;

use crate::instruction::{InstructionLoader, InstructionSource};
use crate::system::SystemPrompt;
use crate::{MessageRole, PartType, Session, SessionMessage, SessionStateManager};

const MAX_STEPS: u32 = 100;
const STREAM_UPDATE_INTERVAL_MS: u64 = 120;

/// Returns `true` when the finish reason indicates the conversation turn is
/// complete (i.e. not a tool-use continuation or unknown state).
fn is_terminal_finish(reason: Option<&str>) -> bool {
    !matches!(
        reason,
        None | Some("tool-calls") | Some("tool_calls") | Some("unknown")
    )
}

#[derive(Debug, Clone)]
pub struct PromptInput {
    pub session_id: String,
    pub message_id: Option<String>,
    pub model: Option<ModelRef>,
    pub agent: Option<String>,
    pub no_reply: bool,
    pub system: Option<String>,
    pub variant: Option<String>,
    pub parts: Vec<PartInput>,
    pub tools: Option<HashMap<String, bool>>,
    pub ingress: Option<IngressTurnEnvelope>,
}

#[derive(Debug, Clone)]
pub struct ModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PartInput {
    Text {
        text: String,
    },
    File {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    Agent {
        name: String,
    },
    Subtask {
        prompt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        agent: String,
    },
}

impl TryFrom<serde_json::Value> for PartInput {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value).map_err(|e| format!("Invalid PartInput: {}", e))
    }
}

impl PartInput {
    /// Parse a JSON array of parts into a Vec<PartInput>, skipping invalid entries.
    pub fn parse_array(value: &serde_json::Value) -> Vec<PartInput> {
        match value.as_array() {
            Some(arr) => arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            None => Vec::new(),
        }
    }
}

struct PromptState {
    cancel_token: CancellationToken,
}

#[derive(Debug, Clone)]
struct StreamToolState {
    name: String,
    raw_input: String,
    input: serde_json::Value,
    status: crate::ToolCallStatus,
    state: crate::ToolState,
    emitted_output_start: bool,
    emitted_output_detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub(super) struct PersistedSubsession {
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
    #[serde(default)]
    disabled_tools: Vec<String>,
    #[serde(default)]
    history: Vec<PersistedSubsessionTurn>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PersistedSubsessionTurn {
    prompt: String,
    output: String,
}

/// LLM parameters derived from agent configuration.
#[derive(Debug, Clone, Default)]
pub struct AgentParams {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

pub type SessionUpdateHook = Arc<dyn Fn(&Session) + Send + Sync + 'static>;
pub type EventBroadcastHook = Arc<dyn Fn(serde_json::Value) + Send + Sync + 'static>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBlockEvent {
    pub session_id: String,
    pub block: OutputBlock,
    pub id: Option<String>,
}
pub type OutputBlockHook = Arc<
    dyn Fn(OutputBlockEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
>;
pub type AgentLookup =
    Arc<dyn Fn(&str) -> Option<rocode_tool::TaskAgentInfo> + Send + Sync + 'static>;
pub type PublishBusHook = Arc<
    dyn Fn(String, serde_json::Value) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync
        + 'static,
>;
pub type AskQuestionHook = Arc<
    dyn Fn(
            String,
            Vec<rocode_tool::QuestionDef>,
        )
            -> Pin<Box<dyn Future<Output = Result<Vec<Vec<String>>, rocode_tool::ToolError>> + Send>>
        + Send
        + Sync
        + 'static,
>;
pub type AskPermissionHook = Arc<
    dyn Fn(
            String,
            rocode_tool::PermissionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<(), rocode_tool::ToolError>> + Send>>
        + Send
        + Sync
        + 'static,
>;

#[derive(Clone, Default)]
pub struct PromptHooks {
    pub update_hook: Option<SessionUpdateHook>,
    pub event_broadcast: Option<EventBroadcastHook>,
    pub output_block_hook: Option<OutputBlockHook>,
    pub agent_lookup: Option<AgentLookup>,
    pub ask_question_hook: Option<AskQuestionHook>,
    pub ask_permission_hook: Option<AskPermissionHook>,
    pub publish_bus_hook: Option<PublishBusHook>,
}

#[derive(Clone)]
pub struct PromptRequestContext {
    pub provider: Arc<dyn Provider>,
    pub system_prompt: Option<String>,
    pub memory_prefetch: Option<MemoryRetrievalPacket>,
    pub tools: Vec<ToolDefinition>,
    pub tool_source_digests: Vec<rocode_provider::cache::ToolSurfaceSourceDigest>,
    pub compiled_request: CompiledExecutionRequest,
    pub hooks: PromptHooks,
}

pub struct SessionPrompt {
    state: Arc<Mutex<HashMap<String, PromptState>>>,
    session_state: Arc<RwLock<SessionStateManager>>,
    mcp_clients: Option<Arc<rocode_mcp::McpClientRegistry>>,
    lsp_registry: Option<Arc<rocode_lsp::LspClientRegistry>>,
    tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    config_store: Option<Arc<rocode_config::ConfigStore>>,
    memory_authority: Option<Arc<rocode_memory::MemoryAuthority>>,
    proposal_repo: Option<Arc<rocode_storage::SkillEvolutionProposalRepository>>,
    review_nudge_state: std::sync::Mutex<HashMap<String, ReviewNudgeThrottleState>>,
}

/// Signals collected from a completed session turn that drive the nudge
/// decision for background memory review.
///
/// Mirrors Hermes's nudge heartbeat: enough tool calls, errors, or skill
/// writes trigger a deterministic consolidation run against the current
/// workspace evidence.
#[derive(Debug, Clone)]
pub struct RuntimeReviewNudge {
    pub session_id: String,
    pub workspace_key: String,
    pub step_count: usize,
    pub tool_call_count: usize,
    pub error_tool_call_count: usize,
    pub skill_write_count: usize,
    pub used_skill_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ReviewNudgeThrottleState {
    last_completed_at: Option<tokio::time::Instant>,
    in_flight: bool,
}

impl RuntimeReviewNudge {
    /// Extract nudge signals from session messages after a completed loop.
    pub fn from_session(session: &Session, step_count: usize) -> Self {
        let turn_start = session
            .messages
            .iter()
            .rposition(|m| m.role == MessageRole::User)
            .unwrap_or(0);

        let mut tool_call_count = 0usize;
        let mut error_tool_call_count = 0usize;
        let mut skill_write_count = 0usize;
        let mut used_skill_names = Vec::new();

        for msg in session.messages.iter().skip(turn_start) {
            if msg.role != MessageRole::Assistant {
                continue;
            }
            for part in &msg.parts {
                match &part.part_type {
                    PartType::ToolCall { name, .. } => {
                        tool_call_count += 1;
                        if name == "skill_manage" {
                            skill_write_count += 1;
                        }
                    }
                    PartType::ToolResult { is_error, .. } => {
                        if *is_error {
                            error_tool_call_count += 1;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(skill_name) = msg.metadata.get("skill_name").and_then(|v| v.as_str()) {
                let name = skill_name.to_string();
                if !used_skill_names.contains(&name) {
                    used_skill_names.push(name);
                }
            }
        }

        Self {
            session_id: session.id.clone(),
            workspace_key: session_review_scope_key(session),
            step_count,
            tool_call_count,
            error_tool_call_count,
            skill_write_count,
            used_skill_names,
        }
    }
}

fn session_review_scope_key(session: &Session) -> String {
    let directory = session.directory.trim();
    if !directory.is_empty() {
        return format!("directory:{directory}");
    }

    let project_id = session.project_id.trim();
    if !project_id.is_empty() {
        return format!("project:{project_id}");
    }

    format!("session:{}", session.id)
}

/// Why a consolidation nudge was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkippedReason {
    /// Not enough tool calls, errors, or skill writes.
    BelowThreshold,
    /// A review ran recently for the same workspace/session scope.
    CooldownActive,
    /// A review is already running for the same workspace/session scope.
    ReviewInFlight,
    /// No memory repository is available.
    MemoryUnavailable,
    /// Consolidation was triggered but the engine call failed.
    ConsolidationFailed,
}

/// Outcome of the nudge decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NudgeDecision {
    /// Triggered: consolidation ran. `promoted_records` is the count of
    /// records that were promoted (which may include Lesson→Pattern as
    /// well as Pattern→MethodologyCandidate; filter by kind before
    /// treating as skill-worthy).
    Triggered {
        promoted: u32,
        merged: u32,
        archived: u32,
        promoted_records: u32,
        proposals_created: u32,
        proposals_skipped: u32,
    },
    /// Skipped for a specific reason.
    Skipped { reason: SkippedReason },
}

/// Append a session notice when the nudge generated skill evolution proposals.
/// The notice appears in the TUI session timeline as a synthetic assistant
/// message so the user can see proposals were created and use `/proposals`.
pub fn maybe_append_proposal_notice(session: &mut Session, decision: &NudgeDecision) {
    let proposals_created = match decision {
        NudgeDecision::Triggered {
            proposals_created, ..
        } => *proposals_created,
        NudgeDecision::Skipped { .. } => return,
    };
    if proposals_created == 0 {
        return;
    }

    let note = session.add_assistant_message();
    note.metadata.insert(
        "runtime_hint".to_string(),
        serde_json::json!(HiddenRuntimeHint::ProposalNotice.as_str()),
    );
    note.add_text(format!(
        "{} skill evolution proposal(s) generated from this run.\n\
         Review: type /proposals or run `rocode skill proposal list`.",
        proposals_created,
    ));
}

pub fn compact_session_now(session: &mut Session) -> Option<String> {
    compact_session_now_with_focus(session, None)
}

pub fn compact_session_now_with_focus(
    session: &mut Session,
    focus: Option<&str>,
) -> Option<String> {
    let filtered = SessionPrompt::filter_compacted_messages(&session.messages);
    SessionPrompt::trigger_compaction(session, &filtered, focus)
}

pub fn auto_compact_session_with_focus_if_needed(
    session: &mut Session,
    provider: &dyn rocode_provider::Provider,
    model_id: &str,
    max_output_tokens: Option<u64>,
    config_store: Option<&rocode_config::ConfigStore>,
    live_context_tokens: Option<u64>,
    focus: Option<&str>,
) -> Option<String> {
    let filtered = SessionPrompt::filter_compacted_messages(&session.messages);
    let compaction_config = SessionPrompt::runtime_compaction_config(config_store);
    if !SessionPrompt::should_compact(
        &filtered,
        provider,
        model_id,
        max_output_tokens,
        &compaction_config,
        live_context_tokens,
    ) {
        return None;
    }
    SessionPrompt::trigger_compaction(session, &filtered, focus)
}

pub fn estimate_current_context_tokens(messages: &[SessionMessage]) -> Option<u64> {
    let filtered = SessionPrompt::filter_compacted_messages(messages);
    latest_prompt_input_tokens(&filtered).or_else(|| estimate_tail_content_tokens(&filtered))
}

fn latest_prompt_input_tokens(messages: &[SessionMessage]) -> Option<u64> {
    messages.iter().rev().find_map(|message| {
        if !matches!(message.role, MessageRole::Assistant) {
            return None;
        }

        message
            .usage
            .as_ref()
            .map(|usage| usage.context_tokens)
            .filter(|tokens| *tokens > 0)
            .or_else(|| metadata_u64(message, "tokens_input"))
            .or_else(|| metadata_usage_u64(message, "prompt_tokens"))
    })
}

fn estimate_tail_content_tokens(messages: &[SessionMessage]) -> Option<u64> {
    let total_chars: usize = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .map(|part| match &part.part_type {
            PartType::Text { text, .. } => text.len(),
            PartType::ToolResult { content, title, .. } => {
                content.len() + title.as_ref().map_or(0, |title| title.len())
            }
            PartType::ToolCall { input, raw, .. } => {
                serde_json::to_string(input).map_or(0, |value| value.len())
                    + raw.as_ref().map_or(0, |value| value.len())
            }
            PartType::Reasoning { text } => text.len(),
            PartType::File {
                url,
                filename,
                mime,
            } => url.len() + filename.len() + mime.len(),
            PartType::Snapshot { content } => content.len(),
            PartType::Patch {
                old_string,
                new_string,
                filepath,
            } => old_string.len() + new_string.len() + filepath.len(),
            PartType::Compaction { summary } => summary.len(),
            PartType::StepFinish { output, .. } => output.as_ref().map_or(0, |value| value.len()),
            PartType::StepStart { name, .. } => name.len(),
            PartType::Agent { name, status } => name.len() + status.len(),
            PartType::Subtask {
                description,
                status,
                ..
            } => description.len() + status.len(),
            PartType::Retry { reason, .. } => reason.len(),
        })
        .sum();

    if total_chars == 0 {
        None
    } else {
        Some((total_chars as u64 / 4).max(1))
    }
}

fn metadata_u64(message: &SessionMessage, key: &str) -> Option<u64> {
    message.metadata.get(key).and_then(|value| value.as_u64())
}

fn metadata_usage_u64(message: &SessionMessage, key: &str) -> Option<u64> {
    message
        .metadata
        .get("usage")
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_u64())
}

type StreamToolResultEntry = (
    String,
    String,
    bool,
    Option<String>,
    Option<HashMap<String, serde_json::Value>>,
    Option<Vec<serde_json::Value>>,
);

#[derive(Default)]
struct SessionStepShared {
    assistant_message_id: Option<String>,
}

fn tool_progress_detail(
    input: &serde_json::Value,
    raw: Option<&str>,
    status: &crate::ToolCallStatus,
) -> Option<String> {
    if let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(raw.to_string());
    }

    match status {
        crate::ToolCallStatus::Pending | crate::ToolCallStatus::Running => {
            if input.is_null() {
                return None;
            }
            if let Some(obj) = input.as_object() {
                if obj.is_empty() {
                    return None;
                }
            }
            if let Some(arr) = input.as_array() {
                if arr.is_empty() {
                    return None;
                }
            }
            if let Some(text) = input.as_str() {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return None;
                }
                return Some(trimmed.to_string());
            }
            Some(input.to_string())
        }
        crate::ToolCallStatus::Completed | crate::ToolCallStatus::Error => None,
    }
}

fn tool_result_detail(title: Option<&str>, content: &str) -> Option<String> {
    match title.map(str::trim).filter(|value| !value.is_empty()) {
        Some(title) => Some(format!("{title}: {content}")),
        None if content.trim().is_empty() => None,
        None => Some(content.to_string()),
    }
}

impl SessionPrompt {
    async fn apply_runtime_workspace_context(&self, session: &mut Session) -> anyhow::Result<()> {
        let project_dir = std::path::PathBuf::from(&session.directory);
        let config_instructions = self
            .config_store
            .as_ref()
            .map(|store| store.config().instructions.clone())
            .unwrap_or_default();
        let mut loader = InstructionLoader::new();
        let instructions = loader.load_all(&project_dir, &config_instructions).await;

        let runtime_instruction_sources = instructions
            .iter()
            .filter_map(|instruction| {
                let path = std::path::PathBuf::from(&instruction.path);
                match instruction.source {
                    InstructionSource::AgentsMd
                    | InstructionSource::ClaudeMd
                    | InstructionSource::ContextMd
                    | InstructionSource::Custom(_) => {
                        if path.starts_with(&project_dir) {
                            Some(RuntimeInstructionSource {
                                path,
                                content: instruction.content.clone(),
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        if runtime_instruction_sources.is_empty() {
            session.remove_metadata("runtime_skill_instructions");
        } else {
            session.insert_metadata(
                "runtime_skill_instructions",
                serde_json::to_value(&runtime_instruction_sources)?,
            );
        }

        let Some(user_msg) = session
            .messages_mut()
            .iter_mut()
            .rfind(|message| matches!(message.role, MessageRole::User))
        else {
            return Ok(());
        };

        if !instructions.is_empty() {
            let merged = InstructionLoader::merge_instructions(&instructions);
            if !merged.trim().is_empty() {
                user_msg.add_text(SystemPrompt::system_reminder(&merged));
            }
            let loaded_paths = instructions
                .iter()
                .map(|instruction| instruction.path.clone())
                .collect::<std::collections::HashSet<_>>();
            Self::store_loaded_instruction_paths(user_msg, loaded_paths);
        }

        Ok(())
    }

    fn apply_runtime_memory_prefetch(
        session: &mut Session,
        packet: Option<&MemoryRetrievalPacket>,
    ) -> anyhow::Result<()> {
        let Some(user_msg) = session
            .messages_mut()
            .iter_mut()
            .rfind(|message| matches!(message.role, MessageRole::User))
        else {
            return Ok(());
        };

        let Some(packet) = packet else {
            user_msg.metadata.remove("memory_prefetch_packet");
            return Ok(());
        };

        user_msg.metadata.insert(
            "memory_prefetch_packet".to_string(),
            serde_json::to_value(packet)?,
        );
        if let Some(reminder) = Self::render_memory_prefetch_reminder(packet) {
            user_msg.add_text(SystemPrompt::system_reminder(&reminder));
        }

        Ok(())
    }

    fn render_memory_prefetch_reminder(packet: &MemoryRetrievalPacket) -> Option<String> {
        if packet.items.is_empty() {
            return None;
        }

        let mut lines = vec!["Turn Memory Recall:".to_string()];
        if let Some(query) = packet
            .query
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            lines.push(format!("- query: {}", query.trim()));
        }
        for item in &packet.items {
            lines.push(format!(
                "- {} [{:?} / {:?}]",
                item.card.title, item.card.kind, item.card.validation_status
            ));
            lines.push(format!("  why: {}", item.why_recalled));
            lines.push(format!("  summary: {}", item.card.summary));
            if let Some(evidence) = item.evidence_summary.as_deref() {
                lines.push(format!("  evidence: {}", evidence));
            }
            if let Some(last_validated_at) = item.card.last_validated_at {
                lines.push(format!("  last_validated_at: {}", last_validated_at));
            }
        }

        Some(lines.join("\n"))
    }

    fn text_from_prompt_parts(parts: &[PartInput]) -> String {
        parts
            .iter()
            .filter_map(|p| match p {
                PartInput::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn truncate_debug_text(value: &str, max_chars: usize) -> String {
        if value.chars().count() <= max_chars {
            return value.to_string();
        }
        let mut out = value.chars().take(max_chars).collect::<String>();
        out.push_str("...[truncated]");
        out
    }

    fn annotate_latest_user_message(
        session: &mut Session,
        input: &PromptInput,
        system_prompt: Option<&str>,
    ) {
        let Some(user_msg) = session
            .messages_mut()
            .iter_mut()
            .rfind(|m| matches!(m.role, MessageRole::User))
        else {
            return;
        };

        if let Some(agent) = input.agent.as_deref() {
            user_msg
                .metadata
                .insert("resolved_agent".to_string(), serde_json::json!(agent));
        }

        if let Some(system) = system_prompt {
            user_msg.metadata.insert(
                "resolved_system_prompt".to_string(),
                serde_json::json!(Self::truncate_debug_text(system, 8000)),
            );
            user_msg.metadata.insert(
                "resolved_system_prompt_applied".to_string(),
                serde_json::json!(true),
            );
        } else if input.agent.is_some() {
            user_msg.metadata.insert(
                "resolved_system_prompt_applied".to_string(),
                serde_json::json!(false),
            );
        }

        let user_prompt = Self::text_from_prompt_parts(&input.parts);
        if !user_prompt.is_empty() {
            user_msg.metadata.insert(
                "resolved_user_prompt".to_string(),
                serde_json::json!(Self::truncate_debug_text(&user_prompt, 8000)),
            );
        }
    }

    fn maybe_append_runtime_skill_save_suggestion(session: &mut Session, turn_start_index: usize) {
        if !turn_looks_skillworthy(session, turn_start_index)
            || turn_used_skill_manage(session, turn_start_index)
        {
            return;
        }

        let note = session.add_assistant_message();
        note.metadata.insert(
            "runtime_hint".to_string(),
            serde_json::json!(HiddenRuntimeHint::SkillSaveSuggestion.as_str()),
        );
        note.add_text(
            "System suggestion: this turn may be a good skill candidate. Save it only if you can express reusable triggers, steps, validation, and boundaries with `skill_manage`.",
        );
    }

    pub fn new(session_state: Arc<RwLock<SessionStateManager>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            session_state,
            mcp_clients: None,
            lsp_registry: None,
            tool_runtime_config: rocode_tool::ToolRuntimeConfig::default(),
            config_store: None,
            memory_authority: None,
            proposal_repo: None,
            review_nudge_state: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_tool_runtime_config(
        mut self,
        tool_runtime_config: rocode_tool::ToolRuntimeConfig,
    ) -> Self {
        self.tool_runtime_config = tool_runtime_config;
        self
    }

    pub fn with_config_store(mut self, config_store: Arc<rocode_config::ConfigStore>) -> Self {
        self.config_store = Some(config_store);
        self
    }

    pub fn with_memory_authority(
        mut self,
        memory_authority: Arc<rocode_memory::MemoryAuthority>,
    ) -> Self {
        self.memory_authority = Some(memory_authority);
        self
    }

    pub fn with_proposal_repo(
        mut self,
        proposal_repo: Arc<rocode_storage::SkillEvolutionProposalRepository>,
    ) -> Self {
        self.proposal_repo = Some(proposal_repo);
        self
    }

    /// Post-run consolidation nudge: if the completed turn produced enough
    /// tool/error/skill signals, run a deterministic memory consolidation
    /// against the workspace repository.
    ///
    /// Trigger conditions (any one is sufficient):
    /// - `skill_write_count >= 1`
    /// - `error_tool_call_count >= 2`
    /// - `tool_call_count >= 5`
    /// - `used_skill_names` non-empty AND `tool_call_count >= 3`
    ///
    /// Cooldown: at most one successful consolidation per workspace/session
    /// scope per 10 minutes, with an in-flight guard to avoid concurrent
    /// duplicate reviews.
    /// Consolidation runs inline (no LLM; pure DB).
    pub async fn maybe_enqueue_background_review(
        &self,
        nudge: &RuntimeReviewNudge,
    ) -> NudgeDecision {
        const MIN_TOOL_CALLS: usize = 5;
        const MIN_TOOL_CALLS_WITH_SKILL: usize = 3;
        const MIN_ERRORS: usize = 2;
        const COOLDOWN: core::time::Duration = core::time::Duration::from_secs(600);

        let triggered = nudge.tool_call_count >= MIN_TOOL_CALLS
            || nudge.error_tool_call_count >= MIN_ERRORS
            || nudge.skill_write_count >= 1
            || (!nudge.used_skill_names.is_empty()
                && nudge.tool_call_count >= MIN_TOOL_CALLS_WITH_SKILL);

        if !triggered {
            return NudgeDecision::Skipped {
                reason: SkippedReason::BelowThreshold,
            };
        }

        let Some(memory) = self.memory_authority.as_deref() else {
            return NudgeDecision::Skipped {
                reason: SkippedReason::MemoryUnavailable,
            };
        };

        if let Err(reason) = self.try_begin_review_nudge_scope(
            &nudge.workspace_key,
            tokio::time::Instant::now(),
            COOLDOWN,
        ) {
            tracing::debug!(
                session_id = %nudge.session_id,
                workspace_key = %nudge.workspace_key,
                reason = ?reason,
                "nudge: skipped"
            );
            return NudgeDecision::Skipped { reason };
        }

        let started = tokio::time::Instant::now();
        tracing::info!(
            session_id = %nudge.session_id,
            workspace_key = %nudge.workspace_key,
            tool_calls = nudge.tool_call_count,
            errors = nudge.error_tool_call_count,
            skill_writes = nudge.skill_write_count,
            "nudge: running consolidation after session turn"
        );

        match memory
            .run_consolidation(&rocode_types::MemoryConsolidationRequest::default())
            .await
        {
            Ok(response) => {
                self.finish_review_nudge_scope(
                    &nudge.workspace_key,
                    Some(tokio::time::Instant::now()),
                );
                let promoted = response.run.promoted_count;
                let merged = response.run.merged_count;
                let archived = response.archived_record_ids.len() as u32;
                let promoted_records = response.promoted_record_ids.len() as u32;
                let elapsed_ms = started.elapsed().as_millis();

                // Fetch promoted records and generate skill evolution proposals.
                let (proposals_created, proposals_skipped) = self
                    .maybe_generate_proposals(
                        memory,
                        &nudge.session_id,
                        &response.promoted_record_ids,
                    )
                    .await;

                if elapsed_ms > 1000 {
                    tracing::warn!(
                        session_id = %nudge.session_id,
                        elapsed_ms,
                        "nudge: slow consolidation"
                    );
                } else if promoted > 0 || merged > 0 || proposals_created > 0 {
                    tracing::info!(
                        session_id = %nudge.session_id,
                        promoted,
                        merged,
                        archived,
                        promoted_records,
                        proposals_created,
                        proposals_skipped,
                        elapsed_ms,
                        "nudge: consolidation completed"
                    );
                }
                NudgeDecision::Triggered {
                    promoted: response.run.promoted_count,
                    merged: response.run.merged_count,
                    archived,
                    promoted_records,
                    proposals_created,
                    proposals_skipped,
                }
            }
            Err(error) => {
                self.finish_review_nudge_scope(&nudge.workspace_key, None);
                tracing::warn!(
                    session_id = %nudge.session_id,
                    workspace_key = %nudge.workspace_key,
                    %error,
                    "nudge: consolidation failed"
                );
                NudgeDecision::Skipped {
                    reason: SkippedReason::ConsolidationFailed,
                }
            }
        }
    }

    fn try_begin_review_nudge_scope(
        &self,
        scope_key: &str,
        now: tokio::time::Instant,
        cooldown: core::time::Duration,
    ) -> Result<(), SkippedReason> {
        let mut states = self
            .review_nudge_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = states.entry(scope_key.to_string()).or_default();
        if state.in_flight {
            return Err(SkippedReason::ReviewInFlight);
        }
        if state
            .last_completed_at
            .is_some_and(|last| now.duration_since(last) < cooldown)
        {
            return Err(SkippedReason::CooldownActive);
        }
        state.in_flight = true;
        Ok(())
    }

    fn finish_review_nudge_scope(
        &self,
        scope_key: &str,
        completed_at: Option<tokio::time::Instant>,
    ) {
        let mut states = self
            .review_nudge_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut remove_scope = false;
        if let Some(state) = states.get_mut(scope_key) {
            state.in_flight = false;
            if let Some(at) = completed_at {
                state.last_completed_at = Some(at);
            } else if state.last_completed_at.is_none() {
                remove_scope = true;
            }
        }
        if remove_scope {
            states.remove(scope_key);
        }
    }

    /// Fetch promoted records from memory, filter to MethodologyCandidates,
    /// and generate SkillEvolutionProposals.
    async fn maybe_generate_proposals(
        &self,
        memory: &rocode_memory::MemoryAuthority,
        session_id: &str,
        promoted_record_ids: &[rocode_types::MemoryRecordId],
    ) -> (u32, u32) {
        let Some(repo) = self.proposal_repo.as_deref() else {
            return (0, 0);
        };

        let mut candidates = Vec::new();
        for record_id in promoted_record_ids {
            match memory.get_memory_detail(record_id).await {
                Ok(Some(detail)) => candidates.push(detail.record),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        session_id,
                        record_id = %record_id.0,
                        %error,
                        "nudge: failed to fetch promoted record for proposal generation"
                    );
                }
            }
        }

        if candidates.is_empty() {
            return (0, 0);
        }

        match rocode_storage::generate_skill_evolution_proposals(repo, &candidates, session_id)
            .await
        {
            Ok(summary) => (summary.proposals_created, summary.proposals_skipped),
            Err(error) => {
                tracing::warn!(
                    session_id,
                    %error,
                    "nudge: proposal generation failed"
                );
                (0, 0)
            }
        }
    }

    pub fn with_mcp_clients(mut self, clients: Arc<rocode_mcp::McpClientRegistry>) -> Self {
        self.mcp_clients = Some(clients);
        self
    }

    pub fn with_lsp_registry(mut self, registry: Arc<rocode_lsp::LspClientRegistry>) -> Self {
        self.lsp_registry = Some(registry);
        self
    }

    pub async fn assert_not_busy(&self, session_id: &str) -> anyhow::Result<()> {
        let state = self.state.lock().await;
        if state.contains_key(session_id) {
            return Err(anyhow::anyhow!("Session {} is busy", session_id));
        }
        Ok(())
    }

    pub async fn reserve_session_run(&self, session_id: &str) -> anyhow::Result<CancellationToken> {
        self.start(session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Session {} is busy", session_id))
    }

    pub async fn release_reserved_session_run(&self, session_id: &str) {
        self.finish_run(session_id).await;
    }

    pub async fn create_user_message(
        &self,
        input: &PromptInput,
        session: &mut Session,
    ) -> anyhow::Result<()> {
        // Collect text parts for the primary message
        let text = input
            .parts
            .iter()
            .filter_map(|p| match p {
                PartInput::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let has_non_text = input
            .parts
            .iter()
            .any(|p| !matches!(p, PartInput::Text { .. }));

        if text.is_empty() && !has_non_text {
            return Err(anyhow::anyhow!("No content in prompt"));
        }

        let project_root = session.directory.clone();

        // Create the user message with text (or empty if only non-text parts)
        let msg = if text.is_empty() {
            session.add_user_message(" ")
        } else {
            session.add_user_message(&text)
        };

        // Add non-text parts to the message
        for part in &input.parts {
            match part {
                PartInput::Text { .. } => {} // already handled above
                PartInput::File {
                    url,
                    filename,
                    mime,
                } => {
                    self.add_file_part(
                        msg,
                        url,
                        filename.as_deref(),
                        mime.as_deref(),
                        &project_root,
                    )
                    .await;
                }
                PartInput::Agent { name } => {
                    msg.add_agent(name.clone());
                    // Add synthetic text instructing the LLM to invoke the agent
                    msg.add_text(format!(
                        "Use the above message and context to generate a prompt and prefer calling task_flow with operation=create and agent=\"{}\". Only fall back to the task tool if task_flow is unavailable in this session.",
                        name
                    ));
                }
                PartInput::Subtask {
                    prompt,
                    description,
                    agent,
                } => {
                    let subtask_id = format!("sub_{}", uuid::Uuid::new_v4());
                    let description = description.clone().unwrap_or_else(|| prompt.clone());
                    msg.add_subtask(subtask_id.clone(), description.clone());
                    let mut pending = msg
                        .metadata
                        .get("pending_subtasks")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    pending.push(serde_json::json!({
                        "id": subtask_id,
                        "agent": agent,
                        "prompt": prompt,
                        "description": description,
                    }));
                    msg.metadata.insert(
                        "pending_subtasks".to_string(),
                        serde_json::Value::Array(pending),
                    );
                }
            }
        }

        Self::annotate_ingress_metadata(msg, input.ingress.as_ref());

        Ok(())
    }

    fn annotate_ingress_metadata(
        msg: &mut crate::SessionMessage,
        ingress: Option<&IngressTurnEnvelope>,
    ) {
        let Some(ingress) = ingress else {
            return;
        };
        msg.metadata.insert(
            "ingress_source".to_string(),
            serde_json::json!(&ingress.source),
        );
        msg.metadata.insert(
            "ingress_turn_id".to_string(),
            serde_json::json!(&ingress.turn_id),
        );
        msg.metadata.insert(
            "ingress_stabilization".to_string(),
            serde_json::json!(&ingress.stabilization),
        );
        if let Some(key) = ingress.idempotency_key.as_deref() {
            msg.metadata.insert(
                "ingress_idempotency_key".to_string(),
                serde_json::json!(key),
            );
        }
        if let Some(context_key) = ingress.context_key.as_deref() {
            msg.metadata.insert(
                "ingress_context_key".to_string(),
                serde_json::json!(context_key),
            );
        }
        if let Some(stage_id) = ingress.scheduler_stage_id.as_deref() {
            msg.metadata.insert(
                "ingress_scheduler_stage_id".to_string(),
                serde_json::json!(stage_id),
            );
        }
    }

    // --- file_parts methods moved to file_parts.rs ---

    async fn start(&self, session_id: &str) -> Option<CancellationToken> {
        let state = self.state.lock().await;
        if state.contains_key(session_id) {
            return None;
        }
        drop(state);

        let token = CancellationToken::new();
        let mut state = self.state.lock().await;
        state.insert(
            session_id.to_string(),
            PromptState {
                cancel_token: token.clone(),
            },
        );
        Some(token)
    }

    async fn resume(&self, session_id: &str) -> Option<CancellationToken> {
        let state = self.state.lock().await;
        state.get(session_id).map(|s| s.cancel_token.clone())
    }

    pub async fn is_running(&self, session_id: &str) -> bool {
        let state = self.state.lock().await;
        state.contains_key(session_id)
    }

    async fn finish_run(&self, session_id: &str) {
        let mut state = self.state.lock().await;
        state.remove(session_id);
        drop(state);

        let mut session_state = self.session_state.write().await;
        session_state.set_idle(session_id);
    }

    pub async fn cancel(&self, session_id: &str) {
        let mut state = self.state.lock().await;
        if let Some(prompt_state) = state.remove(session_id) {
            prompt_state.cancel_token.cancel();
        }

        let mut session_state = self.session_state.write().await;
        session_state.set_idle(session_id);
    }

    pub async fn prompt(
        &self,
        input: PromptInput,
        session: &mut Session,
        provider: Arc<dyn Provider>,
        system_prompt: Option<String>,
        tools: Vec<ToolDefinition>,
        compiled_request: CompiledExecutionRequest,
    ) -> anyhow::Result<()> {
        self.prompt_with_update_hook(
            input,
            session,
            PromptRequestContext {
                provider,
                system_prompt,
                memory_prefetch: None,
                tools,
                tool_source_digests: Vec::new(),
                compiled_request,
                hooks: PromptHooks::default(),
            },
        )
        .await
    }
}

fn turn_looks_complex(session: &Session, turn_start_index: usize) -> bool {
    let slice = session.messages.get(turn_start_index..).unwrap_or(&[]);
    let assistant_count = slice
        .iter()
        .filter(|message| matches!(message.role, MessageRole::Assistant))
        .count();
    let tool_result_count = slice
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter(|part| matches!(part.part_type, PartType::ToolResult { .. }))
        .count();
    assistant_count >= 2 || tool_result_count >= 3
}

#[derive(Default)]
struct TurnSkillSignals {
    assistant_count: usize,
    user_count: usize,
    tool_result_count: usize,
    tool_names: HashSet<String>,
    has_error_signal: bool,
    has_validation_signal: bool,
    has_mutation_signal: bool,
}

fn turn_looks_skillworthy(session: &Session, turn_start_index: usize) -> bool {
    if !turn_looks_complex(session, turn_start_index) {
        return false;
    }

    let signals = collect_turn_skill_signals(session, turn_start_index);
    let tool_kind_count = signals.tool_names.len();

    let has_edit_then_validate = signals.has_mutation_signal && signals.has_validation_signal;
    let has_error_recovery_pattern = signals.has_error_signal
        && (signals.has_validation_signal
            || (signals.has_mutation_signal && signals.assistant_count >= 2));
    let has_user_guided_refinement =
        signals.user_count >= 2 && tool_kind_count >= 2 && signals.tool_result_count >= 3;
    let has_diverse_execution_flow =
        signals.has_mutation_signal && tool_kind_count >= 2 && signals.tool_result_count >= 3;

    has_edit_then_validate
        || has_error_recovery_pattern
        || has_user_guided_refinement
        || has_diverse_execution_flow
}

fn collect_turn_skill_signals(session: &Session, turn_start_index: usize) -> TurnSkillSignals {
    let mut signals = TurnSkillSignals::default();

    for message in session.messages.get(turn_start_index..).unwrap_or(&[]) {
        match message.role {
            MessageRole::Assistant => signals.assistant_count += 1,
            MessageRole::User => signals.user_count += 1,
            _ => {}
        }

        for part in &message.parts {
            match &part.part_type {
                PartType::ToolCall {
                    name,
                    input,
                    status,
                    state,
                    ..
                } => {
                    signals.tool_names.insert(name.clone());
                    signals.has_mutation_signal |= tool_is_mutation(name);
                    signals.has_validation_signal |= tool_is_validation(name, input);
                    signals.has_error_signal |= matches!(status, crate::ToolCallStatus::Error)
                        || matches!(state, Some(crate::ToolState::Error { .. }));
                }
                PartType::ToolResult { is_error, .. } => {
                    signals.tool_result_count += 1;
                    signals.has_error_signal |= *is_error;
                }
                _ => {}
            }
        }
    }

    signals
}

fn tool_is_mutation(name: &str) -> bool {
    matches!(
        name,
        "edit" | "write" | "apply_patch" | "ast_grep_replace" | "skill_manage"
    )
}

fn tool_is_validation(name: &str, input: &serde_json::Value) -> bool {
    if tool_name_looks_validation(name) {
        return true;
    }

    if name != "bash" {
        return false;
    }

    let command = input
        .get("command")
        .or_else(|| input.get("cmd"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    bash_command_looks_validation(command)
}

fn tool_name_looks_validation(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    validation_word_matches(&lower)
        || lower
            .split(|ch: char| !(ch.is_ascii_alphanumeric()))
            .filter(|token| !token.is_empty())
            .any(validation_word_matches)
}

fn bash_command_looks_validation(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();

    if [
        "--dry-run",
        "--check",
        "--verify",
        "--validate",
        "--validation",
        "--audit",
        "--probe",
        "--health-check",
        "--smoke-test",
    ]
    .iter()
    .any(|flag| lower.contains(flag))
    {
        return true;
    }

    let words: Vec<&str> = lower
        .split_whitespace()
        .map(trim_shell_word)
        .filter(|word| !word.is_empty())
        .collect();

    let Some(exec_index) = words.iter().position(|word| !is_shell_wrapper_word(word)) else {
        return false;
    };

    let executable = words[exec_index];
    if validation_word_matches(executable) {
        return true;
    }

    if shell_output_emitter_word(executable) {
        return false;
    }

    words[exec_index + 1..]
        .iter()
        .any(|word| validation_word_matches(word))
}

fn trim_shell_word(word: &str) -> &str {
    word.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
        )
    })
}

fn is_shell_wrapper_word(word: &str) -> bool {
    matches!(word, "env" | "command" | "sudo" | "time")
        || (word.contains('=')
            && !word.starts_with('-')
            && word
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_'))
}

fn shell_output_emitter_word(word: &str) -> bool {
    matches!(
        word,
        "echo"
            | "printf"
            | "cat"
            | "sed"
            | "awk"
            | "jq"
            | "yq"
            | "rg"
            | "grep"
            | "ls"
            | "find"
            | "pwd"
            | "which"
    )
}

fn validation_word_matches(word: &str) -> bool {
    matches!(
        word,
        "test"
            | "tests"
            | "check"
            | "checks"
            | "verify"
            | "verified"
            | "validate"
            | "validation"
            | "audit"
            | "probe"
            | "lint"
            | "diagnostic"
            | "diagnostics"
            | "doctor"
            | "healthcheck"
            | "health-check"
            | "smoketest"
            | "smoke-test"
            | "selftest"
            | "self-test"
    )
}

fn turn_used_skill_manage(session: &Session, turn_start_index: usize) -> bool {
    session
        .messages
        .get(turn_start_index..)
        .unwrap_or(&[])
        .iter()
        .flat_map(|message| message.parts.iter())
        .any(|part| {
            matches!(
                &part.part_type,
                PartType::ToolCall { name, .. } if name == "skill_manage"
            )
        })
}

impl Default for SessionPrompt {
    fn default() -> Self {
        Self::new(Arc::new(RwLock::new(SessionStateManager::new())))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PromptError {
    #[error("Session is busy: {0}")]
    Busy(String),
    #[error("No user message found")]
    NoUserMessage,
    #[error("{0}")]
    ProviderFailure(rocode_orchestrator::runtime::events::ModelFailure),
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptProviderFailure {
    TypedSummary(rocode_provider::ProviderErrorSummary),
    LegacyMessage(String),
}

impl PromptError {
    pub fn provider_failure(&self) -> Option<PromptProviderFailure> {
        match self {
            Self::ProviderFailure(
                rocode_orchestrator::runtime::events::ModelFailure::Provider(summary),
            ) => Some(PromptProviderFailure::TypedSummary(summary.clone())),
            Self::ProviderFailure(rocode_orchestrator::runtime::events::ModelFailure::Message(
                message,
            ))
            | Self::Provider(message) => {
                Some(PromptProviderFailure::LegacyMessage(message.clone()))
            }
            Self::Busy(_) | Self::NoUserMessage | Self::Cancelled => None,
        }
    }

    pub fn provider_error_summary(&self) -> Option<rocode_provider::ProviderErrorSummary> {
        match self.provider_failure()? {
            PromptProviderFailure::TypedSummary(summary) => Some(summary),
            PromptProviderFailure::LegacyMessage(_) => None,
        }
    }
}

pub fn provider_failure_from_anyhow(error: &anyhow::Error) -> Option<PromptProviderFailure> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<PromptError>())?
        .provider_failure()
}

pub fn provider_error_summary_from_anyhow(
    error: &anyhow::Error,
) -> Option<rocode_provider::ProviderErrorSummary> {
    match provider_failure_from_anyhow(error)? {
        PromptProviderFailure::TypedSummary(summary) => Some(summary),
        PromptProviderFailure::LegacyMessage(_) => None,
    }
}

pub fn legacy_provider_error_text_from_anyhow(error: &anyhow::Error) -> Option<String> {
    match provider_failure_from_anyhow(error)? {
        PromptProviderFailure::TypedSummary(_) => None,
        PromptProviderFailure::LegacyMessage(message) => Some(message),
    }
}

/// Regex that matches `@reference` patterns. We use a capturing group for the
/// preceding character instead of a lookbehind (unsupported by the `regex` crate).
/// Group 1 = preceding char (or empty at start of string), Group 2 = the reference name.
const FILE_REFERENCE_REGEX: &str = r"(?:^|([^\w`]))@(\.?[^\s`,.]*(?:\.[^\s`,.]+)*)";

pub async fn resolve_prompt_parts(
    template: &str,
    worktree: &std::path::Path,
    known_agents: &[String],
) -> Vec<PartInput> {
    let mut parts = vec![PartInput::Text {
        text: template.to_string(),
    }];

    let re = regex::Regex::new(FILE_REFERENCE_REGEX).unwrap();
    let mut seen = std::collections::HashSet::new();

    for cap in re.captures_iter(template) {
        // Group 1 is the preceding char — if it matched a word char or backtick
        // the overall pattern wouldn't match (they're excluded by [^\w`]).
        // Group 2 is the actual reference name.
        if let Some(name) = cap.get(2) {
            let name = name.as_str();
            if name.is_empty() || seen.contains(name) {
                continue;
            }
            seen.insert(name.to_string());

            let filepath = if let Some(stripped) = name.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(stripped)
                } else {
                    continue;
                }
            } else {
                worktree.join(name)
            };

            if let Ok(metadata) = tokio::fs::metadata(&filepath).await {
                let url = format!("file://{}", filepath.display());

                if metadata.is_dir() {
                    parts.push(PartInput::File {
                        url,
                        filename: Some(name.to_string()),
                        mime: Some("application/x-directory".to_string()),
                    });
                } else {
                    parts.push(PartInput::File {
                        url,
                        filename: Some(name.to_string()),
                        mime: Some("text/plain".to_string()),
                    });
                }
            } else if known_agents.iter().any(|a| a == name) {
                // Not a file — check if it's a known agent name
                parts.push(PartInput::Agent {
                    name: name.to_string(),
                });
            }
        }
    }

    parts
}

pub fn extract_file_references(template: &str) -> Vec<String> {
    let re = regex::Regex::new(FILE_REFERENCE_REGEX).unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for cap in re.captures_iter(template) {
        if let Some(name) = cap.get(2) {
            let name = name.as_str().to_string();
            if !name.is_empty() && !seen.contains(&name) {
                seen.insert(name.clone());
                result.push(name);
            }
        }
    }

    result
}
