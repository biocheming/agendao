mod file_parts;
pub mod ingress;
mod ingress_metadata;
mod message_building;
pub(crate) mod reflow_context;
mod surface_contract;
#[cfg(test)]
mod tests;
pub mod tools_and_output;

// ── Metadata Key Observability Registry (AgenDao §10, P2.2) ──────────
//
// Each key MUST have a writer, reader, and displayer.  "只写不读" is a
// governance violation.  This table is the single audit reference.
//
// The retired direct prompt loop (loop_lifecycle and its pressure/boundary
// governance chain) used to write several of these keys; those writers were
// removed with the scheduler-native migration.  Keys whose writers are gone
// are kept read-only: they may still appear in sessions persisted by older
// builds, and readers/displayer fallbacks handle their absence.
//
// Key                                    | Writer(s)                     | Reader(s)                        | Displayer(s)                 | Fallback
// ---------------------------------------|-------------------------------|----------------------------------|------------------------------|----------
// prompt_surface_state_snapshot           | (retired; legacy sessions)    | session_artifact, telemetry      | TUI/Web diagnostics          | missing → no snapshot in sidecar
// prompt_surface_evidence                | (retired; legacy sessions)    | session_artifact, cache_semantics| TUI status panels, API       | missing → "surface changed"
// context_compaction_record              | message_building (compaction) | session_artifact, telemetry      | TUI/Web diagnostics          | missing → no compaction visible
// context_compaction_continuity_packet   | message_building (compaction) | message_building (filter),       | TUI/Web diagnostics,         | missing/invalid → reject compacted view
//                                        |                               | session_artifact, scheduler      | scheduler hydrate            |
// context_compaction_lifecycle_summary   | (retired; legacy sessions)    | session_artifact, telemetry,     | TUI status/input pipeline,   | missing → no lifecycle display
//                                        |                               | TUI (input_pipeline, status)     | API                          |
// context_pressure_governance_summary    | (retired; legacy sessions)    | session_artifact, telemetry,     | TUI status panels, API       | missing → no pressure display
//                                        |                               | server session_runtime           |                              |
// context_lightweight_trim_summary       | (retired; legacy sessions)    | session_artifact                | TUI/Web diagnostics          | missing → no trim visible
// request_boundary_hygiene_summary       | (retired; legacy sessions)    | session_artifact, telemetry      | TUI/Web diagnostics, API     | missing → no boundary hygiene visible
// pending_sanitizer_stage               | server (resume/continue)      | scheduler (consume-on-read)      | internal only               | missing → defaults to PreRequest
//
// "Consume-on-read" keys (like pending_sanitizer_stage) are removed from
// metadata after first read — they are transient lifecycle signals, not
// persistent state.
//
pub const PROMPT_SURFACE_STATE_SNAPSHOT_METADATA_KEY: &str = "prompt_surface_state_snapshot";
pub const PROMPT_SURFACE_EVIDENCE_METADATA_KEY: &str = "prompt_surface_evidence";
pub const CONTEXT_COMPACTION_RECORD_METADATA_KEY: &str = "context_compaction_record";
pub const CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY: &str =
    "context_compaction_continuity_packet";
pub const CONTEXT_COMPACTION_LIFECYCLE_SUMMARY_METADATA_KEY: &str =
    "context_compaction_lifecycle_summary";
pub const CONTEXT_PRESSURE_GOVERNANCE_SUMMARY_METADATA_KEY: &str =
    "context_pressure_governance_summary";
pub const CONTEXT_LIGHTWEIGHT_TRIM_SUMMARY_METADATA_KEY: &str = "context_lightweight_trim_summary";
pub const REQUEST_BOUNDARY_HYGIENE_SUMMARY_METADATA_KEY: &str = "request_boundary_hygiene_summary";
pub const PENDING_SANITIZER_STAGE_METADATA_KEY: &str = "pending_sanitizer_stage";

pub fn sanctioned_model_context_summary(message: &SessionMessage) -> Option<&str> {
    surface_contract::sanctioned_model_context_projection_for_message(message)
        .map(|projection| projection.summary)
}

pub fn replay_provider_messages(
    messages: &[SessionMessage],
) -> anyhow::Result<Vec<agendao_provider::Message>> {
    SessionPrompt::build_chat_messages(messages, None, &[])
}

pub fn continuity_packet_allowed_message_ids(value: &serde_json::Value) -> Option<Vec<String>> {
    let packet = SessionContinuityPacket::from_value(value)?;
    let ctx = PromptReflowContext::build("", None, Some(&packet), false, false, None, None);
    Some(ctx.continuity?.hydrate_message_ids)
}

pub fn continuity_packet_inspection(
    value: &serde_json::Value,
) -> Option<SessionCompactionContinuityInspection> {
    let packet = SessionContinuityPacket::from_value(value)?;
    let ctx = PromptReflowContext::build("", None, Some(&packet), false, false, None, None);
    let continuity = ctx.continuity?;
    Some(SessionCompactionContinuityInspection {
        source: agendao_types::SessionCompactionContinuityInspectionSource::ContinuityPacket,
        summary_message_id: packet
            .latest_compaction_summary
            .as_ref()
            .map(|summary| summary.message_id.clone()),
        summary_text: continuity
            .compaction_summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        eligible_message_count: Some(continuity.eligible_message_count),
        exact_recent_tail_count: Some(continuity.exact_recent_tail_count),
        omitted_older_turns: Some(continuity.omitted_older_turns),
        has_working_ledger: !packet.working_ledger.is_empty(),
        has_memory_anchors: !packet.memory_anchors.is_empty(),
        recall_policy: continuity
            .recall_policy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    })
}

pub fn request_boundary_hygiene_summary(
    value: &serde_json::Value,
) -> Option<agendao_types::RequestBoundaryHygieneSummary> {
    serde_json::from_value(value.clone()).ok()
}

pub fn render_session_reflow_diagnostics_summary(session: &Session) -> Option<String> {
    let memory_prefetch = session
        .metadata
        .get("memory_last_prefetch_packet")
        .cloned()
        .and_then(|value| serde_json::from_value::<MemoryRetrievalPacket>(value).ok());
    let continuity_packet = session.messages.iter().rev().find_map(|message| {
        message
            .metadata
            .get(CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY)
            .cloned()
            .and_then(|value| SessionContinuityPacket::from_value(&value))
    });
    let hygiene_summary = session
        .metadata
        .get(REQUEST_BOUNDARY_HYGIENE_SUMMARY_METADATA_KEY)
        .and_then(request_boundary_hygiene_summary);
    let has_frozen_snapshot = session.metadata.contains_key("memory_frozen_snapshot");
    let has_last_prefetch = session.metadata.contains_key("memory_last_prefetch_packet");
    let has_any_reflow = memory_prefetch.is_some()
        || continuity_packet.is_some()
        || hygiene_summary.is_some()
        || has_frozen_snapshot
        || has_last_prefetch;
    if !has_any_reflow {
        return None;
    }
    let ctx = PromptReflowContext::build(
        session.id.clone(),
        memory_prefetch.as_ref(),
        continuity_packet.as_ref(),
        has_frozen_snapshot,
        has_last_prefetch,
        None,
        hygiene_summary,
    );
    Some(ctx.render_summary())
}

pub use ingress::{
    external_adapter_event_to_ingress_turn, normalize_ingress_source, stabilize_ingress_turns,
    ExternalAdapterIngressMappingError, IngressAttachmentRef, IngressSource,
    IngressStabilizationMetadata, IngressTurnEnvelope, INGRESS_POLICY_ENTRY_METADATA_ONLY,
    INGRESS_POLICY_EXTERNAL_ADAPTER_METADATA_ONLY, INGRESS_POLICY_SAME_SESSION_CONTEXT_BATCH,
    INGRESS_POLICY_SCHEDULER_METADATA_ONLY, INGRESS_POLICY_UNSPECIFIED,
};
use reflow_context::PromptReflowContext;
use surface_contract::HiddenRuntimeHint;
pub use tools_and_output::{
    compose_session_title_source, generate_session_title, generate_session_title_for_session,
    generate_session_title_llm, sanitize_session_title_source,
};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use agendao_output_blocks::OutputBlock;
use agendao_provider::cache::CacheEvidenceSummary;
use agendao_skill::SkillGovernanceAuthority;
use agendao_types::{
    message_latest_compaction_summary, tool_call_replay_text_len,
    ContextCompactionInstalledDiagnostics, ContextCompactionLifecycleStatus,
    ContextCompactionLifecycleSummary, ContextCompactionSummary, ContextPressureGovernanceSummary,
    MemoryRetrievalPacket, PromptSurfaceEvidenceSummary, SessionCacheBoundaryKind,
    SessionCacheBoundarySummary, SessionCacheEvidenceExplain, SessionCacheSemanticsBasis,
    SessionCacheSemanticsSummary, SessionCacheSeverity, SessionCompactionContinuityInspection,
    SessionContextExplain, SessionContinuityPacket,
};

use crate::{MessageRole, PartType, Session, SessionMessage, SessionStateManager};

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

pub type SessionUpdateHook = Arc<dyn Fn(&Session) + Send + Sync + 'static>;
pub type EventBroadcastHook = Arc<dyn Fn(serde_json::Value) + Send + Sync + 'static>;
pub type CompactionLifecycleHook =
    Arc<dyn Fn(ContextCompactionLifecycleSummary) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextUsageSnapshot {
    pub live_context_tokens: Option<u64>,
    pub request_context_tokens: Option<u64>,
    pub request_body_chars: Option<usize>,
}

impl ContextUsageSnapshot {
    pub const fn new(
        live_context_tokens: Option<u64>,
        request_context_tokens: Option<u64>,
        request_body_chars: Option<usize>,
    ) -> Self {
        Self {
            live_context_tokens,
            request_context_tokens,
            request_body_chars,
        }
    }
}

pub struct AutoCompactionOptions<'a> {
    pub focus: Option<&'a str>,
    pub trigger: &'a str,
    pub phase: Option<&'a str>,
    pub usage: ContextUsageSnapshot,
}

pub struct ContextGovernanceAssessment<'a> {
    pub trigger: &'a str,
    pub phase: &'a str,
    pub compaction_attempted: bool,
    pub compaction_succeeded: bool,
    pub usage: ContextUsageSnapshot,
}

pub struct PreDispatchContextGovernance<'a> {
    pub focus: Option<&'a str>,
    pub trigger: &'a str,
    pub phase: &'a str,
    pub usage: ContextUsageSnapshot,
    pub update_hook: Option<&'a SessionUpdateHook>,
    pub compaction_lifecycle_hook: Option<&'a CompactionLifecycleHook>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBlockEvent {
    pub session_id: String,
    pub block: OutputBlock,
    /// Optional transcript block ID used when no live identity is available.
    pub id: Option<String>,
    /// Canonical live-stream identity. When populated, consumers route stream
    /// fragments by identity instead of heuristic guessing.
    /// Non-streaming synthetic blocks may omit it.
    pub live_identity: Option<agendao_types::LiveMessagePartIdentity>,
}

pub fn assistant_text_live_identity(
    message_id: &str,
    phase: agendao_types::LivePartPhase,
) -> agendao_types::LiveMessagePartIdentity {
    agendao_types::LiveMessagePartIdentity {
        message_id: message_id.to_string(),
        part_key: agendao_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
        part_kind: agendao_types::LiveMessagePartKind::AssistantText,
        phase,
    }
}

pub fn assistant_reasoning_live_identity(
    message_id: &str,
    phase: agendao_types::LivePartPhase,
) -> agendao_types::LiveMessagePartIdentity {
    agendao_types::LiveMessagePartIdentity {
        message_id: message_id.to_string(),
        part_key: agendao_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
        part_kind: agendao_types::LiveMessagePartKind::AssistantReasoning,
        phase,
    }
}

pub fn tool_call_live_identity(
    message_id: &str,
    tool_call_id: &str,
    phase: agendao_types::LivePartPhase,
) -> agendao_types::LiveMessagePartIdentity {
    agendao_types::LiveMessagePartIdentity {
        message_id: message_id.to_string(),
        part_key: agendao_types::tool_call_part_key(tool_call_id),
        part_kind: agendao_types::LiveMessagePartKind::ToolCall,
        phase,
    }
}

pub fn tool_result_live_identity(
    message_id: &str,
    tool_call_id: &str,
    phase: agendao_types::LivePartPhase,
) -> agendao_types::LiveMessagePartIdentity {
    agendao_types::LiveMessagePartIdentity {
        message_id: message_id.to_string(),
        part_key: agendao_types::tool_result_part_key(tool_call_id),
        part_kind: agendao_types::LiveMessagePartKind::ToolResult,
        phase,
    }
}
pub type OutputBlockHook = Arc<
    dyn Fn(OutputBlockEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
>;
pub type PublishBusHook = Arc<
    dyn Fn(String, serde_json::Value) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync
        + 'static,
>;
pub type AskQuestionHook = Arc<
    dyn Fn(
            String,
            Vec<agendao_tool::QuestionDef>,
        ) -> Pin<
            Box<dyn Future<Output = Result<Vec<Vec<String>>, agendao_tool::ToolError>> + Send>,
        > + Send
        + Sync
        + 'static,
>;
pub type AskPermissionHook = Arc<
    dyn Fn(
            String,
            agendao_tool::PermissionRequest,
        ) -> Pin<Box<dyn Future<Output = Result<(), agendao_tool::ToolError>> + Send>>
        + Send
        + Sync
        + 'static,
>;

#[derive(Clone, Default)]
pub struct PromptHooks {
    pub update_hook: Option<SessionUpdateHook>,
    pub event_broadcast: Option<EventBroadcastHook>,
    pub compaction_lifecycle_hook: Option<CompactionLifecycleHook>,
    pub output_block_hook: Option<OutputBlockHook>,
    pub ask_question_hook: Option<AskQuestionHook>,
    pub ask_permission_hook: Option<AskPermissionHook>,
    pub publish_bus_hook: Option<PublishBusHook>,
    /// P0 steering: called after tool execution to drain pending steering messages.
    /// Returns texts to inject as user messages before the next model request.
    pub steering_boundary_hook: Option<SteeringBoundaryHook>,
}

/// A steering message drained from the server-owned queue, ready for injection.
#[derive(Debug, Clone)]
pub struct SteeringMessage {
    pub text: String,
    pub created_at: i64,
    pub source_session_id: Option<String>,
}

/// Hook called at the tool boundary to drain pending steering messages.
/// Constitution §9: session calls the hook; server owns the queue.
pub type SteeringBoundaryHook = Arc<
    dyn Fn(
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<SteeringMessage>> + Send>>
        + Send
        + Sync,
>;

/// Session prompt surface authority.
///
/// # Prompt surface construction pipeline (AgenDao 土律)
///
/// ```text
/// SystemPrompt                  ← product header + env (static layer)
///   → SessionPrompt             ← surface assembly authority
///     ← PromptSurfaceInputs     ← aggregated inputs
///     → PromptSurfaceSections   ← canonical output sections
///       → ProviderOptions       ← cache hints, reasoning policy
///       → API request           ← final model call
/// ```
///
/// Providers declare capabilities per profile.
/// `SessionPrompt` is the single assembler.
pub struct SessionPrompt {
    state: Arc<Mutex<HashMap<String, PromptState>>>,
    session_state: Arc<RwLock<SessionStateManager>>,
    mcp_clients: Option<Arc<agendao_mcp::McpClientRegistry>>,
    lsp_registry: Option<Arc<agendao_lsp::LspClientRegistry>>,
    tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    config_store: Option<Arc<agendao_config::ConfigStore>>,
    memory_authority: Option<Arc<agendao_memory::MemoryAuthority>>,
    proposal_repo: Option<Arc<agendao_storage::SkillEvolutionProposalRepository>>,
    /// Todo state shared with the server's read path (GET /session todos).
    /// Standalone use falls back to a private in-memory manager.
    todo_manager: Arc<crate::TodoManager>,
    /// Stale-file guard shared across read→write tool sequences.
    file_time_tracker: Arc<agendao_tool::FileTimeTracker>,
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
    candidates: &[agendao_types::MemoryRecord],
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
    candidates: &[agendao_types::MemoryRecord],
) -> BTreeMap<String, String> {
    let mut skill_names = BTreeMap::new();
    for record in candidates {
        if record.kind != agendao_types::MemoryKind::MethodologyCandidate {
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
         Review: type /proposals or run `agendao skill proposal list`.",
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
    compact_session_now_with_focus_result(session, focus).summary
}

#[derive(Debug, Clone)]
pub struct ManualCompactionResult {
    pub summary: Option<String>,
    pub lifecycle: ContextCompactionLifecycleSummary,
    pub compaction: Option<ContextCompactionSummary>,
}

impl ManualCompactionResult {
    pub fn success(&self) -> bool {
        self.lifecycle.status == ContextCompactionLifecycleStatus::Installed
    }

    pub fn message(&self, focus: Option<&str>) -> String {
        match self.lifecycle.status {
            ContextCompactionLifecycleStatus::Installed => {
                let compacted = self
                    .compaction
                    .as_ref()
                    .and_then(|record| record.compacted_message_count)
                    .unwrap_or_default();
                let kept = self
                    .compaction
                    .as_ref()
                    .and_then(|record| record.kept_message_count)
                    .unwrap_or_default();
                if let Some(focus) = focus.map(str::trim).filter(|value| !value.is_empty()) {
                    if compacted > 0 || kept > 0 {
                        format!(
                            "Session compacted around focus: {focus} ({compacted} summarized, {kept} kept)."
                        )
                    } else {
                        format!("Session compacted around focus: {focus}")
                    }
                } else if compacted > 0 || kept > 0 {
                    format!("Session compacted ({compacted} summarized, {kept} kept).")
                } else {
                    "Session compacted successfully.".to_string()
                }
            }
            ContextCompactionLifecycleStatus::Skipped => match self.lifecycle.reason.as_deref() {
                Some("session.manual_compact.no_prompt_continuity_owner") => {
                    "This session does not own prompt continuity.".to_string()
                }
                Some("session.manual_compact.insufficient_messages") => {
                    "Nothing to compact yet.".to_string()
                }
                _ => "Manual compaction skipped.".to_string(),
            },
            ContextCompactionLifecycleStatus::Failed => "Manual compaction failed.".to_string(),
            ContextCompactionLifecycleStatus::Started => "Manual compaction started.".to_string(),
        }
    }
}

pub fn compact_session_now_with_focus_result(
    session: &mut Session,
    focus: Option<&str>,
) -> ManualCompactionResult {
    if !session.context_kind().owns_prompt_continuity() {
        let lifecycle = context_compaction_lifecycle_summary(
            "manual",
            Some("session.manual_compact"),
            Some("session.manual_compact.no_prompt_continuity_owner"),
            ContextCompactionLifecycleStatus::Skipped,
            true,
            ContextUsageSnapshot::default(),
            None,
        );
        persist_context_compaction_lifecycle_summary(session, &lifecycle);
        return ManualCompactionResult {
            summary: None,
            lifecycle,
            compaction: None,
        };
    }
    let filtered = SessionPrompt::filter_compacted_messages(&session.messages);
    if filtered.len() < message_building::FORCE_COMPACTION_MIN_MESSAGES {
        let lifecycle = context_compaction_lifecycle_summary(
            "manual",
            Some("session.manual_compact"),
            Some("session.manual_compact.insufficient_messages"),
            ContextCompactionLifecycleStatus::Skipped,
            true,
            ContextUsageSnapshot::default(),
            None,
        );
        persist_context_compaction_lifecycle_summary(session, &lifecycle);
        return ManualCompactionResult {
            summary: None,
            lifecycle,
            compaction: None,
        };
    }
    let lifecycle = context_compaction_lifecycle_summary(
        "manual",
        Some("session.manual_compact"),
        None,
        ContextCompactionLifecycleStatus::Started,
        true,
        ContextUsageSnapshot::default(),
        None,
    );
    persist_context_compaction_lifecycle_summary(session, &lifecycle);
    session.start_compacting();
    let record = SessionPrompt::build_compaction_record(
        "manual",
        Some("session.manual_compact"),
        None,
        true,
        ContextUsageSnapshot::default(),
        None,
    );
    let summary = SessionPrompt::trigger_compaction_with_record(
        session,
        &filtered,
        focus,
        Some(record),
        true,
    );
    let mut lifecycle = lifecycle;
    lifecycle.status = if summary.is_some() {
        ContextCompactionLifecycleStatus::Installed
    } else {
        ContextCompactionLifecycleStatus::Failed
    };
    if summary.is_some() {
        install_compaction_lifecycle_summary(session, &mut lifecycle);
    }
    persist_context_compaction_lifecycle_summary(session, &lifecycle);
    session.finish_compacting();
    let compaction = if summary.is_some() {
        latest_context_compaction_summary_from_session(session)
    } else {
        None
    };
    ManualCompactionResult {
        summary,
        lifecycle,
        compaction,
    }
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

fn persist_context_compaction_lifecycle_summary(
    session: &mut Session,
    summary: &ContextCompactionLifecycleSummary,
) {
    if let Ok(value) = serde_json::to_value(summary) {
        session.insert_metadata(
            CONTEXT_COMPACTION_LIFECYCLE_SUMMARY_METADATA_KEY.to_string(),
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

fn context_compaction_lifecycle_summary(
    trigger: &str,
    phase: Option<&str>,
    reason: Option<&str>,
    status: ContextCompactionLifecycleStatus,
    forced: bool,
    usage: ContextUsageSnapshot,
    limit_tokens: Option<u64>,
) -> ContextCompactionLifecycleSummary {
    ContextCompactionLifecycleSummary {
        trigger: trigger.to_string(),
        phase: phase.map(str::to_string),
        reason: reason.map(str::to_string),
        status,
        forced,
        request_context_tokens: usage.request_context_tokens,
        live_context_tokens: usage.live_context_tokens,
        limit_tokens,
        body_chars: usage.request_body_chars,
        installed: None,
    }
}

fn latest_context_compaction_summary_from_session(
    session: &Session,
) -> Option<ContextCompactionSummary> {
    session
        .record()
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            message
                .metadata
                .get(CONTEXT_COMPACTION_RECORD_METADATA_KEY)
                .cloned()
        })
        .and_then(|value| serde_json::from_value::<ContextCompactionSummary>(value).ok())
        .map(|mut summary| {
            if let Some(packet_summary) =
                session.record().messages.iter().rev().find_map(|message| {
                    if !matches!(message.role, MessageRole::Assistant) {
                        return None;
                    }
                    message_latest_compaction_summary(
                        &message.metadata,
                        &message.id,
                        summary.summary.as_deref(),
                    )
                })
            {
                summary.summary = Some(packet_summary.summary);
            }
            summary
        })
}

fn installed_compaction_diagnostics(session: &Session) -> ContextCompactionInstalledDiagnostics {
    let context_explain = explain_session_context(session, None);
    let cache_explanation =
        latest_context_compaction_summary_from_session(session).and_then(|summary| {
            explain_session_cache_semantics(&context_explain, Some(&summary), None, None).label
        });

    ContextCompactionInstalledDiagnostics {
        request_context_tokens: context_explain.api_view_estimated_input_tokens,
        live_context_tokens: context_explain.live_context_tokens,
        body_chars: context_explain.api_view_body_chars,
        cache_explanation,
    }
}

pub(super) fn install_compaction_lifecycle_summary(
    session: &Session,
    lifecycle: &mut ContextCompactionLifecycleSummary,
) {
    lifecycle.installed = Some(installed_compaction_diagnostics(session));
}

pub fn estimate_current_context_tokens(messages: &[SessionMessage]) -> Option<u64> {
    let filtered = SessionPrompt::filter_compacted_messages_cow(messages);
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
    let filtered = SessionPrompt::filter_compacted_messages_cow(&record.messages);
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
    let live_context_tokens = estimate_current_context_tokens(&record.messages);
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
        live_context_tokens,
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
    value: agendao_provider::cache::CacheEvidenceSeverity,
) -> SessionCacheSeverity {
    match value {
        agendao_provider::cache::CacheEvidenceSeverity::Stable => SessionCacheSeverity::Stable,
        agendao_provider::cache::CacheEvidenceSeverity::LowChange => {
            SessionCacheSeverity::LowChange
        }
        agendao_provider::cache::CacheEvidenceSeverity::MediumChange => {
            SessionCacheSeverity::MediumChange
        }
        agendao_provider::cache::CacheEvidenceSeverity::HighChange => {
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
    use super::{
        compact_session_now_with_focus_result, explain_session_cache_semantics,
        ContextCompactionLifecycleStatus,
    };
    use crate::Session;
    use agendao_provider::cache::{CacheEvidenceSeverity, CacheEvidenceSummary};
    use agendao_types::{
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
            agendao_types::SessionCacheSemanticsBasis::ApiView
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
            stable_prefix_change: None,
            dynamic_overlay_reasons: Vec::new(),
            drift_details: Vec::new(),
            volatility_findings: Vec::new(),
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
    fn cache_semantics_preserves_surface_label_with_structured_details() {
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
            stable_prefix_change: None,
            dynamic_overlay_reasons: Vec::new(),
            drift_details: vec![agendao_types::PromptSurfaceDriftDetail {
                category: agendao_types::PromptSurfaceDriftCategory::IngressPolicy,
                field: "ingressPolicyHash".to_string(),
                detail: "ingress policy changed".to_string(),
                severity: SessionCacheSeverity::LowChange,
            }],
            volatility_findings: vec![agendao_types::PromptSurfaceVolatilityFinding {
                kind: agendao_types::PromptSurfaceVolatilityKind::ProviderOptionsAffectSurface,
                field: "provider_options".to_string(),
                detail: "reasoning keys: 1 · tool policy keys: 0".to_string(),
            }],
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
                .map(|value| value.drift_details.len()),
            Some(1)
        );
        assert_eq!(
            summary
                .prompt_surface_evidence
                .as_ref()
                .map(|value| value.volatility_findings.len()),
            Some(1)
        );
    }

    #[test]
    fn compact_session_now_reports_skipped_when_history_is_too_small() {
        let mut session = Session::new("proj", ".");
        session.add_user_message("hello");

        let result = compact_session_now_with_focus_result(&mut session, None);

        assert!(result.summary.is_none());
        assert_eq!(
            result.lifecycle.status,
            ContextCompactionLifecycleStatus::Skipped
        );
        assert_eq!(
            result.lifecycle.reason.as_deref(),
            Some("session.manual_compact.insufficient_messages")
        );
        assert_eq!(result.message(None), "Nothing to compact yet.");
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
                tool_call_replay_text_len(input, raw.as_deref())
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

impl SessionPrompt {
    pub fn new(session_state: Arc<RwLock<SessionStateManager>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            session_state,
            mcp_clients: None,
            lsp_registry: None,
            tool_runtime_config: agendao_tool::ToolRuntimeConfig::default(),
            config_store: None,
            memory_authority: None,
            proposal_repo: None,
            todo_manager: Arc::new(crate::TodoManager::new()),
            file_time_tracker: Arc::new(agendao_tool::FileTimeTracker::default()),
            review_nudge_state: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn with_tool_runtime_config(
        mut self,
        tool_runtime_config: agendao_tool::ToolRuntimeConfig,
    ) -> Self {
        self.tool_runtime_config = tool_runtime_config;
        self
    }

    pub fn with_todo_manager(mut self, todo_manager: Arc<crate::TodoManager>) -> Self {
        self.todo_manager = todo_manager;
        self
    }

    pub fn with_file_time_tracker(
        mut self,
        file_time_tracker: Arc<agendao_tool::FileTimeTracker>,
    ) -> Self {
        self.file_time_tracker = file_time_tracker;
        self
    }

    pub fn with_config_store(mut self, config_store: Arc<agendao_config::ConfigStore>) -> Self {
        self.config_store = Some(config_store);
        self
    }

    pub fn with_memory_authority(
        mut self,
        memory_authority: Arc<agendao_memory::MemoryAuthority>,
    ) -> Self {
        self.memory_authority = Some(memory_authority);
        self
    }

    pub fn with_proposal_repo(
        mut self,
        proposal_repo: Arc<agendao_storage::SkillEvolutionProposalRepository>,
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
            .run_consolidation(&agendao_types::MemoryConsolidationRequest::default())
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
        memory: &agendao_memory::MemoryAuthority,
        session_id: &str,
        workspace_directory: Option<&str>,
        promoted_record_ids: &[agendao_types::MemoryRecordId],
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

        match agendao_storage::generate_skill_evolution_proposals(
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
        candidates: &[agendao_types::MemoryRecord],
    ) -> Vec<agendao_types::MemoryRecord> {
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return candidates.to_vec();
        };

        let mut rewritten = Vec::with_capacity(candidates.len());
        for record in candidates {
            if record.kind != agendao_types::MemoryKind::MethodologyCandidate {
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
        candidates: &[agendao_types::MemoryRecord],
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
        repo: &agendao_storage::SkillEvolutionProposalRepository,
        linked_methodology_skills: &BTreeMap<String, String>,
    ) {
        if linked_methodology_skills.is_empty() {
            return;
        }
        let Some(governance) = self.skill_governance_for_workspace(workspace_directory) else {
            return;
        };

        let draft_proposals = match repo
            .list_by_status(&agendao_types::ProposalStatus::Draft)
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

    pub fn with_mcp_clients(mut self, clients: Arc<agendao_mcp::McpClientRegistry>) -> Self {
        self.mcp_clients = Some(clients);
        self
    }

    pub fn with_lsp_registry(mut self, registry: Arc<agendao_lsp::LspClientRegistry>) -> Self {
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
            }
        }

        ingress_metadata::annotate_message_ingress_metadata(msg, input.ingress.as_ref());

        Ok(())
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
    #[error("{message}", message = .0.message)]
    ProviderFailure(agendao_provider::ProviderErrorSummary),
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptProviderFailure {
    TypedSummary(agendao_provider::ProviderErrorSummary),
    UntypedMessage(String),
}

impl PromptError {
    pub fn provider_failure(&self) -> Option<PromptProviderFailure> {
        match self {
            Self::ProviderFailure(summary) => {
                Some(PromptProviderFailure::TypedSummary(summary.clone()))
            }
            Self::Provider(message) => Some(PromptProviderFailure::UntypedMessage(message.clone())),
            Self::Busy(_) | Self::NoUserMessage | Self::Cancelled => None,
        }
    }

    pub fn provider_error_summary(&self) -> Option<agendao_provider::ProviderErrorSummary> {
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
) -> Option<agendao_provider::ProviderErrorSummary> {
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
static FILE_REFERENCE_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?:^|([^\w`]))@(\.?[^\s`,.]*(?:\.[^\s`,.]+)*)").unwrap()
});

pub async fn resolve_prompt_parts(template: &str, worktree: &std::path::Path) -> Vec<PartInput> {
    let mut parts = vec![PartInput::Text {
        text: template.to_string(),
    }];

    let mut seen = std::collections::HashSet::new();

    for cap in FILE_REFERENCE_REGEX.captures_iter(template) {
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
                // 展开用户输入的 `@~/...` 文件引用，要的是真实用户主目录，不经 agendao_home。
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
            }
        }
    }

    parts
}

pub fn extract_file_references(template: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for cap in FILE_REFERENCE_REGEX.captures_iter(template) {
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
