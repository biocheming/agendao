use chrono::{DateTime, Utc};
use rocode_content::stage_protocol::StageStatus;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;

use crate::{
    MemoryScope, ProviderConnectionDescriptorCandidate, ProviderProfileDescriptorView,
    SessionMemoryTelemetrySummary, SessionRepairQuerySnapshot, ToolTrajectoryQualitySummary,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffs: Option<Vec<FileDiff>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionShare {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRevert {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionRuleset {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTime {
    pub created: i64,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacting: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<i64>,
}

impl Default for SessionTime {
    fn default() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            created: now,
            updated: now,
            compacting: None,
            archived: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_miss_tokens: u64,
    /// Latest prompt/context window occupancy for the session.
    /// Unlike the other fields, this is not cumulative; it tracks the most
    /// recent assistant turn's context pressure for UI meters and compaction.
    #[serde(default)]
    pub context_tokens: u64,
    pub total_cost: f64,
}

impl SessionUsage {
    /// Owner-local cumulative usage for this session only.
    pub fn session_cumulative_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.reasoning_tokens
    }

    /// Session-owned live context for the next request on this session.
    pub fn live_context_tokens(&self) -> Option<u64> {
        (self.context_tokens > 0).then_some(self.context_tokens)
    }

    pub fn workflow_usage_summary(&self) -> WorkflowUsageSummary {
        WorkflowUsageSummary {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            cache_write_tokens: self.cache_write_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_miss_tokens: self.cache_miss_tokens,
            total_cost: self.total_cost,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct WorkflowUsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_miss_tokens: u64,
    pub total_cost: f64,
}

impl WorkflowUsageSummary {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.reasoning_tokens
    }

    pub fn accumulate_session_usage(&mut self, usage: &SessionUsage) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.reasoning_tokens += usage.reasoning_tokens;
        self.cache_write_tokens += usage.cache_write_tokens;
        self.cache_read_tokens += usage.cache_read_tokens;
        self.cache_miss_tokens += usage.cache_miss_tokens;
        self.total_cost += usage.total_cost;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SessionUsageBooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default)]
    pub workflow_cumulative: WorkflowUsageSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompactionSummary {
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub forced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_count_before: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_message_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_message_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCompactionLifecycleStatus {
    Started,
    Installed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompactionInstalledDiagnostics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompactionLifecycleSummary {
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub status: ContextCompactionLifecycleStatus,
    #[serde(default)]
    pub forced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<ContextCompactionInstalledDiagnostics>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressureGovernanceStatus {
    Ready,
    Compacted,
    Deferred,
    Blocked,
}

impl ContextPressureGovernanceStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Compacted => "compacted",
            Self::Deferred => "deferred",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPressureGovernanceSummary {
    pub trigger: String,
    pub phase: String,
    pub status: ContextPressureGovernanceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_pressure_percent: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_pressure_percent: Option<u64>,
    #[serde(default)]
    pub compaction_attempted: bool,
    #[serde(default)]
    pub compaction_succeeded: bool,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightweight_trim: Option<LightweightTrimSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_trace: Option<ContextCompactionDecisionTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextCompactionDecisionTrace {
    pub path: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<ContextCompactionAssessmentSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff: Option<ContextCompactionBackoffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightweight_trim: Option<LightweightTrimSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextCompactionAssessmentSummary {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextCompactionBackoffSummary {
    pub last_compaction_index: usize,
    pub messages_since_last: usize,
    pub user_turns_since_last: usize,
    pub recent_compaction_count: usize,
    pub min_messages_after_last: usize,
    pub min_user_turns_after_last: usize,
    pub recent_window_messages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LightweightTrimSummary {
    #[serde(default)]
    pub trimmed_rounds: usize,
    #[serde(default)]
    pub trimmed_tool_calls: usize,
    #[serde(default)]
    pub trimmed_tool_results: usize,
    #[serde(default)]
    pub trimmed_call_tokens: usize,
    #[serde(default)]
    pub trimmed_result_tokens: usize,
    #[serde(default)]
    pub used_round_grouping: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionCacheSeverity {
    Stable,
    LowChange,
    MediumChange,
    HighChange,
}

impl SessionCacheSeverity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::LowChange => "low change",
            Self::MediumChange => "medium change",
            Self::HighChange => "high change",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionCacheSemanticsBasis {
    #[default]
    ApiView,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionCacheBoundaryKind {
    Compaction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSurfaceEvidenceSummary {
    pub severity: SessionCacheSeverity,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCacheEvidenceExplain {
    pub status: String,
    pub severity: SessionCacheSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
    #[serde(default)]
    pub change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCacheBoundarySummary {
    pub kind: SessionCacheBoundaryKind,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_count_before: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_message_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_message_count: Option<usize>,
    #[serde(default)]
    pub trimmed_model_visible_messages: usize,
    #[serde(default)]
    pub likely_changed_prefix: bool,
    #[serde(default)]
    pub possible_cache_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCacheSemanticsSummary {
    pub basis: SessionCacheSemanticsBasis,
    pub api_view_messages: usize,
    #[serde(default)]
    pub trimmed_model_visible_messages: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<SessionCacheBoundarySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_evidence: Option<SessionCacheEvidenceExplain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_evidence: Option<PromptSurfaceEvidenceSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContextClosureContract {
    pub prefix_stability: SessionPrefixStabilityContract,
    pub compaction_boundary: SessionCompactionBoundaryContract,
    pub cache_explainability: SessionCacheExplainabilityContract,
    pub child_history_isolation: SessionChildHistoryIsolationContract,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionCompactionContinuityInspectionSource {
    ContinuityPacket,
    RawSummaryFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCompactionContinuityInspection {
    pub source: SessionCompactionContinuityInspectionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligible_message_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_recent_tail_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_older_turns: Option<usize>,
    #[serde(default)]
    pub has_working_ledger: bool,
    #[serde(default)]
    pub has_memory_anchors: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_policy: Option<String>,
}

impl SessionCompactionContinuityInspection {
    pub fn from_packet(packet: &SessionContinuityPacket) -> Self {
        Self {
            source: SessionCompactionContinuityInspectionSource::ContinuityPacket,
            summary_message_id: packet
                .latest_compaction_summary
                .as_ref()
                .map(|summary| summary.message_id.clone()),
            summary_text: packet
                .latest_compaction_summary
                .as_ref()
                .map(|summary| summary.summary.trim().to_string())
                .filter(|summary| !summary.is_empty()),
            eligible_message_count: Some(packet.eligible_message_count),
            exact_recent_tail_count: Some(packet.exact_recent_tail_count),
            omitted_older_turns: Some(packet.omitted_older_turns),
            has_working_ledger: !packet.working_ledger.is_empty(),
            has_memory_anchors: !packet.memory_anchors.is_empty(),
            recall_policy: packet
                .recall_policy
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        }
    }

    pub fn from_raw_summary(
        summary: &ContextCompactionSummary,
        summary_message_id: Option<String>,
    ) -> Option<Self> {
        let summary_text = summary
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if summary_text.is_none() && summary_message_id.is_none() {
            return None;
        }
        Some(Self {
            source: SessionCompactionContinuityInspectionSource::RawSummaryFallback,
            summary_message_id,
            summary_text,
            eligible_message_count: summary.message_count_before,
            exact_recent_tail_count: summary.kept_message_count,
            omitted_older_turns: summary.compacted_message_count,
            has_working_ledger: false,
            has_memory_anchors: false,
            recall_policy: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPrefixStabilityContract {
    pub basis: SessionCacheSemanticsBasis,
    #[serde(default)]
    pub tracked_on_api_view: bool,
    #[serde(default)]
    pub api_view_messages: usize,
    #[serde(default)]
    pub trimmed_model_visible_messages: usize,
    #[serde(default)]
    pub prefix_change_detected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCompactionBoundaryContract {
    #[serde(default)]
    pub boundary_recorded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_status: Option<ContextCompactionLifecycleStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_status: Option<ContextPressureGovernanceStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_pressure_percent: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_pressure_percent: Option<u64>,
    #[serde(default)]
    pub compaction_attempted: bool,
    #[serde(default)]
    pub compaction_succeeded: bool,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<ContextCompactionInstalledDiagnostics>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionCacheExplainabilitySource {
    #[default]
    None,
    CacheEvidence,
    SurfaceEvidence,
    BoundaryEvidence,
}

impl SessionCacheExplainabilitySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "no evidence",
            Self::CacheEvidence => "cache evidence",
            Self::SurfaceEvidence => "surface evidence",
            Self::BoundaryEvidence => "boundary evidence",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCacheExplainabilityContract {
    #[serde(default)]
    pub issue_present: bool,
    #[serde(default)]
    pub explained: bool,
    #[serde(default)]
    pub source: SessionCacheExplainabilitySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<SessionCacheSeverity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionChildHistoryIsolationContract {
    #[serde(default)]
    pub attached_subtree_session_count: usize,
    pub owner_session_cumulative_tokens: u64,
    pub workflow_cumulative_tokens: u64,
    pub attached_subtree_cumulative_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_live_context_tokens: Option<u64>,
    #[serde(default)]
    pub owner_local_live_prefix: bool,
    #[serde(default)]
    pub child_history_in_live_prefix_detected: bool,
    pub explanation: String,
}

impl SessionPrefixStabilityContract {
    pub fn status_label(&self) -> &'static str {
        if self.prefix_change_detected {
            "prefix changed"
        } else {
            "stable prefix"
        }
    }
}

impl SessionCompactionBoundaryContract {
    pub fn status_label(&self) -> &'static str {
        if self.boundary_recorded {
            "boundary recorded"
        } else {
            "boundary clear"
        }
    }
}

impl SessionCacheExplainabilityContract {
    pub fn status_label(&self) -> &'static str {
        if !self.issue_present {
            "cache stable"
        } else if self.explained {
            "cache explained"
        } else {
            "cache unexplained"
        }
    }
}

impl SessionChildHistoryIsolationContract {
    pub fn status_label(&self) -> &'static str {
        if self.child_history_in_live_prefix_detected {
            "leak detected"
        } else if self.owner_local_live_prefix {
            "isolated"
        } else {
            "not owner-local"
        }
    }
}

impl SessionContextClosureContract {
    pub fn coarse_diagnostic_label(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.cache_explainability.issue_present {
            parts.push(self.cache_explainability.status_label());
        }
        if self.prefix_stability.prefix_change_detected {
            parts.push(self.prefix_stability.status_label());
        } else if self.compaction_boundary.boundary_recorded {
            parts.push(self.compaction_boundary.status_label());
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContextExplain {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<SessionForkExplain>,
    pub raw_history_messages: usize,
    pub raw_model_visible_messages: usize,
    pub api_view_messages: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_view_estimated_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_view_body_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_request_context_tokens: Option<u64>,
    pub owner_session_cumulative_tokens: u64,
    pub workflow_cumulative_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionForkExplain {
    pub origin_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_message_id: Option<String>,
    #[serde(default)]
    pub history_mode: SessionForkHistoryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_message_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_history_messages: Option<usize>,
    #[serde(default)]
    pub imported_history_messages: usize,
    #[serde(default)]
    pub policy_frozen: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frozen_policy_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cache_stability_keys: Vec<String>,
    pub lifecycle: SessionForkLifecycleExplain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionForkLifecycleExplain {
    pub usage_replay_scope: SessionForkLifecycleScope,
    pub revert_scope: SessionForkLifecycleScope,
    pub recovery_scope: SessionForkLifecycleScope,
    pub compaction_scope: SessionForkLifecycleScope,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionForkLifecycleScope {
    LocalOnly,
    ForkPromptSurface,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionForkHistoryMode {
    None,
    #[default]
    All,
    LastN,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    Active,
    Completed,
    Archived,
    Compacting,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionContextKind {
    #[default]
    RootSessionContinuity,
    DelegatedSubsession,
    SchedulerStageOutputSession,
    ExplicitFullHistoryFork,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionHandoffMode {
    SelfContinuity,
    BoundedHandoff,
    StageOutputSink,
    FullHistoryFork,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionProviderModelRole {
    RequestShapeOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionWorkflowUsageRole {
    ObservationOnly,
}

pub const SESSION_CONTINUITY_PACKET_VERSION: u64 = 1;

fn default_session_continuity_packet_version() -> u64 {
    SESSION_CONTINUITY_PACKET_VERSION
}

fn string_is_empty(value: &str) -> bool {
    value.is_empty()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionContinuityLedgerKind {
    SessionTitle,
    SessionDiff,
    LatestUserTurn,
    LatestAssistantOutcome,
    SourcePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityLedgerEntry {
    pub kind: SessionContinuityLedgerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub text: String,
}

impl SessionContinuityLedgerEntry {
    pub fn new(kind: SessionContinuityLedgerKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            source_id: None,
            text: text.into(),
        }
    }

    pub fn with_source_id(
        kind: SessionContinuityLedgerKind,
        source_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            source_id: Some(source_id.into()),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityTurn {
    pub message_id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "string_is_empty")]
    pub text: String,
    #[serde(default)]
    pub projected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityCompactionSummary {
    pub message_id: String,
    #[serde(default, skip_serializing_if = "string_is_empty")]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityMemoryAnchor {
    pub record_id: String,
    #[serde(default, skip_serializing_if = "string_is_empty")]
    pub title: String,
    pub kind: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "string_is_empty")]
    pub why_recalled: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityLimits {
    #[serde(default)]
    pub recent_tail_messages: usize,
    #[serde(default)]
    pub context_text_chars: usize,
    #[serde(default)]
    pub turn_text_chars: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionContinuityDependencyKind {
    AssistantToolCallContinuation,
}

impl SessionContinuityDependencyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AssistantToolCallContinuation => "assistant_tool_call_continuation",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityDependency {
    pub kind: SessionContinuityDependencyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuityPacket {
    #[serde(default = "default_session_continuity_packet_version")]
    pub version: u64,
    #[serde(default)]
    pub eligible_message_count: usize,
    #[serde(default)]
    pub exact_recent_tail_count: usize,
    #[serde(default)]
    pub omitted_older_turns: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exact_recent_tail: Vec<SessionContinuityTurn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_anchors: Vec<SessionContinuityMemoryAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub working_ledger: Vec<SessionContinuityLedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuation_dependencies: Vec<SessionContinuityDependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_compaction_summary: Option<SessionContinuityCompactionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<SessionContinuityLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_policy: Option<String>,
}

impl Default for SessionContinuityPacket {
    fn default() -> Self {
        Self {
            version: SESSION_CONTINUITY_PACKET_VERSION,
            eligible_message_count: 0,
            exact_recent_tail_count: 0,
            omitted_older_turns: 0,
            exact_recent_tail: Vec::new(),
            memory_anchors: Vec::new(),
            working_ledger: Vec::new(),
            continuation_dependencies: Vec::new(),
            latest_compaction_summary: None,
            limits: None,
            recall_policy: None,
        }
    }
}

impl SessionContinuityPacket {
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let packet = serde_json::from_value::<Self>(value.clone()).ok()?;
        (packet.version == SESSION_CONTINUITY_PACKET_VERSION).then_some(packet)
    }

    pub fn metadata_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("session continuity packet should serialize")
    }

    pub fn allowed_message_ids(&self) -> Vec<String> {
        let mut ids = self
            .exact_recent_tail
            .iter()
            .filter(|turn| !turn.projected)
            .map(|turn| turn.message_id.clone())
            .collect::<Vec<_>>();
        ids.extend(
            self.continuation_dependencies
                .iter()
                .flat_map(|dependency| dependency.message_ids.iter().cloned()),
        );
        if let Some(compaction) = self.latest_compaction_summary.as_ref() {
            ids.push(compaction.message_id.clone());
        }
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn allowed_memory_record_ids(&self) -> Vec<String> {
        let mut ids = self
            .memory_anchors
            .iter()
            .map(|anchor| anchor.record_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn stable_refs_value(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "eligible_message_count": self.eligible_message_count,
            "exact_recent_tail": self
                .exact_recent_tail
                .iter()
                .map(|turn| {
                    serde_json::json!({
                        "message_id": turn.message_id,
                        "role": turn.role,
                        "projected": turn.projected,
                    })
                })
                .collect::<Vec<_>>(),
            "memory_anchors": self
                .memory_anchors
                .iter()
                .map(|anchor| {
                    serde_json::json!({
                        "record_id": anchor.record_id,
                        "kind": anchor.kind,
                        "status": anchor.status,
                    })
                })
                .collect::<Vec<_>>(),
            "continuation_dependencies": self
                .continuation_dependencies
                .iter()
                .map(|dependency| {
                    serde_json::json!({
                        "kind": dependency.kind,
                        "anchor_message_id": dependency.anchor_message_id,
                        "message_ids": dependency.message_ids,
                    })
                })
                .collect::<Vec<_>>(),
            "latest_compaction_summary": self
                .latest_compaction_summary
                .as_ref()
                .map(|summary| summary.message_id.clone()),
        })
    }

    pub fn render(&self) -> String {
        let mut sections = vec!["## Session Continuity Context\n\
This is same-session continuity context for resolving follow-up references such as \
`previous`, `above`, `继续`, `前面`, `刚才`, or `把结果写入`. Treat it as task context, \
not as a replacement for checking live files or rerunning verification when exact state matters."
            .to_string()];

        sections.push(self.render_context_coverage());

        let source_anchors = self.render_source_anchors();
        if !source_anchors.is_empty() {
            sections.push(format!("## Source Anchors\n{source_anchors}"));
        }
        let memory_anchors = self.render_memory_anchors();
        if !memory_anchors.is_empty() {
            sections.push(format!("## Memory Anchors\n{memory_anchors}"));
        }
        let continuation_dependencies = self.render_continuation_dependencies();
        if !continuation_dependencies.is_empty() {
            sections.push(format!(
                "## Continuation Dependencies\n{continuation_dependencies}"
            ));
        }

        sections.push(self.render_hydration_guidance());

        if !self.working_ledger.is_empty() {
            sections.push(format!(
                "## Working Ledger\n{}",
                self.working_ledger
                    .iter()
                    .map(|entry| format!("- {}", entry.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        if let Some(compaction) = self.latest_compaction_summary.as_ref() {
            sections.push(format!(
                "## Latest Compaction Summary\nsource: assistant `{}`\n{}",
                compaction.message_id,
                self.truncate_turn_text(&compaction.summary)
            ));
        }

        if !self.exact_recent_tail.is_empty() {
            let turns = self
                .exact_recent_tail
                .iter()
                .map(|turn| {
                    let source_kind = if turn.projected { "projected" } else { "exact" };
                    format!(
                        "- {} `{}` ({source_kind}):\n{}",
                        turn.role,
                        turn.message_id,
                        self.indent_block(&self.truncate_turn_text(&turn.text))
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("## Exact Recent Tail\n{turns}"));
        }

        self.truncate_context_text(&sections.join("\n\n"))
    }

    fn render_context_coverage(&self) -> String {
        let exact_count = self
            .exact_recent_tail_count
            .max(self.exact_recent_tail.len());
        let omitted_count = self.omitted_older_turns.max(
            self.eligible_message_count
                .saturating_sub(self.exact_recent_tail.len()),
        );
        let mut rows = vec![
            format!(
                "- exact_recent_tail: last {exact_count} of {} eligible user/assistant messages",
                self.eligible_message_count
            ),
            format!("- omitted_older_turns: {omitted_count}"),
        ];
        if let Some(compaction) = self.latest_compaction_summary.as_ref() {
            rows.push(format!(
                "- latest_compaction_summary: assistant `{}`",
                compaction.message_id
            ));
        } else {
            rows.push("- latest_compaction_summary: none".to_string());
        }
        rows.push(format!(
            "- memory_anchors: {} recalled records",
            self.memory_anchors.len()
        ));
        rows.push(format!(
            "- continuation_dependencies: {} exact chain(s)",
            self.continuation_dependencies.len()
        ));
        rows.push(format!(
            "- recall_policy: {}",
            self.recall_policy.as_deref().unwrap_or(
                "use exact tail for recent follow-up references; treat ledger and compaction as lossy summaries; use `scheduler_context_hydrate` for authorized Source Anchors when prior exact text is needed; use `scheduler_memory_hydrate` for authorized Memory Anchors when exact memory detail is needed; use memory, artifacts, or other tools for facts outside the anchors."
            )
        ));
        format!("## Context Coverage\n{}", rows.join("\n"))
    }

    fn render_hydration_guidance(&self) -> String {
        let omitted_count = self.omitted_older_turns.max(
            self.eligible_message_count
                .saturating_sub(self.exact_recent_tail.len()),
        );
        let mut rows = vec![
            "- Use `scheduler_context_hydrate({\"message_ids\":[...]})` only with ids listed in Source Anchors when the current task needs exact prior text that is truncated, ambiguous, or summarized.".to_string(),
            "- Do not invent message ids. The runtime rejects ids that are not authorized by the scheduler continuity packet.".to_string(),
            "- Prefer the visible Exact Recent Tail when it already contains the needed prior output.".to_string(),
            "- Use `scheduler_memory_hydrate({\"record_ids\":[...]})` only with ids listed in Memory Anchors when exact recalled memory details matter.".to_string(),
        ];
        if !self.continuation_dependencies.is_empty() {
            rows.push(
                "- Preserve Continuation Dependency message ids as exact assistant/tool history when provider continuation or reasoning replay depends on them.".to_string(),
            );
        }
        if omitted_count > 0 {
            rows.push(format!(
                "- omitted_older_turns is {omitted_count}; if the user refers to older context outside Source Anchors, recover it through memory, artifacts, or other tools before acting."
            ));
        }
        format!("## Hydration Guidance\n{}", rows.join("\n"))
    }

    fn render_source_anchors(&self) -> String {
        let mut anchors = Vec::new();
        if !self.exact_recent_tail.is_empty() {
            anchors.push(format!(
                "- exact_tail_message_ids: {}",
                self.exact_recent_tail
                    .iter()
                    .map(|turn| format!("`{}`", turn.message_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(compaction) = self.latest_compaction_summary.as_ref() {
            anchors.push(format!(
                "- compaction_summary_message_id: `{}`",
                compaction.message_id
            ));
        }
        for dependency in &self.continuation_dependencies {
            let ids = dependency
                .message_ids
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let anchor = dependency
                .anchor_message_id
                .as_deref()
                .map(|id| format!(" anchored at `{id}`"))
                .unwrap_or_default();
            anchors.push(format!(
                "- continuation_dependency [{}]{}: {}",
                dependency.kind.label(),
                anchor,
                ids
            ));
        }
        anchors.join("\n")
    }

    fn render_continuation_dependencies(&self) -> String {
        self.continuation_dependencies
            .iter()
            .map(|dependency| {
                let anchor = dependency
                    .anchor_message_id
                    .as_deref()
                    .map(|id| format!("anchor `{id}`"))
                    .unwrap_or_else(|| "no explicit anchor".to_string());
                let ids = dependency
                    .message_ids
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("- {} ({anchor}): {ids}", dependency.kind.label())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_memory_anchors(&self) -> String {
        self.memory_anchors
            .iter()
            .map(|anchor| {
                format!(
                    "- memory `{}` [{} / {}]: {}\n  why: {}",
                    anchor.record_id, anchor.kind, anchor.status, anchor.title, anchor.why_recalled
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn limits_or_default(&self) -> SessionContinuityLimits {
        self.limits.clone().unwrap_or(SessionContinuityLimits {
            recent_tail_messages: 0,
            context_text_chars: 4000,
            turn_text_chars: 1200,
        })
    }

    fn truncate_turn_text(&self, value: &str) -> String {
        truncate_chars(value, self.limits_or_default().turn_text_chars.max(1))
    }

    fn truncate_context_text(&self, value: &str) -> String {
        truncate_chars(value, self.limits_or_default().context_text_chars.max(1))
    }

    fn indent_block(&self, text: &str) -> String {
        text.lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub const CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY: &str =
    "context_compaction_continuity_packet";

pub fn message_continuity_packet(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<SessionContinuityPacket> {
    metadata
        .get(CONTEXT_COMPACTION_CONTINUITY_PACKET_METADATA_KEY)
        .and_then(SessionContinuityPacket::from_value)
}

pub fn message_latest_compaction_summary(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    fallback_message_id: &str,
    fallback_text: Option<&str>,
) -> Option<SessionContinuityCompactionSummary> {
    if let Some(summary) =
        message_continuity_packet(metadata).and_then(|packet| packet.latest_compaction_summary)
    {
        let trimmed = summary.summary.trim();
        if !trimmed.is_empty() {
            return Some(SessionContinuityCompactionSummary {
                message_id: summary.message_id,
                summary: trimmed.to_string(),
            });
        }
    }

    let trimmed = fallback_text
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(SessionContinuityCompactionSummary {
        message_id: fallback_message_id.to_string(),
        summary: trimmed.to_string(),
    })
}

fn truncate_chars(value: &str, limit: usize) -> String {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubsessionHandoffRichness {
    #[default]
    Bounded,
    Enriched,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubsessionHandoffFieldKind {
    Goal,
    Constraint,
    Fact,
    RequiredPath,
    SupportingContext,
    PreflightContext,
    RecentConclusion,
    SanctionedRecentTail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubsessionHandoffField {
    pub kind: SubsessionHandoffFieldKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub text: String,
}

impl SubsessionHandoffField {
    pub fn new(kind: SubsessionHandoffFieldKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            title: None,
            text: text.into(),
        }
    }

    pub fn titled(
        kind: SubsessionHandoffFieldKind,
        title: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: Some(title.into()),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SubsessionHandoffPacket {
    #[serde(default)]
    pub richness: SubsessionHandoffRichness,
    #[serde(default)]
    pub fields: Vec<SubsessionHandoffField>,
}

impl SubsessionHandoffPacket {
    pub fn bounded_goal(goal: impl Into<String>) -> Self {
        let mut packet = Self::default();
        packet.push_text(SubsessionHandoffFieldKind::Goal, goal);
        packet
    }

    pub fn push_field(&mut self, field: SubsessionHandoffField) {
        self.fields.push(field);
    }

    pub fn push_text(&mut self, kind: SubsessionHandoffFieldKind, text: impl Into<String>) {
        self.push_field(SubsessionHandoffField::new(kind, text));
    }

    pub fn push_titled_text(
        &mut self,
        kind: SubsessionHandoffFieldKind,
        title: impl Into<String>,
        text: impl Into<String>,
    ) {
        self.push_field(SubsessionHandoffField::titled(kind, title, text));
    }

    pub fn effective_richness(&self) -> SubsessionHandoffRichness {
        if self.fields.iter().any(|field| {
            matches!(
                field.kind,
                SubsessionHandoffFieldKind::SupportingContext
                    | SubsessionHandoffFieldKind::PreflightContext
                    | SubsessionHandoffFieldKind::RecentConclusion
                    | SubsessionHandoffFieldKind::SanctionedRecentTail
            )
        }) {
            SubsessionHandoffRichness::Enriched
        } else {
            self.richness
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubsessionResultAbsorbMode {
    SummaryOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubsessionResultEnvelope {
    pub absorb_mode: SubsessionResultAbsorbMode,
    pub text: String,
}

impl SubsessionResultEnvelope {
    pub fn summary(text: impl Into<String>) -> Self {
        Self {
            absorb_mode: SubsessionResultAbsorbMode::SummaryOnly,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionOwnershipSummary {
    pub context_kind: SessionContextKind,
    pub handoff_mode: SessionHandoffMode,
    #[serde(default)]
    pub owns_prompt_continuity: bool,
    #[serde(default)]
    pub compact_owner: bool,
    pub provider_model_role: SessionProviderModelRole,
    pub workflow_usage_role: SessionWorkflowUsageRole,
}

impl SessionContextKind {
    /// Whether this session kind is expected to own an ongoing prompt surface
    /// that can accumulate context pressure and may need compaction.
    pub fn owns_prompt_continuity(self) -> bool {
        !matches!(self, Self::SchedulerStageOutputSession)
    }

    pub fn handoff_mode(self) -> SessionHandoffMode {
        match self {
            Self::RootSessionContinuity => SessionHandoffMode::SelfContinuity,
            Self::DelegatedSubsession => SessionHandoffMode::BoundedHandoff,
            Self::SchedulerStageOutputSession => SessionHandoffMode::StageOutputSink,
            Self::ExplicitFullHistoryFork => SessionHandoffMode::FullHistoryFork,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RootSessionContinuity => "root continuity",
            Self::DelegatedSubsession => "delegated subsession",
            Self::SchedulerStageOutputSession => "stage output sink",
            Self::ExplicitFullHistoryFork => "full-history fork",
        }
    }

    pub fn ownership_summary(self) -> SessionOwnershipSummary {
        let owns_prompt_continuity = self.owns_prompt_continuity();
        SessionOwnershipSummary {
            context_kind: self,
            handoff_mode: self.handoff_mode(),
            owns_prompt_continuity,
            compact_owner: owns_prompt_continuity,
            provider_model_role: SessionProviderModelRole::RequestShapeOnly,
            workflow_usage_role: SessionWorkflowUsageRole::ObservationOnly,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunStatus {
    Idle,
    Busy,
    Retrying { attempt: u32 },
}

use crate::message::SessionMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTime,
    pub messages: Vec<SessionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<SessionShare>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert: Option<SessionRevert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionRuleset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
    #[serde(default)]
    pub status: SessionStatus,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing)]
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn touch(&mut self) {
        let now = Utc::now();
        self.time.updated = now.timestamp_millis();
        self.updated_at = now;
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTelemetrySnapshotVersion {
    #[default]
    V1,
    V2,
    V3,
    V4,
    V5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRepairCount {
    pub key: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRepairToolSummary {
    pub tool_name: String,
    pub call_count: u64,
    pub repaired_call_count: u64,
    pub error_call_count: u64,
    pub repair_event_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_kinds: Vec<ToolRepairCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionToolRepairTelemetrySummary {
    pub total_tool_calls: u64,
    pub repaired_tool_call_count: u64,
    pub error_tool_call_count: u64,
    pub repair_event_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub provider_diagnostic_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_diagnostic_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_layers: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolRepairToolSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelToolRepairTelemetrySummary {
    pub provider_id: String,
    pub model_id: String,
    pub session_count: u64,
    pub repaired_session_count: u64,
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub error_session_count: u64,
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub provider_diagnostic_session_count: u64,
    pub total_tool_calls: u64,
    pub repaired_tool_call_count: u64,
    pub error_tool_call_count: u64,
    pub repair_event_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub provider_diagnostic_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_diagnostic_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_kinds: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_layers: Vec<ToolRepairCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolRepairToolSummary>,
}

fn u64_is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListSummary {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionListItem {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SessionListSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<SessionListHints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_command_invocation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionListContract {
    pub filter_query_parameters: Vec<String>,
    pub search_fields: Vec<String>,
    pub non_search_fields: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionListResponse {
    pub items: Vec<SessionListItem>,
    pub contract: SessionListContract,
}

pub type SessionTimeInfo = SessionTime;
pub type SessionSummaryInfo = SessionListSummary;
pub type SessionShareInfo = SessionShare;
pub type SessionRevertInfo = SessionRevert;
pub type PermissionRulesetInfo = PermissionRuleset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMultimodalAttachmentInfo {
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMultimodalInsight {
    pub user_message_id: String,
    pub attachment_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_downgrade: Option<String>,
    #[serde(default)]
    pub hard_block: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_replaced_parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SessionMultimodalAttachmentInfo>,
}

impl SessionMultimodalInsight {
    pub fn display_label(&self) -> Cow<'_, str> {
        self.compact_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| {
                if self.attachment_count == 1 {
                    Cow::Borrowed("attachment-backed input")
                } else {
                    Cow::Owned(format!("{} attachments", self.attachment_count))
                }
            })
    }

    pub fn combined_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for warning in self
            .warnings
            .iter()
            .chain(self.transport_warnings.iter())
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !warnings.iter().any(|existing| existing == warning) {
                warnings.push(warning.to_string());
            }
        }
        warnings
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInsightsResponse {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub updated: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<SessionTelemetrySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<crate::SessionMemoryInsight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal: Option<SessionMultimodalInsight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_policy: Option<SessionEffectivePolicyView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectivePolicyView {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<SessionEffectiveSchedulerPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<SessionEffectiveProviderPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree: Option<SessionEffectiveSkillTreePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<SessionEffectiveMemoryPolicy>,
    pub compaction: SessionEffectiveCompactionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_adapter: Option<SessionEffectiveExternalAdapterPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEffectiveSchedulerTraceStepKind {
    RequestedProfile,
    CommandWorkflowOverride,
    SessionPinnedProfile,
    LegacySessionPinnedProfile,
    ConfigDefaultProfile,
    AutoRoute,
    SoftFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveSchedulerTraceStep {
    pub kind: SessionEffectiveSchedulerTraceStepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveSchedulerPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_profile: Option<String>,
    pub source: String,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selection_trace: Vec<SessionEffectiveSchedulerTraceStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveProviderRuntimeProfile {
    pub profile: ProviderProfileDescriptorView,
    pub profile_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveProviderPolicy {
    pub provider_id: String,
    pub model_id: String,
    pub resolved_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_descriptor: Option<ProviderConnectionDescriptorCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_descriptor_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile: Option<SessionEffectiveProviderRuntimeProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveSkillTreePolicy {
    pub configured: bool,
    pub enabled: bool,
    pub applied: bool,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveMemoryPolicy {
    pub workspace_key: String,
    pub workspace_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_scopes: Vec<MemoryScope>,
    #[serde(default)]
    pub frozen_snapshot_items: u32,
    #[serde(default)]
    pub last_prefetch_items: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveCompactionPolicy {
    pub auto: bool,
    pub prune: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionEffectiveExternalAdapterPolicy {
    pub last_ingress_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ingress_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ingress_batch_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub slug: String,
    pub project_id: String,
    pub directory: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub version: String,
    pub time: SessionTimeInfo,
    #[serde(default)]
    pub summary: Option<SessionSummaryInfo>,
    #[serde(default)]
    pub share: Option<SessionShareInfo>,
    #[serde(default)]
    pub revert: Option<SessionRevertInfo>,
    #[serde(default)]
    pub permission: Option<PermissionRulesetInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<SessionForkExplain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<SessionTelemetrySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusInfo {
    pub status: String,
    pub idle: bool,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedStageTelemetrySummary {
    pub stage_id: String,
    pub stage_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_total: Option<u64>,
    pub status: StageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_truncation_strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_tree_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_attempt: Option<u64>,
    pub active_agent_count: u32,
    pub active_tool_count: u32,
    pub attached_session_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_attached_session_id: Option<String>,
}

impl From<rocode_content::stage_protocol::StageSummary> for PersistedStageTelemetrySummary {
    fn from(value: rocode_content::stage_protocol::StageSummary) -> Self {
        Self {
            stage_id: value.stage_id,
            stage_name: value.stage_name,
            index: value.index,
            total: value.total,
            step: value.step,
            step_total: value.step_total,
            status: value.status,
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            reasoning_tokens: value.reasoning_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_write_tokens: value.cache_write_tokens,
            focus: value.focus,
            last_event: value.last_event,
            waiting_on: value.waiting_on,
            activity: value.activity,
            estimated_context_tokens: value.estimated_context_tokens,
            skill_tree_budget: value.skill_tree_budget,
            skill_tree_truncation_strategy: value.skill_tree_truncation_strategy,
            skill_tree_truncated: value.skill_tree_truncated,
            retry_attempt: value.retry_attempt,
            active_agent_count: value.active_agent_count,
            active_tool_count: value.active_tool_count,
            attached_session_count: value.attached_session_count,
            primary_attached_session_id: value.primary_attached_session_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTelemetrySnapshot {
    #[serde(default)]
    pub version: SessionTelemetrySnapshotVersion,
    pub usage: SessionUsage,
    #[serde(default)]
    pub stage_summaries: Vec<PersistedStageTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_repair_summary: Option<SessionToolRepairTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<SessionMemoryTelemetrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_continuity: Option<SessionCompactionContinuityInspection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_query_snapshot: Option<SessionRepairQuerySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_trajectory_quality: Option<ToolTrajectoryQualitySummary>,
    /// Steering telemetry (P4): counts and timestamps for mid-run steering.
    #[serde(default)]
    pub pending_steering_count: u64,
    #[serde(default)]
    pub consumed_steering_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_steering_injected_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_steering_source_session_id: Option<String>,
    pub last_run_status: String,
    pub updated_at: i64,
}
