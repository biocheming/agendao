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

pub const PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY: &str = "prompt_surface_state_snapshot";
pub const PROMPT_SURFACE_EVIDENCE_METADATA_KEY: &str = "prompt_surface_evidence";
pub const CONTEXT_COMPACTION_RECORD_METADATA_KEY: &str = "context_compaction_record";
pub const CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY: &str =
    "context_pressure_governance_summary";

pub fn sanctioned_model_context_summary(message: &SessionMessage) -> Option<&str> {
    surface_contract::sanctioned_model_context_projection_for_message(message)
        .map(|projection| projection.summary)
}

pub use compaction_helpers::{should_compact, trigger_compaction};
pub(crate) use hooks::{
    apply_chat_message_hook_outputs, apply_chat_messages_hook_outputs, session_message_hook_payload,
};
pub use ingress::{
    external_adapter_event_to_ingress_turn, normalize_ingress_source, stabilize_ingress_turns,
    ExternalAdapterIngressMappingError, IngressAttachmentRef, IngressSource,
    IngressStabilizationMetadata, IngressTurnEnvelope, INGRESS_POLICY_ENTRY_METADATA_ONLY,
    INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY, INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH,
    INGRESS_POLICY_SCHEDULER_METADATA_ONLY, INGRESS_POLICY_UNSPECIFIED,
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use rocode_content::output_blocks::OutputBlock;
use rocode_execution_types::CompiledExecutionRequest;
use rocode_provider::{cache::CacheEvidenceSummary, Provider, ToolDefinition};
use rocode_skill::{infer_runtime_skill_names, RuntimeInstructionSource, SkillGovernanceAuthority};
use rocode_types::SkillRuntimeCompositionHintKind;
use rocode_types::{
    context_usage_percent, ContextCompactionSummary, ContextPressureGovernanceStatus,
    ContextPressureGovernanceSummary, MemoryRetrievalPacket, PromptSurfaceEvidenceSummary,
    SessionCacheBoundaryKind, SessionCacheBoundarySummary, SessionCacheEvidenceExplain,
    SessionCacheSemanticsBasis, SessionCacheSemanticsSummary, SessionCacheSeverity,
    SessionContextExplain, SessionContextKind, SubsessionHandoffPacket, SubsessionResultEnvelope,
};

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
    #[serde(default = "default_persisted_subsession_kind")]
    kind: SessionContextKind,
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

fn default_persisted_subsession_kind() -> SessionContextKind {
    SessionContextKind::DelegatedSubsession
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PersistedSubsessionTurn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff: Option<SubsessionHandoffPacket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<SubsessionResultEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<String>,
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
    pub workspace_directory: Option<String>,
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
            workspace_directory: normalized_nudge_workspace_directory(session),
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

fn normalized_nudge_workspace_directory(session: &Session) -> Option<String> {
    let directory = session.directory.trim();
    (!directory.is_empty()).then(|| directory.to_string())
}

fn normalize_linked_skill_name(skill_name: &str) -> String {
    skill_name.trim().to_ascii_lowercase()
}

fn linked_skill_memory_promotion_counts(
    candidates: &[rocode_types::MemoryRecord],
) -> BTreeMap<String, (String, u64)> {
    let mut counts = BTreeMap::<String, (String, u64)>::new();
    for record in candidates {
        let Some(skill_name) = record
            .linked_skill_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let key = normalize_linked_skill_name(skill_name);
        let entry = counts
            .entry(key)
            .or_insert_with(|| (skill_name.to_string(), 0));
        entry.1 += 1;
    }
    counts
}

fn linked_methodology_skill_names(
    candidates: &[rocode_types::MemoryRecord],
) -> BTreeMap<String, String> {
    let mut skill_names = BTreeMap::new();
    for record in candidates {
        if record.kind != rocode_types::MemoryKind::MethodologyCandidate {
            continue;
        }
        let Some(skill_name) = record
            .linked_skill_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        skill_names.insert(
            normalize_linked_skill_name(skill_name),
            skill_name.to_string(),
        );
    }
    skill_names
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
    if !session.context_kind().owns_prompt_continuity() {
        return None;
    }
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
    request_context_tokens: Option<u64>,
    focus: Option<&str>,
    trigger: &str,
    phase: Option<&str>,
) -> Option<String> {
    if !session.context_kind().owns_prompt_continuity() {
        return None;
    }
    let filtered = SessionPrompt::filter_compacted_messages(&session.messages);
    let compaction_config = SessionPrompt::runtime_compaction_config(config_store);
    let assessment = SessionPrompt::assess_compaction(
        &filtered,
        provider,
        model_id,
        max_output_tokens,
        &compaction_config,
        live_context_tokens,
        request_context_tokens,
        None,
    )?;
    let record = SessionPrompt::build_compaction_record(
        trigger,
        phase,
        Some(assessment.reason),
        false,
        request_context_tokens,
        live_context_tokens,
        assessment.limit_tokens,
        assessment.body_chars,
    );
    SessionPrompt::trigger_compaction_with_record(session, &filtered, focus, Some(record), false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextPressureGovernanceOutcome {
    Proceed(ContextPressureGovernanceSummary),
    Blocked(ContextPressureGovernanceSummary),
}

fn persist_context_pressure_governance_summary(
    session: &mut Session,
    summary: &ContextPressureGovernanceSummary,
) {
    if let Ok(value) = serde_json::to_value(summary) {
        session.insert_metadata(
            CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY.to_string(),
            value,
        );
    }
}

pub fn record_context_pressure_governance_summary(
    session: &mut Session,
    summary: &ContextPressureGovernanceSummary,
) {
    persist_context_pressure_governance_summary(session, summary);
}

fn should_block_pre_dispatch_governance(
    reason: &str,
    request_pressure_percent: Option<u64>,
    live_pressure_percent: Option<u64>,
) -> bool {
    matches!(
        reason,
        "usage_overflow"
            | "live_context_overflow"
            | "request_view_overflow"
            | "session_content_overflow"
            | "request_body_too_large"
    ) || request_pressure_percent
        .map(|percent| percent >= rocode_types::CONTEXT_PRESSURE_CRITICAL_PERCENT)
        .unwrap_or(false)
        || live_pressure_percent
            .map(|percent| percent >= rocode_types::CONTEXT_PRESSURE_CRITICAL_PERCENT)
            .unwrap_or(false)
}

fn request_view_metrics_for_governance(
    session: &Session,
    fallback_request_context_tokens: Option<u64>,
    fallback_body_chars: Option<usize>,
) -> (Option<u64>, Option<usize>) {
    let explain = explain_session_context(session, None);
    (
        explain
            .api_view_estimated_input_tokens
            .or(fallback_request_context_tokens),
        explain.api_view_body_chars.or(fallback_body_chars),
    )
}

fn context_pressure_governance_summary(
    trigger: &str,
    phase: &str,
    status: ContextPressureGovernanceStatus,
    reason: Option<&str>,
    request_context_tokens: Option<u64>,
    live_context_tokens: Option<u64>,
    limit_tokens: Option<u64>,
    body_chars: Option<usize>,
    compaction_attempted: bool,
    compaction_succeeded: bool,
    blocking: bool,
) -> ContextPressureGovernanceSummary {
    ContextPressureGovernanceSummary {
        trigger: trigger.to_string(),
        phase: phase.to_string(),
        status,
        reason: reason.map(str::to_string),
        request_context_tokens,
        live_context_tokens,
        limit_tokens,
        body_chars,
        request_pressure_percent: request_context_tokens
            .zip(limit_tokens)
            .and_then(|(used, limit)| context_usage_percent(used, limit)),
        live_pressure_percent: live_context_tokens
            .zip(limit_tokens)
            .and_then(|(used, limit)| context_usage_percent(used, limit)),
        compaction_attempted,
        compaction_succeeded,
        blocking,
    }
}

pub fn assess_request_view_context_governance(
    provider: &dyn rocode_provider::Provider,
    model_id: &str,
    max_output_tokens: Option<u64>,
    config_store: Option<&rocode_config::ConfigStore>,
    live_context_tokens: Option<u64>,
    request_context_tokens: Option<u64>,
    request_body_chars: Option<usize>,
    trigger: &str,
    phase: &str,
    compaction_attempted: bool,
    compaction_succeeded: bool,
) -> ContextPressureGovernanceSummary {
    let compaction_config = SessionPrompt::runtime_compaction_config(config_store);
    let assessment = SessionPrompt::assess_compaction(
        &[],
        provider,
        model_id,
        max_output_tokens,
        &compaction_config,
        live_context_tokens,
        request_context_tokens,
        request_body_chars,
    );

    match assessment {
        Some(assessment) => {
            let blocking = compaction_attempted
                && should_block_pre_dispatch_governance(
                    assessment.reason,
                    request_context_tokens
                        .zip(assessment.limit_tokens)
                        .and_then(|(used, limit)| context_usage_percent(used, limit)),
                    live_context_tokens
                        .zip(assessment.limit_tokens)
                        .and_then(|(used, limit)| context_usage_percent(used, limit)),
                );
            context_pressure_governance_summary(
                trigger,
                phase,
                if blocking {
                    ContextPressureGovernanceStatus::Blocked
                } else if compaction_attempted && compaction_succeeded {
                    ContextPressureGovernanceStatus::Compacted
                } else {
                    ContextPressureGovernanceStatus::Deferred
                },
                Some(assessment.reason),
                request_context_tokens,
                live_context_tokens,
                assessment.limit_tokens,
                assessment.body_chars.or(request_body_chars),
                compaction_attempted,
                compaction_succeeded,
                blocking,
            )
        }
        None => context_pressure_governance_summary(
            trigger,
            phase,
            if compaction_attempted && compaction_succeeded {
                ContextPressureGovernanceStatus::Compacted
            } else {
                ContextPressureGovernanceStatus::Ready
            },
            None,
            request_context_tokens,
            live_context_tokens,
            None,
            request_body_chars,
            compaction_attempted,
            compaction_succeeded,
            false,
        ),
    }
}

pub fn govern_pre_dispatch_session_context(
    session: &mut Session,
    provider: &dyn rocode_provider::Provider,
    model_id: &str,
    max_output_tokens: Option<u64>,
    config_store: Option<&rocode_config::ConfigStore>,
    live_context_tokens: Option<u64>,
    request_context_tokens: Option<u64>,
    request_body_chars: Option<usize>,
    focus: Option<&str>,
    trigger: &str,
    phase: &str,
) -> ContextPressureGovernanceOutcome {
    let live_context_tokens =
        live_context_tokens.or_else(|| estimate_current_context_tokens(&session.record().messages));
    if !session.context_kind().owns_prompt_continuity() {
        let summary = context_pressure_governance_summary(
            trigger,
            phase,
            ContextPressureGovernanceStatus::Ready,
            None,
            request_context_tokens,
            live_context_tokens,
            None,
            request_body_chars,
            false,
            false,
            false,
        );
        persist_context_pressure_governance_summary(session, &summary);
        return ContextPressureGovernanceOutcome::Proceed(summary);
    }

    let filtered = SessionPrompt::filter_compacted_messages(&session.record().messages);
    let compaction_config = SessionPrompt::runtime_compaction_config(config_store);
    let Some(assessment) = SessionPrompt::assess_compaction(
        &filtered,
        provider,
        model_id,
        max_output_tokens,
        &compaction_config,
        live_context_tokens,
        request_context_tokens,
        request_body_chars,
    ) else {
        let summary = context_pressure_governance_summary(
            trigger,
            phase,
            ContextPressureGovernanceStatus::Ready,
            None,
            request_context_tokens,
            live_context_tokens,
            None,
            request_body_chars,
            false,
            false,
            false,
        );
        persist_context_pressure_governance_summary(session, &summary);
        return ContextPressureGovernanceOutcome::Proceed(summary);
    };

    let record = SessionPrompt::build_compaction_record(
        trigger,
        Some(phase),
        Some(assessment.reason),
        false,
        request_context_tokens,
        live_context_tokens,
        assessment.limit_tokens,
        assessment.body_chars.or(request_body_chars),
    );
    let compacted = SessionPrompt::trigger_compaction_with_record(
        session,
        &filtered,
        focus,
        Some(record),
        false,
    )
    .is_some();

    let (request_context_tokens, request_body_chars, live_context_tokens, reassessment) =
        if compacted {
            let filtered = SessionPrompt::filter_compacted_messages(&session.record().messages);
            let live_context_tokens =
                estimate_current_context_tokens(&session.record().messages).or(live_context_tokens);
            let (request_context_tokens, request_body_chars) = request_view_metrics_for_governance(
                session,
                request_context_tokens,
                request_body_chars,
            );
            let reassessment = SessionPrompt::assess_compaction(
                &filtered,
                provider,
                model_id,
                max_output_tokens,
                &compaction_config,
                live_context_tokens,
                request_context_tokens,
                request_body_chars,
            );
            (
                request_context_tokens,
                request_body_chars,
                live_context_tokens,
                reassessment,
            )
        } else {
            (
                request_context_tokens,
                request_body_chars,
                live_context_tokens,
                Some(assessment.clone()),
            )
        };

    let summary = if let Some(assessment) = reassessment {
        let blocking = should_block_pre_dispatch_governance(
            assessment.reason,
            request_context_tokens
                .zip(assessment.limit_tokens)
                .and_then(|(used, limit)| context_usage_percent(used, limit)),
            live_context_tokens
                .zip(assessment.limit_tokens)
                .and_then(|(used, limit)| context_usage_percent(used, limit)),
        );
        context_pressure_governance_summary(
            trigger,
            phase,
            if blocking {
                ContextPressureGovernanceStatus::Blocked
            } else if compacted {
                ContextPressureGovernanceStatus::Compacted
            } else {
                ContextPressureGovernanceStatus::Deferred
            },
            Some(assessment.reason),
            request_context_tokens,
            live_context_tokens,
            assessment.limit_tokens,
            assessment.body_chars.or(request_body_chars),
            true,
            compacted,
            blocking,
        )
    } else {
        context_pressure_governance_summary(
            trigger,
            phase,
            ContextPressureGovernanceStatus::Compacted,
            Some(assessment.reason),
            request_context_tokens,
            live_context_tokens,
            assessment.limit_tokens,
            assessment.body_chars.or(request_body_chars),
            true,
            true,
            false,
        )
    };
    persist_context_pressure_governance_summary(session, &summary);

    if summary.blocking {
        ContextPressureGovernanceOutcome::Blocked(summary)
    } else {
        ContextPressureGovernanceOutcome::Proceed(summary)
    }
}

pub fn estimate_current_context_tokens(messages: &[SessionMessage]) -> Option<u64> {
    let filtered = SessionPrompt::filter_compacted_messages(messages);
    latest_prompt_input_tokens(&filtered).or_else(|| estimate_tail_content_tokens(&filtered))
}

pub fn explain_session_context(
    session: &Session,
    workflow_cumulative_tokens: Option<u64>,
) -> SessionContextExplain {
    let record = session.record();
    let provider_id = record
        .metadata
        .get("model_provider")
        .and_then(|value| value.as_str())
        .unwrap_or("default");
    let model_id = record
        .metadata
        .get("model_id")
        .and_then(|value| value.as_str())
        .unwrap_or("default");
    let raw_history_messages = record.messages.len();
    let raw_model_visible_messages = record
        .messages
        .iter()
        .filter(|message| SessionPrompt::is_model_visible_message(message))
        .count();
    let filtered = SessionPrompt::filter_compacted_messages(&record.messages);
    let message_with_parts =
        SessionPrompt::to_message_with_parts(&filtered, provider_id, model_id, &record.directory);
    let api_view_messages = crate::message_v2::to_model_messages(
        &message_with_parts,
        &crate::message_v2::ModelContext {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            api_npm: String::new(),
            api_id: model_id.to_string(),
        },
    );
    let (api_view_estimated_input_tokens, api_view_body_chars) =
        SessionPrompt::estimate_request_context_tokens_from_provider_messages(&api_view_messages);
    let usage = session.get_usage();
    let resolved_model = (provider_id != "default" || model_id != "default")
        .then(|| format!("{provider_id}/{model_id}"));

    SessionContextExplain {
        resolved_model,
        fork: session.fork_explain(),
        raw_history_messages,
        raw_model_visible_messages,
        api_view_messages: api_view_messages.len(),
        api_view_estimated_input_tokens,
        api_view_body_chars: (api_view_body_chars > 0).then_some(api_view_body_chars),
        live_context_tokens: usage.live_context_tokens(),
        last_request_context_tokens: session.latest_request_context_tokens(),
        owner_session_cumulative_tokens: usage.session_cumulative_tokens(),
        workflow_cumulative_tokens: workflow_cumulative_tokens
            .unwrap_or_else(|| usage.session_cumulative_tokens()),
    }
}

pub fn explain_session_cache_semantics(
    context_explain: &SessionContextExplain,
    context_compaction_summary: Option<&ContextCompactionSummary>,
    cache_evidence: Option<&CacheEvidenceSummary>,
    prompt_surface_evidence: Option<&PromptSurfaceEvidenceSummary>,
) -> SessionCacheSemanticsSummary {
    let trimmed_model_visible_messages = context_explain
        .raw_model_visible_messages
        .saturating_sub(context_explain.api_view_messages);
    let boundary = context_compaction_summary.map(|summary| {
        let likely_changed_prefix =
            trimmed_model_visible_messages > 0 || summary.compacted_message_count.unwrap_or(0) > 0;
        let possible_cache_evidence = likely_changed_prefix
            && cache_evidence
                .map(|summary| {
                    session_cache_severity_from_provider(summary.severity)
                        >= SessionCacheSeverity::MediumChange
                        && summary.primary_cause.as_deref().is_some_and(|cause| {
                            cause.contains("prefix changed before the stable boundary")
                        })
                })
                .unwrap_or(false);

        SessionCacheBoundarySummary {
            kind: SessionCacheBoundaryKind::Compaction,
            trigger: summary.trigger.clone(),
            phase: summary.phase.clone(),
            reason: summary.reason.clone(),
            message_count_before: summary.message_count_before,
            compacted_message_count: summary.compacted_message_count,
            kept_message_count: summary.kept_message_count,
            trimmed_model_visible_messages,
            likely_changed_prefix,
            possible_cache_evidence,
        }
    });
    let cache_evidence = cache_evidence.map(|summary| SessionCacheEvidenceExplain {
        status: summary.status.clone(),
        severity: session_cache_severity_from_provider(summary.severity),
        primary_cause: summary.primary_cause.clone(),
        change_count: summary.change_count,
    });
    let prompt_surface_evidence = prompt_surface_evidence.cloned();
    let label = cache_semantics_label(
        boundary.as_ref(),
        cache_evidence.as_ref(),
        prompt_surface_evidence.as_ref(),
    );

    SessionCacheSemanticsSummary {
        basis: SessionCacheSemanticsBasis::ApiView,
        api_view_messages: context_explain.api_view_messages,
        trimmed_model_visible_messages,
        boundary,
        cache_evidence,
        prompt_surface_evidence,
        label,
    }
}

fn session_cache_severity_from_provider(
    value: rocode_provider::cache::CacheEvidenceSeverity,
) -> SessionCacheSeverity {
    match value {
        rocode_provider::cache::CacheEvidenceSeverity::Stable => SessionCacheSeverity::Stable,
        rocode_provider::cache::CacheEvidenceSeverity::LowChange => SessionCacheSeverity::LowChange,
        rocode_provider::cache::CacheEvidenceSeverity::MediumChange => {
            SessionCacheSeverity::MediumChange
        }
        rocode_provider::cache::CacheEvidenceSeverity::HighChange => {
            SessionCacheSeverity::HighChange
        }
    }
}

fn cache_semantics_label(
    boundary: Option<&SessionCacheBoundarySummary>,
    cache_evidence: Option<&SessionCacheEvidenceExplain>,
    prompt_surface_evidence: Option<&PromptSurfaceEvidenceSummary>,
) -> Option<String> {
    if let Some(cache_evidence) = cache_evidence {
        if should_surface_cache_evidence(cache_evidence) {
            let cause = if boundary.is_some_and(|boundary| boundary.possible_cache_evidence) {
                "boundary recorded · prefix changed".to_string()
            } else {
                cache_evidence
                    .primary_cause
                    .as_deref()
                    .map(cache_semantics_evidence_detail_label)
                    .unwrap_or_else(|| "surface changed".to_string())
            };
            return Some(cause);
        }
    }

    if let Some(evidence) = prompt_surface_evidence {
        if evidence.severity > SessionCacheSeverity::Stable {
            let reason = cache_semantics_evidence_detail_label(&evidence.reason);
            if !reason.is_empty() {
                return Some(reason);
            }
        }
    }

    let boundary = boundary?;
    if boundary.likely_changed_prefix {
        if boundary.trimmed_model_visible_messages > 0 {
            return Some(format!(
                "boundary recorded · {} earlier messages trimmed from the API view",
                boundary.trimmed_model_visible_messages
            ));
        }

        return Some("boundary recorded · session compacted before the next request".to_string());
    }

    None
}

fn should_surface_cache_evidence(summary: &SessionCacheEvidenceExplain) -> bool {
    !matches!(summary.status.as_str(), "stable" | "cold_start")
        && summary.severity > SessionCacheSeverity::Stable
}

fn cache_semantics_evidence_detail_label(detail: &str) -> String {
    let normalized = detail.trim();
    if normalized.is_empty() {
        return "surface changed".to_string();
    }

    if let Some(field_list) = normalized.strip_prefix("surface changed:") {
        let fields = field_list.trim();
        return if fields.is_empty() {
            "surface changed".to_string()
        } else {
            format!("surface changed · {}", fields)
        };
    }

    normalized.to_string()
}

#[cfg(test)]
mod cache_semantics_tests {
    use super::{compact_session_now, explain_session_cache_semantics};
    use crate::Session;
    use rocode_provider::cache::{CacheEvidenceSeverity, CacheEvidenceSummary};
    use rocode_types::{
        ContextCompactionSummary, PromptSurfaceEvidenceSummary, SessionCacheSeverity,
        SessionContextExplain,
    };

    #[test]
    fn cache_semantics_marks_compact_boundary_as_possible_bust() {
        let explain = SessionContextExplain {
            resolved_model: Some("openai/gpt-4o".to_string()),
            fork: None,
            raw_history_messages: 18,
            raw_model_visible_messages: 15,
            api_view_messages: 8,
            api_view_estimated_input_tokens: Some(92_000),
            api_view_body_chars: Some(360_000),
            live_context_tokens: Some(82_000),
            last_request_context_tokens: Some(88_000),
            owner_session_cumulative_tokens: 104_000,
            workflow_cumulative_tokens: 143_000,
        };
        let compaction = ContextCompactionSummary {
            trigger: "auto_preflight".to_string(),
            phase: Some("prompt.pre_request".to_string()),
            reason: Some("request_view_threshold".to_string()),
            forced: false,
            request_context_tokens: Some(92_000),
            live_context_tokens: Some(82_000),
            limit_tokens: Some(100_000),
            body_chars: Some(360_000),
            message_count_before: Some(15),
            compacted_message_count: Some(7),
            kept_message_count: Some(8),
            summary: Some("Compacted 7 messages.".to_string()),
        };
        let cache_evidence = CacheEvidenceSummary {
            status: "degraded".to_string(),
            severity: CacheEvidenceSeverity::MediumChange,
            primary_cause: Some("prefix changed before the stable boundary".to_string()),
            change_count: 1,
        };

        let summary = explain_session_cache_semantics(
            &explain,
            Some(&compaction),
            Some(&cache_evidence),
            None,
        );

        assert_eq!(
            summary.basis,
            rocode_types::SessionCacheSemanticsBasis::ApiView
        );
        assert_eq!(summary.trimmed_model_visible_messages, 7);
        assert!(summary
            .boundary
            .as_ref()
            .is_some_and(|boundary| boundary.possible_cache_evidence));
        assert_eq!(
            summary.label.as_deref(),
            Some("boundary recorded · prefix changed")
        );
    }

    #[test]
    fn cache_semantics_falls_back_to_prompt_surface_evidence() {
        let explain = SessionContextExplain {
            resolved_model: None,
            fork: None,
            raw_history_messages: 4,
            raw_model_visible_messages: 4,
            api_view_messages: 4,
            api_view_estimated_input_tokens: Some(8_000),
            api_view_body_chars: Some(32_000),
            live_context_tokens: Some(8_000),
            last_request_context_tokens: Some(8_000),
            owner_session_cumulative_tokens: 9_000,
            workflow_cumulative_tokens: 9_000,
        };
        let evidence = PromptSurfaceEvidenceSummary {
            severity: SessionCacheSeverity::LowChange,
            reason: "surface changed: ingressPolicyHash".to_string(),
            changed_fields: vec!["ingressPolicyHash".to_string()],
        };

        let summary = explain_session_cache_semantics(&explain, None, None, Some(&evidence));

        assert_eq!(
            summary.label.as_deref(),
            Some("surface changed · ingressPolicyHash")
        );
        assert_eq!(
            summary
                .prompt_surface_evidence
                .as_ref()
                .map(|value| value.changed_fields.clone()),
            Some(vec!["ingressPolicyHash".to_string()])
        );
    }

    #[test]
    fn compact_session_now_skips_stage_output_sinks() {
        let parent = Session::new("proj", ".");
        let mut child = Session::attached_with_context_kind(
            &parent,
            rocode_types::SessionContextKind::SchedulerStageOutputSession,
        );
        child.add_user_message("hello");
        child.add_assistant_message().add_text("world");

        let summary = compact_session_now(&mut child);

        assert!(summary.is_none());
        assert_eq!(child.record().messages.len(), 2);
    }
}

fn latest_prompt_input_tokens(messages: &[SessionMessage]) -> Option<u64> {
    messages.iter().rev().find_map(|message| {
        if !matches!(message.role, MessageRole::Assistant) {
            return None;
        }

        message
            .usage
            .as_ref()
            .and_then(|usage| usage.live_context_tokens())
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
        let workspace_directory =
            (!session.directory.trim().is_empty()).then(|| session.directory.clone());

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
            if let Some(reminder) = self.render_runtime_skill_composition_reminder(
                workspace_directory.as_deref(),
                &project_dir,
                &runtime_instruction_sources,
            ) {
                user_msg.add_text(SystemPrompt::system_reminder(&reminder));
            }
            let loaded_paths = instructions
                .iter()
                .map(|instruction| instruction.path.clone())
                .collect::<std::collections::HashSet<_>>();
            Self::store_loaded_instruction_paths(user_msg, loaded_paths);
        }

        Ok(())
    }

    fn render_runtime_skill_composition_reminder(
        &self,
        workspace_directory: Option<&str>,
        project_dir: &std::path::Path,
        runtime_instruction_sources: &[RuntimeInstructionSource],
    ) -> Option<String> {
        if runtime_instruction_sources.is_empty() {
            return None;
        }
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return None;
        };

        let skill_names = infer_runtime_skill_names(project_dir, runtime_instruction_sources);
        if skill_names.is_empty() {
            return None;
        }

        let hints = governance.runtime_skill_composition_hints(&skill_names);
        if hints.is_empty() {
            return None;
        }

        let mut lines = vec![
            "Runtime Skill Governance:".to_string(),
            "- The following hints come from accepted composition relationships and active capability groups.".to_string(),
        ];
        for hint in hints {
            let label = match hint.kind {
                SkillRuntimeCompositionHintKind::PreferCanonicalSkill => "prefer canonical",
                SkillRuntimeCompositionHintKind::ComplementaryBundle => "keep complementary",
            };
            lines.push(format!("- {label}: {}", hint.summary));
        }
        Some(lines.join("\n"))
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
                        nudge.workspace_directory.as_deref(),
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
        workspace_directory: Option<&str>,
        promoted_record_ids: &[rocode_types::MemoryRecordId],
    ) -> (u32, u32) {
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

        self.sync_skill_memory_promotion_evidence(workspace_directory, session_id, &candidates);

        let Some(repo) = self.proposal_repo.as_deref() else {
            return (0, 0);
        };
        let proposal_candidates = self.retarget_methodology_candidates_for_composition(
            workspace_directory,
            session_id,
            &candidates,
        );
        let linked_methodology_skills = linked_methodology_skill_names(&proposal_candidates);

        match rocode_storage::generate_skill_evolution_proposals(
            repo,
            &proposal_candidates,
            session_id,
        )
        .await
        {
            Ok(summary) => {
                self.sync_skill_proposal_evidence(
                    workspace_directory,
                    session_id,
                    repo,
                    &linked_methodology_skills,
                )
                .await;
                (summary.proposals_created, summary.proposals_skipped)
            }
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

    fn retarget_methodology_candidates_for_composition(
        &self,
        workspace_directory: Option<&str>,
        session_id: &str,
        candidates: &[rocode_types::MemoryRecord],
    ) -> Vec<rocode_types::MemoryRecord> {
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return candidates.to_vec();
        };

        let mut rewritten = Vec::with_capacity(candidates.len());
        for record in candidates {
            if record.kind != rocode_types::MemoryKind::MethodologyCandidate {
                rewritten.push(record.clone());
                continue;
            }

            let target = record
                .linked_skill_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .and_then(|skill_name| governance.skill_composition_proposal_target(skill_name))
                .or_else(|| {
                    record
                        .derived_skill_name
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .and_then(|skill_name| {
                            governance.skill_composition_proposal_target(skill_name)
                        })
                });

            let Some(target_skill_name) = target else {
                rewritten.push(record.clone());
                continue;
            };
            if record
                .linked_skill_name
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case(&target_skill_name))
                .unwrap_or(false)
            {
                rewritten.push(record.clone());
                continue;
            }

            tracing::debug!(
                session_id,
                record_id = %record.id.0,
                previous_linked_skill_name = ?record.linked_skill_name,
                derived_skill_name = ?record.derived_skill_name,
                target_skill_name = %target_skill_name,
                "nudge: retargeting methodology candidate to canonical composition proposal target"
            );

            let mut rewritten_record = record.clone();
            rewritten_record.linked_skill_name = Some(target_skill_name);
            rewritten.push(rewritten_record);
        }

        rewritten
    }

    fn sync_skill_memory_promotion_evidence(
        &self,
        workspace_directory: Option<&str>,
        session_id: &str,
        candidates: &[rocode_types::MemoryRecord],
    ) {
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return;
        };

        for (_key, (skill_name, count)) in linked_skill_memory_promotion_counts(candidates) {
            if let Err(error) = governance.record_skill_memory_promotion_signal(&skill_name, count)
            {
                tracing::warn!(
                    session_id,
                    skill_name = %skill_name,
                    %error,
                    "nudge: failed to sync skill memory promotion evidence"
                );
            }
        }
    }

    async fn sync_skill_proposal_evidence(
        &self,
        workspace_directory: Option<&str>,
        session_id: &str,
        repo: &rocode_storage::SkillEvolutionProposalRepository,
        linked_methodology_skills: &BTreeMap<String, String>,
    ) {
        if linked_methodology_skills.is_empty() {
            return;
        }
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return;
        };

        let draft_proposals = match repo
            .list_by_status(&rocode_types::ProposalStatus::Draft)
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(
                    session_id,
                    %error,
                    "nudge: failed to inspect draft proposal state for skill governance"
                );
                return;
            }
        };

        let linked_keys = linked_methodology_skills
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut draft_counts = BTreeMap::<String, u64>::new();
        for proposal in draft_proposals {
            let Some(skill_name) = proposal.linked_skill_name.as_deref() else {
                continue;
            };
            let key = normalize_linked_skill_name(skill_name);
            if linked_keys.contains(&key) {
                *draft_counts.entry(key).or_default() += 1;
            }
        }

        for (key, skill_name) in linked_methodology_skills {
            let draft_count = draft_counts.get(key).copied().unwrap_or(0);
            if let Err(error) = governance.record_skill_proposal_signal(skill_name, draft_count) {
                tracing::warn!(
                    session_id,
                    skill_name = %skill_name,
                    %error,
                    "nudge: failed to sync skill proposal evidence"
                );
            }
        }
    }

    fn skill_governance_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> Option<SkillGovernanceAuthority> {
        let directory = workspace_directory
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        Some(SkillGovernanceAuthority::new(
            PathBuf::from(directory),
            self.config_store.clone(),
        ))
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
    UntypedMessage(String),
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
                Some(PromptProviderFailure::UntypedMessage(message.clone()))
            }
            Self::Busy(_) | Self::NoUserMessage | Self::Cancelled => None,
        }
    }

    pub fn provider_error_summary(&self) -> Option<rocode_provider::ProviderErrorSummary> {
        match self.provider_failure()? {
            PromptProviderFailure::TypedSummary(summary) => Some(summary),
            PromptProviderFailure::UntypedMessage(_) => None,
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
        PromptProviderFailure::UntypedMessage(_) => None,
    }
}

pub fn untyped_provider_error_text_from_anyhow(error: &anyhow::Error) -> Option<String> {
    match provider_failure_from_anyhow(error)? {
        PromptProviderFailure::TypedSummary(_) => None,
        PromptProviderFailure::UntypedMessage(message) => Some(message),
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
