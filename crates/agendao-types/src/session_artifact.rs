use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    MessagePart, MessageRole, PartType, Session, SessionMessage, SessionTelemetrySnapshot,
    ToolState,
};

const SESSION_SANCTIONED_METADATA_KEYS: &[&str] = &[
    "agent",
    "auto_title_pending_refine",
    "context_compaction_lifecycle_summary",
    "context_lightweight_trim_summary",
    "context_pressure_governance_summary",
    "fork_origin_message_id",
    "fork_origin_session_id",
    "fork_history_mode",
    "fork_history_message_limit",
    "fork_source_history_message_count",
    "fork_policy_frozen",
    "memory_frozen_snapshot",
    "memory_last_prefetch_packet",
    "model_id",
    "model_provider",
    "model_variant",
    "pending_command_invocation",
    "prompt_surface_state_snapshot",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_root_agent",
    "scheduler_session_context_packet",
    "scheduler_skill_tree_applied",
    "session_context_kind",
    "skill_reflection",
    "subsessions",
    "telemetry",
    "runtime_skill_instructions",
];

const SESSION_SANCTIONED_METADATA_PREFIXES: &[&str] = &[
    "last_ingress_",
    "last_recovery_",
    "scheduler_handoff_",
    "scheduler_selection_",
];

const MESSAGE_SANCTIONED_METADATA_KEYS: &[&str] = &[
    "agent",
    "cache_evidence_inspection",
    "cache_evidence",
    "cache_request_fingerprint",
    "context_compaction_continuity_packet",
    "context_compaction_record",
    "continuationTargets",
    "cost",
    "finish_reason",
    "fork_imported_history",
    "fork_history_mode",
    "fork_history_message_limit",
    "fork_origin_message_id",
    "fork_origin_session_id",
    "memory_prefetch_packet",
    "mode",
    "model_id",
    "model_provider",
    "model_variant",
    "multimodal_preflight",
    "preflight",
    "prompt_surface_state_snapshot",
    "prompt_surface_evidence",
    "provider_diagnostic",
    "provider_error_summary",
    "resolved_agent",
    "resolved_execution_mode_kind",
    "resolved_system_prompt",
    "resolved_system_prompt_applied",
    "resolved_system_prompt_preview",
    "resolved_user_prompt",
    "runtime_hint",
    "scheduler_applied",
    "scheduler_profile",
    "scheduler_stage",
    "scheduler_steps",
    "scheduler_tool_calls",
    "snapshot",
    "step_finish_snapshot",
    "step_start_snapshot",
    "summary",
    "tokens_input",
    "tokens_output",
    "usage",
    "workflowModeArtifacts",
];

const MESSAGE_SANCTIONED_METADATA_PREFIXES: &[&str] = &[
    "ingress_",
    "scheduler_decision_",
    "scheduler_output_",
    "scheduler_stage_",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionArtifactVersion {
    #[serde(rename = "agendao-rust/v1")]
    AgendaoRustV1,
}

impl SessionArtifactVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgendaoRustV1 => "agendao-rust/v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionArtifactMetadataKeyClassification {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sanctioned_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passthrough_keys: Vec<String>,
}

impl SessionArtifactMetadataKeyClassification {
    pub fn is_empty(&self) -> bool {
        self.sanctioned_keys.is_empty() && self.passthrough_keys.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionArtifactMessageMetadataAuthority {
    pub message_id: String,
    #[serde(
        default,
        skip_serializing_if = "SessionArtifactMetadataKeyClassification::is_empty"
    )]
    pub keys: SessionArtifactMetadataKeyClassification,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SessionArtifactMetadataAuthority {
    #[serde(
        default,
        skip_serializing_if = "SessionArtifactMetadataKeyClassification::is_empty"
    )]
    pub session: SessionArtifactMetadataKeyClassification,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<SessionArtifactMessageMetadataAuthority>,
}

impl SessionArtifactMetadataAuthority {
    fn classify(session: &Session, messages: &[SessionMessage]) -> Option<Self> {
        let session_keys = classify_metadata_keys(
            &session.metadata,
            SESSION_SANCTIONED_METADATA_KEYS,
            SESSION_SANCTIONED_METADATA_PREFIXES,
        );
        let messages = messages
            .iter()
            .filter_map(|message| {
                let keys = classify_metadata_keys(
                    &message.metadata,
                    MESSAGE_SANCTIONED_METADATA_KEYS,
                    MESSAGE_SANCTIONED_METADATA_PREFIXES,
                );
                (!keys.is_empty()).then(|| SessionArtifactMessageMetadataAuthority {
                    message_id: message.id.clone(),
                    keys,
                })
            })
            .collect::<Vec<_>>();

        if session_keys.is_empty() && messages.is_empty() {
            None
        } else {
            Some(Self {
                session: session_keys,
                messages,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionDiagnosticsSidecarVersion {
    #[serde(rename = "agendao-rust/diagnostics/v1")]
    AgendaoRustDiagnosticsV1,
}

impl SessionDiagnosticsSidecarVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgendaoRustDiagnosticsV1 => "agendao-rust/diagnostics/v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionIngressStabilizationSummary {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionExecutionPreflightMetadataSource {
    ToolCallState,
    ToolResultPart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionExecutionPreflightMetadataEntry {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub source: SessionExecutionPreflightMetadataSource,
    pub metadata: serde_json::Value,
}

impl SessionExecutionPreflightMetadataEntry {
    pub fn decode_metadata<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.metadata.clone()).ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionArtifactMessageDiagnosticsSidecar {
    pub message_id: String,
    pub role: MessageRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_evidence_inspection: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_evidence: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_evidence: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_compaction_record: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_compaction_continuity_packet: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error_summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub execution_preflights: Vec<SessionExecutionPreflightMetadataEntry>,
}

impl SessionArtifactMessageDiagnosticsSidecar {
    fn is_empty(&self) -> bool {
        self.cache_evidence_inspection.is_none()
            && self.cache_evidence.is_none()
            && self.prompt_surface_evidence.is_none()
            && self.context_compaction_record.is_none()
            && self.context_compaction_continuity_packet.is_none()
            && self.provider_diagnostic.is_none()
            && self.provider_error_summary.is_none()
            && self.execution_preflights.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDiagnosticsSidecar {
    pub version: SessionDiagnosticsSidecarVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<SessionTelemetrySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_state_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_compaction_lifecycle_summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pressure_governance_summary: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_compaction_decision_trace: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_stabilization: Option<SessionIngressStabilizationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_frozen_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_last_prefetch_packet: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<SessionArtifactMessageDiagnosticsSidecar>,
}

impl SessionDiagnosticsSidecar {
    pub fn derive_from_session(session: &Session) -> Option<Self> {
        Self::derive(session, &session.messages)
    }

    pub fn derive_from_parts(session: &Session, messages: &[SessionMessage]) -> Option<Self> {
        Self::derive(session, messages)
    }

    pub fn prompt_surface_state_snapshot_value(&self) -> Option<serde_json::Value> {
        self.prompt_surface_state_snapshot.clone()
    }

    pub fn context_compaction_lifecycle_summary_value(&self) -> Option<serde_json::Value> {
        self.context_compaction_lifecycle_summary.clone()
    }

    pub fn context_pressure_governance_summary_value(&self) -> Option<serde_json::Value> {
        self.context_pressure_governance_summary.clone()
    }

    pub fn latest_context_compaction_decision_trace_value(&self) -> Option<serde_json::Value> {
        self.context_pressure_governance_summary
            .as_ref()
            .and_then(|summary| summary.get("decision_trace").cloned())
    }

    pub fn ingress_stabilization_value(&self) -> Option<serde_json::Value> {
        self.ingress_stabilization.as_ref().map(|summary| {
            serde_json::json!({
                "source": summary.source,
                "policy": summary.policy,
                "batch_count": summary.batch_count,
            })
        })
    }

    pub fn latest_cache_evidence_inspection_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.cache_evidence_inspection.clone())
    }

    pub fn latest_cache_evidence_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.cache_evidence.clone())
    }

    pub fn latest_prompt_surface_evidence_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.prompt_surface_evidence.clone())
            .or_else(|| {
                self.prompt_surface_state_snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.get("evidence").cloned())
            })
    }

    pub fn latest_context_compaction_record_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.context_compaction_record.clone())
    }

    pub fn latest_context_compaction_continuity_packet_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.context_compaction_continuity_packet.clone())
    }

    pub fn latest_provider_error_summary_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| message.provider_error_summary.clone())
    }

    pub fn latest_provider_diagnostic_value(&self) -> Option<serde_json::Value> {
        self.messages
            .iter()
            .rev()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .find_map(|message| {
                message
                    .provider_error_summary
                    .as_ref()
                    .and_then(|summary| summary.get("provider_diagnostic").cloned())
                    .or_else(|| message.provider_diagnostic.clone())
            })
    }

    pub fn latest_execution_preflight_entry(
        &self,
    ) -> Option<SessionExecutionPreflightMetadataEntry> {
        #[derive(Clone)]
        struct Candidate {
            entry: SessionExecutionPreflightMetadataEntry,
            order: (usize, usize),
        }

        fn merge_execution_preflight_candidate(existing: &mut Candidate, candidate: Candidate) {
            let next_order = existing.order.max(candidate.order);
            let should_replace = match (existing.entry.source, candidate.entry.source) {
                (
                    SessionExecutionPreflightMetadataSource::ToolResultPart,
                    SessionExecutionPreflightMetadataSource::ToolCallState,
                ) => true,
                (
                    SessionExecutionPreflightMetadataSource::ToolCallState,
                    SessionExecutionPreflightMetadataSource::ToolResultPart,
                ) => false,
                _ => candidate.order >= existing.order,
            };

            if should_replace {
                existing.entry = candidate.entry;
            } else if existing.entry.tool_name.is_none() {
                existing.entry.tool_name = candidate.entry.tool_name;
            }

            existing.order = next_order;
        }

        let mut candidates = HashMap::<String, Candidate>::new();
        for (message_index, message) in self.messages.iter().enumerate() {
            for (entry_index, entry) in message.execution_preflights.iter().enumerate() {
                let candidate = Candidate {
                    entry: entry.clone(),
                    order: (message_index, entry_index),
                };
                match candidates.get_mut(&candidate.entry.tool_call_id) {
                    Some(existing) => merge_execution_preflight_candidate(existing, candidate),
                    None => {
                        candidates.insert(candidate.entry.tool_call_id.clone(), candidate);
                    }
                }
            }
        }

        candidates
            .into_values()
            .max_by_key(|candidate| candidate.order)
            .map(|candidate| candidate.entry)
    }

    fn derive(session: &Session, messages: &[SessionMessage]) -> Option<Self> {
        let telemetry = session
            .metadata
            .get("telemetry")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let prompt_surface_state_snapshot = session
            .metadata
            .get("prompt_surface_state_snapshot")
            .cloned()
            .or_else(|| latest_message_metadata_value(messages, "prompt_surface_state_snapshot"));
        let context_compaction_lifecycle_summary = session
            .metadata
            .get("context_compaction_lifecycle_summary")
            .cloned();
        let context_pressure_governance_summary = session
            .metadata
            .get("context_pressure_governance_summary")
            .cloned();
        let context_compaction_decision_trace = context_pressure_governance_summary
            .as_ref()
            .and_then(|summary| summary.get("decision_trace").cloned());
        let ingress_stabilization = ingress_stabilization_from_session(session);
        let memory_frozen_snapshot = session.metadata.get("memory_frozen_snapshot").cloned();
        let memory_last_prefetch_packet =
            session.metadata.get("memory_last_prefetch_packet").cloned();
        let tool_names = tool_call_name_index(messages);
        let messages = messages
            .iter()
            .filter_map(|message| message_diagnostics_sidecar(message, &tool_names))
            .collect::<Vec<_>>();

        let sidecar = Self {
            version: SessionDiagnosticsSidecarVersion::AgendaoRustDiagnosticsV1,
            telemetry,
            prompt_surface_state_snapshot,
            context_compaction_lifecycle_summary,
            context_pressure_governance_summary,
            context_compaction_decision_trace,
            ingress_stabilization,
            memory_frozen_snapshot,
            memory_last_prefetch_packet,
            messages,
        };

        (!sidecar.is_empty()).then_some(sidecar)
    }

    fn is_empty(&self) -> bool {
        self.telemetry.is_none()
            && self.prompt_surface_state_snapshot.is_none()
            && self.context_compaction_lifecycle_summary.is_none()
            && self.context_pressure_governance_summary.is_none()
            && self.context_compaction_decision_trace.is_none()
            && self.ingress_stabilization.is_none()
            && self.memory_frozen_snapshot.is_none()
            && self.memory_last_prefetch_packet.is_none()
            && self.messages.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifactEntry {
    #[serde(rename = "info")]
    pub session: Session,
    pub messages: Vec<SessionMessage>,
    /// Describes which exported metadata keys are part of the current
    /// sanctioned session/message contract versus best-effort passthrough.
    /// Raw metadata remains present for compatibility; this is the Phase 4B
    /// classification start point, not a stripping pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_authority: Option<SessionArtifactMetadataAuthority>,
    /// Phase 4C diagnostics sidecar. This is a derived export artifact built
    /// from persisted session/message owners; it is not core restore
    /// authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics_sidecar: Option<SessionDiagnosticsSidecar>,
}

impl SessionArtifactEntry {
    pub fn new(session: Session, messages: Vec<SessionMessage>) -> Self {
        let metadata_authority = SessionArtifactMetadataAuthority::classify(&session, &messages);
        let diagnostics_sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &messages);
        Self {
            session,
            messages,
            metadata_authority,
            diagnostics_sidecar,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifactBundle {
    pub version: SessionArtifactVersion,
    pub exported_at: i64,
    pub sessions: Vec<SessionArtifactEntry>,
}

impl SessionArtifactBundle {
    pub fn new(exported_at: i64, sessions: Vec<SessionArtifactEntry>) -> Self {
        Self {
            version: SessionArtifactVersion::AgendaoRustV1,
            exported_at,
            sessions,
        }
    }

    pub fn new_now(sessions: Vec<SessionArtifactEntry>) -> Self {
        Self::new(Utc::now().timestamp_millis(), sessions)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionArtifactImportEnvelope {
    Bundle(SessionArtifactImportBundle),
    Single(SessionArtifactImportEntry),
    Legacy(LegacySessionArtifactPayload),
}

impl SessionArtifactImportEnvelope {
    pub fn into_entries(self) -> Vec<SessionArtifactEntry> {
        match self {
            Self::Bundle(bundle) => bundle.into_entries(),
            Self::Single(entry) => vec![entry.into_entry()],
            Self::Legacy(legacy) => vec![legacy.into_entry()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifactImportBundle {
    pub version: SessionArtifactVersion,
    pub exported_at: i64,
    pub sessions: Vec<SessionArtifactImportEntry>,
}

impl SessionArtifactImportBundle {
    fn into_entries(self) -> Vec<SessionArtifactEntry> {
        let _ = self.version;
        let _ = self.exported_at;
        self.sessions
            .into_iter()
            .map(SessionArtifactImportEntry::into_entry)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifactImportEntry {
    #[serde(rename = "info")]
    pub session: Session,
    pub messages: Vec<SessionMessage>,
    #[serde(default, rename = "metadata_authority")]
    pub _metadata_authority: Option<serde_json::Value>,
    #[serde(default, rename = "diagnostics_sidecar")]
    pub _diagnostics_sidecar: Option<serde_json::Value>,
}

impl SessionArtifactImportEntry {
    fn into_entry(self) -> SessionArtifactEntry {
        let _ = self._metadata_authority;
        let _ = self._diagnostics_sidecar;
        SessionArtifactEntry::new(self.session, self.messages)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySessionArtifactPayload {
    #[serde(rename = "info")]
    pub session: Session,
    pub messages: Vec<LegacySessionArtifactMessage>,
}

impl LegacySessionArtifactPayload {
    pub fn into_entry(self) -> SessionArtifactEntry {
        let messages = self
            .messages
            .into_iter()
            .map(LegacySessionArtifactMessage::into_message)
            .collect();
        SessionArtifactEntry::new(self.session, messages)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySessionArtifactMessage {
    #[serde(rename = "info")]
    pub message: SessionMessage,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
}

impl LegacySessionArtifactMessage {
    pub fn into_message(self) -> SessionMessage {
        let mut message = self.message;
        if message.parts.is_empty() {
            message.parts = self.parts;
        }
        message
    }
}

fn classify_metadata_keys(
    metadata: &HashMap<String, serde_json::Value>,
    sanctioned_keys: &[&str],
    sanctioned_prefixes: &[&str],
) -> SessionArtifactMetadataKeyClassification {
    let mut sanctioned = Vec::new();
    let mut passthrough = Vec::new();

    for key in metadata.keys() {
        if is_sanctioned_metadata_key(key, sanctioned_keys, sanctioned_prefixes) {
            sanctioned.push(key.clone());
        } else {
            passthrough.push(key.clone());
        }
    }

    sanctioned.sort();
    passthrough.sort();

    SessionArtifactMetadataKeyClassification {
        sanctioned_keys: sanctioned,
        passthrough_keys: passthrough,
    }
}

fn is_sanctioned_metadata_key(
    key: &str,
    sanctioned_keys: &[&str],
    sanctioned_prefixes: &[&str],
) -> bool {
    sanctioned_keys.contains(&key)
        || sanctioned_prefixes
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn latest_message_metadata_value(
    messages: &[SessionMessage],
    key: &str,
) -> Option<serde_json::Value> {
    messages
        .iter()
        .rev()
        .find_map(|message| message.metadata.get(key).cloned())
}

fn ingress_stabilization_from_session(
    session: &Session,
) -> Option<SessionIngressStabilizationSummary> {
    let source = session
        .metadata
        .get("last_ingress_source")?
        .as_str()?
        .to_string();
    let policy = session
        .metadata
        .get("last_ingress_policy")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let batch_count = session
        .metadata
        .get("last_ingress_batch_count")
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok());

    Some(SessionIngressStabilizationSummary {
        source,
        policy,
        batch_count,
    })
}

fn message_diagnostics_sidecar(
    message: &SessionMessage,
    tool_names: &HashMap<String, String>,
) -> Option<SessionArtifactMessageDiagnosticsSidecar> {
    let sidecar = SessionArtifactMessageDiagnosticsSidecar {
        message_id: message.id.clone(),
        role: message.role.clone(),
        cache_evidence_inspection: message.metadata.get("cache_evidence_inspection").cloned(),
        cache_evidence: message.metadata.get("cache_evidence").cloned(),
        prompt_surface_evidence: message.metadata.get("prompt_surface_evidence").cloned(),
        context_compaction_record: message.metadata.get("context_compaction_record").cloned(),
        context_compaction_continuity_packet: message
            .metadata
            .get("context_compaction_continuity_packet")
            .cloned(),
        provider_diagnostic: message.metadata.get("provider_diagnostic").cloned(),
        provider_error_summary: message.metadata.get("provider_error_summary").cloned(),
        execution_preflights: execution_preflights_from_message(message, tool_names),
    };

    (!sidecar.is_empty()).then_some(sidecar)
}

fn execution_preflights_from_message(
    message: &SessionMessage,
    tool_names: &HashMap<String, String>,
) -> Vec<SessionExecutionPreflightMetadataEntry> {
    let mut entries = Vec::new();
    for part in &message.parts {
        match &part.part_type {
            PartType::ToolCall {
                id,
                name,
                state: Some(ToolState::Completed { metadata, .. }),
                ..
            } => {
                if let Some(metadata) = metadata.get("preflight").cloned() {
                    entries.push(SessionExecutionPreflightMetadataEntry {
                        tool_call_id: id.clone(),
                        tool_name: Some(name.clone()),
                        source: SessionExecutionPreflightMetadataSource::ToolCallState,
                        metadata,
                    });
                }
            }
            PartType::ToolCall {
                id,
                name,
                state:
                    Some(ToolState::Error {
                        metadata: Some(metadata),
                        ..
                    }),
                ..
            } => {
                if let Some(metadata) = metadata.get("preflight").cloned() {
                    entries.push(SessionExecutionPreflightMetadataEntry {
                        tool_call_id: id.clone(),
                        tool_name: Some(name.clone()),
                        source: SessionExecutionPreflightMetadataSource::ToolCallState,
                        metadata,
                    });
                }
            }
            PartType::ToolResult {
                tool_call_id,
                metadata: Some(metadata),
                ..
            } => {
                if let Some(metadata) = metadata.get("preflight").cloned() {
                    entries.push(SessionExecutionPreflightMetadataEntry {
                        tool_call_id: tool_call_id.clone(),
                        tool_name: tool_names.get(tool_call_id).cloned(),
                        source: SessionExecutionPreflightMetadataSource::ToolResultPart,
                        metadata,
                    });
                }
            }
            _ => {}
        }
    }

    entries
}

fn tool_call_name_index(messages: &[SessionMessage]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        for part in &message.parts {
            if let PartType::ToolCall { id, name, .. } = &part.part_type {
                names.insert(id.clone(), name.clone());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use std::collections::HashMap;

    use crate::{
        CompletedTime, MessageRole, PartType, SessionArtifactBundle, SessionArtifactEntry,
        SessionArtifactImportEnvelope, SessionArtifactVersion, SessionMessage, SessionStatus,
        SessionTelemetrySnapshot, SessionTelemetrySnapshotVersion, SessionTime, SessionUsage,
        ToolState,
    };

    use super::{LegacySessionArtifactPayload, SessionDiagnosticsSidecar};

    fn sample_session() -> crate::Session {
        let now = Utc::now();
        crate::Session {
            id: "session-1".to_string(),
            slug: "session-1".to_string(),
            project_id: "project".to_string(),
            directory: "/tmp".to_string(),
            parent_id: None,
            title: "Example".to_string(),
            version: "1".to_string(),
            time: SessionTime {
                created: now.timestamp_millis(),
                updated: now.timestamp_millis(),
                compacting: None,
                archived: None,
            },
            messages: Vec::new(),
            summary: None,
            share: None,
            revert: None,
            permission: None,
            usage: Some(SessionUsage::default()),
            status: SessionStatus::Active,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_message() -> SessionMessage {
        SessionMessage {
            id: "message-1".to_string(),
            session_id: "session-1".to_string(),
            role: MessageRole::Assistant,
            parts: vec![crate::MessagePart {
                id: "part-1".to_string(),
                part_type: PartType::Text {
                    text: "hello".to_string(),
                    synthetic: None,
                    ignored: None,
                },
                created_at: Utc::now(),
                message_id: Some("message-1".to_string()),
            }],
            created_at: Utc::now(),
            metadata: HashMap::new(),
            usage: None,
            finish: Some("stop".to_string()),
        }
    }

    fn sample_telemetry_snapshot() -> SessionTelemetrySnapshot {
        SessionTelemetrySnapshot {
            version: SessionTelemetrySnapshotVersion::V1,
            usage: SessionUsage::default(),
            stage_summaries: Vec::new(),
            tool_repair_summary: None,
            memory: None,
            compaction_continuity: None,
            repair_query_snapshot: None,
            tool_trajectory_quality: None,
            tool_result_governance: None,
            pending_permission_count: 0,
            pending_followup_count: 0,
            granted_by_turn_count: 0,
            granted_by_session_count: 0,
            granted_by_matcher_kind: std::collections::BTreeMap::new(),
            last_permission_matcher_kind: None,
            last_permission_grant_target: None,
            last_permission_miss_count: 0,
            pending_steering_count: 0,
            consumed_steering_count: 0,
            last_steering_injected_at: None,
            last_steering_source_session_id: None,
            last_steering_latency_ms: None,
            last_permission_pending_ms: None,
            last_run_status: "completed".to_string(),
            updated_at: 123,
        }
    }

    #[test]
    fn bundle_serializes_with_stable_version_and_info_field() {
        let bundle = SessionArtifactBundle::new(
            123,
            vec![SessionArtifactEntry::new(
                sample_session(),
                vec![sample_message()],
            )],
        );

        let value = serde_json::to_value(&bundle).expect("bundle should serialize");

        assert_eq!(
            value["version"],
            SessionArtifactVersion::AgendaoRustV1.as_str()
        );
        assert!(value["sessions"][0].get("info").is_some());
        assert!(value["sessions"][0].get("session").is_none());
    }

    #[test]
    fn bundle_roundtrips_through_import_envelope() {
        let bundle = SessionArtifactBundle::new(
            123,
            vec![SessionArtifactEntry::new(
                sample_session(),
                vec![sample_message()],
            )],
        );

        let payload = serde_json::to_string(&bundle).expect("bundle should serialize");
        let envelope: SessionArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("bundle should parse");
        let entries = envelope.into_entries();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session.id, "session-1");
        assert_eq!(entries[0].messages.len(), 1);
    }

    #[test]
    fn entry_serializes_metadata_authority_classification_start_point() {
        let mut session = sample_session();
        session
            .metadata
            .insert("last_ingress_source".to_string(), serde_json::json!("web"));
        session
            .metadata
            .insert("custom_session_key".to_string(), serde_json::json!(true));

        let mut message = sample_message();
        message.metadata.insert(
            "provider_diagnostic".to_string(),
            serde_json::json!({"code": "thinking_replay_rejected"}),
        );
        message.metadata.insert(
            "context_compaction_record".to_string(),
            serde_json::json!({"trigger": "auto_preflight"}),
        );
        message.metadata.insert(
            "context_compaction_continuity_packet".to_string(),
            serde_json::json!({"version": 1}),
        );
        message
            .metadata
            .insert("custom_message_key".to_string(), serde_json::json!(1));

        let entry = SessionArtifactEntry::new(session, vec![message]);
        let value = serde_json::to_value(&entry).expect("entry should serialize");

        assert_eq!(
            value["metadata_authority"]["session"]["sanctioned_keys"],
            serde_json::json!(["last_ingress_source"])
        );
        assert_eq!(
            value["metadata_authority"]["session"]["passthrough_keys"],
            serde_json::json!(["custom_session_key"])
        );
        assert_eq!(
            value["metadata_authority"]["messages"][0]["message_id"],
            serde_json::json!("message-1")
        );
        assert_eq!(
            value["metadata_authority"]["messages"][0]["keys"]["sanctioned_keys"],
            serde_json::json!([
                "context_compaction_continuity_packet",
                "context_compaction_record",
                "provider_diagnostic"
            ])
        );
        assert_eq!(
            value["metadata_authority"]["messages"][0]["keys"]["passthrough_keys"],
            serde_json::json!(["custom_message_key"])
        );
    }

    #[test]
    fn entry_omits_metadata_authority_when_no_metadata_is_present() {
        let entry = SessionArtifactEntry::new(sample_session(), vec![sample_message()]);
        let value = serde_json::to_value(&entry).expect("entry should serialize");

        assert!(value.get("metadata_authority").is_none());
    }

    #[test]
    fn entry_derives_diagnostics_sidecar_from_persisted_owners() {
        let mut session = sample_session();
        session.metadata.insert(
            "telemetry".to_string(),
            serde_json::to_value(sample_telemetry_snapshot()).expect("snapshot should serialize"),
        );
        session.metadata.insert(
            "prompt_surface_state_snapshot".to_string(),
            serde_json::json!({"stable_prefix_hash": "abc123"}),
        );
        session.metadata.insert(
            "context_compaction_lifecycle_summary".to_string(),
            serde_json::json!({
                "trigger": "auto_preflight",
                "phase": "prompt.pre_request",
                "reason": "request_view_threshold",
                "status": "installed"
            }),
        );
        session
            .metadata
            .insert("last_ingress_source".to_string(), serde_json::json!("web"));
        session.metadata.insert(
            "last_ingress_policy".to_string(),
            serde_json::json!("same_session_context_batch"),
        );
        session
            .metadata
            .insert("last_ingress_batch_count".to_string(), serde_json::json!(2));
        session.metadata.insert(
            "memory_frozen_snapshot".to_string(),
            serde_json::json!({"packet": {"items": []}}),
        );
        session.metadata.insert(
            "memory_last_prefetch_packet".to_string(),
            serde_json::json!({"items": []}),
        );

        let mut message = sample_message();
        message.metadata.insert(
            "cache_evidence_inspection".to_string(),
            serde_json::json!({"reason": "system_prompt_changed"}),
        );
        message.metadata.insert(
            "cache_evidence".to_string(),
            serde_json::json!({"severity": "soft_warn"}),
        );
        message.metadata.insert(
            "prompt_surface_evidence".to_string(),
            serde_json::json!({"reason": "tool_surface_changed"}),
        );
        message.metadata.insert(
            "context_compaction_record".to_string(),
            serde_json::json!({
                "trigger": "auto_preflight",
                "reason": "request_view_threshold",
                "compacted_message_count": 4
            }),
        );
        message.metadata.insert(
            "provider_diagnostic".to_string(),
            serde_json::json!({"code": "thinking_replay_rejected"}),
        );
        message.metadata.insert(
            "provider_error_summary".to_string(),
            serde_json::json!({"kind": "invalid_request"}),
        );
        message.parts.push(crate::MessagePart {
            id: "part-2".to_string(),
            part_type: PartType::ToolCall {
                id: "tool-1".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"path": "README.md"}),
                status: crate::ToolCallStatus::Completed,
                raw: None,
                state: Some(ToolState::Completed {
                    input: serde_json::json!({"path": "README.md"}),
                    output: "ok".to_string(),
                    title: "Read".to_string(),
                    metadata: HashMap::from([(
                        "preflight".to_string(),
                        serde_json::json!({"runner": "media_inspect", "status": "ready"}),
                    )]),
                    time: CompletedTime {
                        start: 1,
                        end: 2,
                        compacted: None,
                    },
                    attachments: None,
                }),
            },
            created_at: Utc::now(),
            message_id: Some("message-1".to_string()),
        });

        let entry = SessionArtifactEntry::new(session, vec![message]);
        let value = serde_json::to_value(&entry).expect("entry should serialize");

        assert_eq!(
            value["diagnostics_sidecar"]["version"],
            serde_json::json!("agendao-rust/diagnostics/v1")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["telemetry"]["last_run_status"],
            serde_json::json!("completed")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["prompt_surface_state_snapshot"]["stable_prefix_hash"],
            serde_json::json!("abc123")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["context_compaction_lifecycle_summary"]["status"],
            serde_json::json!("installed")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["ingress_stabilization"]["source"],
            serde_json::json!("web")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["ingress_stabilization"]["batch_count"],
            serde_json::json!(2)
        );
        assert_eq!(
            value["diagnostics_sidecar"]["messages"][0]["provider_diagnostic"]["code"],
            serde_json::json!("thinking_replay_rejected")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["messages"][0]["context_compaction_record"]["reason"],
            serde_json::json!("request_view_threshold")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["messages"][0]["execution_preflights"][0]["tool_name"],
            serde_json::json!("read")
        );
        assert_eq!(
            value["diagnostics_sidecar"]["messages"][0]["execution_preflights"][0]["source"],
            serde_json::json!("tool_call_state")
        );
    }

    #[test]
    fn diagnostics_sidecar_helper_prefers_nested_provider_diagnostic() {
        let session = sample_session();
        let mut message = sample_message();
        message.metadata.insert(
            "provider_diagnostic".to_string(),
            serde_json::json!({"code": "direct_diagnostic"}),
        );
        message.metadata.insert(
            "provider_error_summary".to_string(),
            serde_json::json!({
                "kind": "invalid_request",
                "provider_diagnostic": {
                    "code": "nested_diagnostic",
                    "provider_id": "deepseek"
                }
            }),
        );

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[message])
            .expect("sidecar should exist");
        let diagnostic = sidecar
            .latest_provider_diagnostic_value()
            .expect("diagnostic should exist");

        assert_eq!(diagnostic["code"], serde_json::json!("nested_diagnostic"));
        assert_eq!(diagnostic["provider_id"], serde_json::json!("deepseek"));
    }

    #[test]
    fn diagnostics_sidecar_helper_reads_latest_context_compaction_record() {
        let session = sample_session();
        let mut message = sample_message();
        message.metadata.insert(
            "context_compaction_record".to_string(),
            serde_json::json!({
                "trigger": "overflow_recovery",
                "forced": true,
                "compacted_message_count": 3
            }),
        );

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[message])
            .expect("sidecar should exist");
        let record = sidecar
            .latest_context_compaction_record_value()
            .expect("compaction record should exist");

        assert_eq!(record["trigger"], serde_json::json!("overflow_recovery"));
        assert_eq!(record["forced"], serde_json::json!(true));
        assert_eq!(record["compacted_message_count"], serde_json::json!(3));
    }

    #[test]
    fn diagnostics_sidecar_helper_reads_latest_context_compaction_continuity_packet() {
        let session = sample_session();
        let mut message = sample_message();
        message.metadata.insert(
            "context_compaction_continuity_packet".to_string(),
            serde_json::json!({
                "version": 1,
                "eligible_message_count": 4,
                "exact_recent_tail_count": 2,
                "latest_compaction_summary": {
                    "message_id": "message-1",
                    "summary": "Compacted packet-backed summary."
                }
            }),
        );

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[message])
            .expect("sidecar should exist");
        let packet = sidecar
            .latest_context_compaction_continuity_packet_value()
            .expect("packet should exist");

        assert_eq!(packet["version"], serde_json::json!(1));
        assert_eq!(
            packet["latest_compaction_summary"]["summary"],
            serde_json::json!("Compacted packet-backed summary.")
        );
    }

    #[test]
    fn diagnostics_sidecar_helper_reads_context_compaction_lifecycle_summary() {
        let mut session = sample_session();
        session.metadata.insert(
            "context_compaction_lifecycle_summary".to_string(),
            serde_json::json!({
                "trigger": "auto_preflight",
                "phase": "prompt.pre_request",
                "status": "started"
            }),
        );

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[])
            .expect("sidecar should exist");
        let record = sidecar
            .context_compaction_lifecycle_summary_value()
            .expect("compaction lifecycle should exist");

        assert_eq!(record["trigger"], serde_json::json!("auto_preflight"));
        assert_eq!(record["status"], serde_json::json!("started"));
    }

    #[test]
    fn diagnostics_sidecar_helper_reads_context_compaction_decision_trace() {
        let mut session = sample_session();
        session.metadata.insert(
            "context_pressure_governance_summary".to_string(),
            serde_json::json!({
                "trigger": "auto_preflight",
                "phase": "prompt.pre_request",
                "status": "compacted",
                "decision_trace": {
                    "path": "prompt.pre_request",
                    "mode": "lightweight_trim",
                    "reason": "lightweight_tool_result_trim"
                }
            }),
        );

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[])
            .expect("sidecar should exist");
        let trace = sidecar
            .latest_context_compaction_decision_trace_value()
            .expect("decision trace should exist");

        assert_eq!(trace["path"], serde_json::json!("prompt.pre_request"));
        assert_eq!(trace["mode"], serde_json::json!("lightweight_trim"));
    }

    #[test]
    fn diagnostics_sidecar_helper_prefers_tool_call_state_preflight() {
        let session = sample_session();
        let mut assistant = sample_message();
        assistant.parts.push(crate::MessagePart {
            id: "part-2".to_string(),
            part_type: PartType::ToolCall {
                id: "tool-1".to_string(),
                name: "media_inspect".to_string(),
                input: serde_json::json!({"path": "README.md"}),
                status: crate::ToolCallStatus::Completed,
                raw: None,
                state: Some(ToolState::Completed {
                    input: serde_json::json!({"path": "README.md"}),
                    output: "ok".to_string(),
                    title: "Media Inspect".to_string(),
                    metadata: HashMap::from([(
                        "preflight".to_string(),
                        serde_json::json!({"runner": "tool_call_state", "status": "ready"}),
                    )]),
                    time: CompletedTime {
                        start: 1,
                        end: 2,
                        compacted: None,
                    },
                    attachments: None,
                }),
            },
            created_at: Utc::now(),
            message_id: Some("message-1".to_string()),
        });

        let mut tool = SessionMessage::tool("session-1");
        tool.id = "message-2".to_string();
        tool.parts.push(crate::MessagePart {
            id: "part-3".to_string(),
            part_type: PartType::ToolResult {
                tool_call_id: "tool-1".to_string(),
                content: "delegated".to_string(),
                is_error: false,
                title: None,
                metadata: Some(HashMap::from([(
                    "preflight".to_string(),
                    serde_json::json!({"runner": "tool_result", "status": "soft_warn"}),
                )])),
                attachments: None,
            },
            created_at: Utc::now(),
            message_id: Some("message-2".to_string()),
        });

        let sidecar = SessionDiagnosticsSidecar::derive_from_parts(&session, &[assistant, tool])
            .expect("sidecar should exist");
        let preflight = sidecar
            .latest_execution_preflight_entry()
            .expect("preflight should exist");
        let metadata = preflight
            .decode_metadata::<serde_json::Value>()
            .expect("metadata should decode");

        assert_eq!(
            preflight.source,
            super::SessionExecutionPreflightMetadataSource::ToolCallState
        );
        assert_eq!(preflight.tool_name.as_deref(), Some("media_inspect"));
        assert_eq!(metadata["runner"], serde_json::json!("tool_call_state"));
    }

    #[test]
    fn diagnostics_sidecar_does_not_promote_route_local_rich_fields() {
        let mut session = sample_session();
        session.metadata.insert(
            "telemetry".to_string(),
            serde_json::to_value(sample_telemetry_snapshot()).expect("snapshot should serialize"),
        );

        let entry = SessionArtifactEntry::new(session, vec![sample_message()]);
        let value = serde_json::to_value(&entry).expect("entry should serialize");
        let diagnostics = &value["diagnostics_sidecar"];

        assert!(diagnostics.get("runtime").is_none());
        assert!(diagnostics.get("stages").is_none());
        assert!(diagnostics.get("topology").is_none());
        assert_eq!(diagnostics["telemetry"]["version"], serde_json::json!("v1"));
    }

    #[test]
    fn classifier_demotes_narrow_or_noncanonical_metadata_keys() {
        let mut session = sample_session();
        session.metadata.insert(
            "legacy_session_banner".to_string(),
            serde_json::json!("recent turns"),
        );

        let mut message = sample_message();
        message.metadata.insert(
            "summary_title".to_string(),
            serde_json::json!("Short summary"),
        );
        message.metadata.insert(
            "scheduler_handoff_command".to_string(),
            serde_json::json!("/run plan.md"),
        );

        let entry = SessionArtifactEntry::new(session, vec![message]);
        let value = serde_json::to_value(&entry).expect("entry should serialize");

        assert!(value["metadata_authority"]["session"]
            .get("sanctioned_keys")
            .is_none());
        assert_eq!(
            value["metadata_authority"]["session"]["passthrough_keys"],
            serde_json::json!(["legacy_session_banner"])
        );
        assert!(value["metadata_authority"]["messages"][0]["keys"]
            .get("sanctioned_keys")
            .is_none());
        assert_eq!(
            value["metadata_authority"]["messages"][0]["keys"]["passthrough_keys"],
            serde_json::json!(["scheduler_handoff_command", "summary_title"])
        );
    }

    #[test]
    fn import_envelope_normalizes_single_entry() {
        let entry = SessionArtifactEntry::new(sample_session(), vec![sample_message()]);
        let payload = serde_json::to_string(&entry).expect("entry should serialize");
        let envelope: SessionArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("single entry should parse");

        let entries = envelope.into_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session.id, "session-1");
        assert_eq!(entries[0].messages.len(), 1);
    }

    #[test]
    fn import_envelope_ignores_invalid_diagnostics_sidecar_payload() {
        let payload = serde_json::json!({
            "info": sample_session(),
            "messages": [sample_message()],
            "diagnostics_sidecar": {
                "version": "agendao-rust/diagnostics/v999",
                "runtime": {"state": "active"}
            }
        });

        let envelope: SessionArtifactImportEnvelope =
            serde_json::from_value(payload).expect("single entry should still parse");
        let entries = envelope.into_entries();

        assert_eq!(entries.len(), 1);
        assert!(entries[0].diagnostics_sidecar.is_none());
    }

    #[test]
    fn import_envelope_normalizes_legacy_messages() {
        let legacy = LegacySessionArtifactPayload {
            session: sample_session(),
            messages: vec![super::LegacySessionArtifactMessage {
                message: SessionMessage {
                    parts: Vec::new(),
                    ..sample_message()
                },
                parts: vec![crate::MessagePart {
                    id: "legacy-part".to_string(),
                    part_type: PartType::Text {
                        text: "legacy".to_string(),
                        synthetic: None,
                        ignored: None,
                    },
                    created_at: Utc::now(),
                    message_id: Some("message-1".to_string()),
                }],
            }],
        };

        let payload = serde_json::to_string(&legacy).expect("legacy should serialize");
        let envelope: SessionArtifactImportEnvelope =
            serde_json::from_str(&payload).expect("legacy should parse");
        let entries = envelope.into_entries();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].messages.len(), 1);
        assert_eq!(entries[0].messages[0].parts.len(), 1);
        match &entries[0].messages[0].parts[0].part_type {
            PartType::Text { text, .. } => assert_eq!(text, "legacy"),
            other => panic!("expected text part, got {other:?}"),
        }
    }

    #[test]
    fn import_envelope_rejects_unknown_bundle_version() {
        let payload = serde_json::json!({
            "version": "agendao-rust/v999",
            "exported_at": 123,
            "sessions": [{
                "info": sample_session(),
                "messages": [sample_message()]
            }]
        });

        let error = serde_json::from_value::<SessionArtifactImportEnvelope>(payload)
            .expect_err("unknown version should fail closed");
        assert!(
            error.to_string().contains("did not match any variant")
                || error.to_string().contains("unknown variant")
        );
    }
}
