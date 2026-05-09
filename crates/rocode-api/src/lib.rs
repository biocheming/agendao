use rocode_command::stage_protocol::StageSummary;
use rocode_session::SessionUsage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use rocode_multimodal::{
    ModalityKind, ModalityPreflightResult, MultimodalCapabilitiesResponse,
    MultimodalPolicyResponse, MultimodalPreflightRequest, MultimodalPreflightResponse,
    PreflightCapabilityView, PreflightInputPart,
};
pub use rocode_session::{
    PermissionRulesetInfo, SessionInfo, SessionListContract, SessionListHints, SessionListItem,
    SessionListResponse, SessionListTime, SessionRevertInfo, SessionShareInfo, SessionSummaryInfo,
    SessionTimeInfo,
};
pub use rocode_types::{
    ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
    ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
    ConfigPolicyValidationSnapshot, ContextCompactionLifecycleSummary, ContextCompactionSummary,
    ContextPressureGovernanceSummary, ExternalAdapterResolvedBinding, ExternalAdapterSource,
    ManagedSkillRecord, MemoryConflictResponse, MemoryConsolidationRequest,
    MemoryConsolidationResponse, MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery,
    MemoryDetailView, MemoryListQuery, MemoryListResponse, MemoryRetrievalPreviewResponse,
    MemoryRetrievalQuery, MemoryRuleHitListResponse, MemoryRuleHitQuery,
    MemoryRulePackListResponse, MemoryScope, MemoryValidationReportResponse,
    PromptSurfaceEvidenceSummary, ProposalStatus, ProviderConnectionDescriptorCandidate,
    ProviderProfileDescriptorView, SessionCacheSemanticsSummary, SessionContextClosureContract,
    SessionContextExplain, SessionContextKind, SessionEffectiveCompactionPolicy,
    SessionEffectiveExternalAdapterPolicy, SessionEffectiveMemoryPolicy,
    SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
    SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy,
    SessionEffectiveSchedulerTraceStep, SessionEffectiveSchedulerTraceStepKind,
    SessionEffectiveSkillTreePolicy, SessionForkExplain, SessionForkHistoryMode,
    SessionInsightsResponse, SessionMemoryTelemetrySummary, SessionOwnershipSummary,
    SessionStatusInfo, SessionUsageBooks, SkillArtifactCacheEntry, SkillAuditEvent,
    SkillDistributionRecord, SkillEvolutionProposal, SkillEvolutionProposalKind,
    SkillGovernanceDiagnosticSeverity, SkillGovernanceTimelineEntry, SkillGovernanceTimelineStatus,
    SkillGovernanceWriteResult, SkillGuardReport, SkillGuardStatus, SkillHubArtifactCacheResponse,
    SkillHubAuditResponse, SkillHubDistributionResponse, SkillHubGuardRunRequest,
    SkillHubGuardRunResponse, SkillHubIndexRefreshRequest, SkillHubIndexRefreshResponse,
    SkillHubIndexResponse, SkillHubLifecycleResponse, SkillHubManagedDetachRequest,
    SkillHubManagedDetachResponse, SkillHubManagedRemoveRequest, SkillHubManagedRemoveResponse,
    SkillHubManagedResponse, SkillHubNegativeEntropyResponse, SkillHubPolicy,
    SkillHubPolicyResponse, SkillHubRemoteInstallApplyRequest, SkillHubRemoteInstallPlanRequest,
    SkillHubRemoteUpdateApplyRequest, SkillHubRemoteUpdatePlanRequest,
    SkillHubReviewCandidatesSyncRequest, SkillHubReviewCandidatesSyncResponse,
    SkillHubSemanticConflictResponse, SkillHubSyncApplyRequest, SkillHubSyncPlanRequest,
    SkillHubSyncPlanResponse, SkillHubTimelineQuery, SkillHubTimelineResponse,
    SkillHubUsageLedgerResponse, SkillHubVitalityUpdateRequest, SkillHubVitalityUpdateResponse,
    SkillManagedLifecycleRecord, SkillNegativeEntropyDiagnostic, SkillNegativeEntropySignal,
    SkillOperationalSnapshot, SkillOperationalSourceScope, SkillRemoteInstallAction,
    SkillRemoteInstallEntry, SkillRemoteInstallPlan, SkillRemoteInstallResponse,
    SkillRetirementReason, SkillRetirementReasonKind, SkillSemanticConflictDiagnostic,
    SkillSemanticConflictKind, SkillSourceIndexSnapshot, SkillSourceKind, SkillSourceRef,
    SkillSyncPlan, SkillUsageLedgerEntry, SkillVitalityRecord, SkillVitalityState,
    SkillWriteLedgerAction, SkillWriteLedgerEntry,
};

pub type PromptPart = rocode_session::prompt::PartInput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    pub location: String,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub supporting_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFileRef {
    pub relative_path: String,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDetailMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    pub location: String,
    #[serde(default)]
    pub supporting_files: Vec<SkillFileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDetailSkill {
    pub meta: SkillDetailMeta,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRuntimeResolutionDiagnostic {
    pub inspection_available: bool,
    pub runtime_available: bool,
    pub vitality_state: rocode_types::SkillVitalityState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDetailResponse {
    pub skill: SkillDetailSkill,
    pub source: String,
    pub writable: bool,
    pub runtime_resolution: SkillRuntimeResolutionDiagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillManageAction {
    Create,
    Patch,
    Edit,
    WriteFile,
    RemoveFile,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManageRequest {
    pub session_id: String,
    pub action: SkillManageAction,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub methodology: Option<rocode_skill::SkillMethodologyTemplate>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub directory_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillManageResult {
    pub action: String,
    pub skill_name: String,
    pub location: String,
    #[serde(default)]
    pub supporting_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillManageResponse {
    #[serde(flatten)]
    pub result: SkillManageResult,
    #[serde(default)]
    pub guard_report: Option<SkillGuardReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillCatalogQuery {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub tool_policy: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub toolsets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillDetailQuery {
    pub name: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub tool_policy: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub toolsets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptResponse {
    pub status: String,
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub pending_question_id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingCommandInvocation {
    pub command: String,
    #[serde(rename = "rawArguments", default)]
    pub raw_arguments: String,
    #[serde(rename = "missingFields", default)]
    pub missing_fields: Vec<String>,
    #[serde(rename = "schedulerProfile", default)]
    pub scheduler_profile: Option<String>,
    #[serde(rename = "questionId", default)]
    pub question_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    PromptRun,
    SchedulerRun,
    SchedulerStage,
    ToolCall,
    AgentTask,
    Question,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Waiting,
    Cancelling,
    Retry,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExecutionNode {
    pub id: String,
    pub kind: ExecutionKind,
    pub status: ExecutionStatus,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub recent_event: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub children: Vec<SessionExecutionNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExecutionTopology {
    #[serde(alias = "sessionID", alias = "sessionId")]
    pub session_id: String,
    pub active_count: usize,
    #[serde(default)]
    pub done_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub cancelling_count: usize,
    pub retry_count: usize,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub roots: Vec<SessionExecutionNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRuntimeState {
    pub session_id: String,
    pub run_status: SessionRunStatusKind,
    #[serde(default)]
    pub current_message_id: Option<String>,
    #[serde(default)]
    pub usage: Option<SessionUsage>,
    #[serde(default)]
    pub active_stage_id: Option<String>,
    #[serde(default)]
    pub active_stage_count: u32,
    #[serde(default)]
    pub active_tools: Vec<ActiveToolSummary>,
    #[serde(default)]
    pub pending_question: Option<PendingQuestionSummary>,
    #[serde(default)]
    pub pending_permission: Option<PendingPermissionSummary>,
    #[serde(default)]
    pub attached_sessions: Vec<AttachedSessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRunStatusKind {
    Idle,
    Running,
    Compacting,
    WaitingOnTool,
    WaitingOnUser,
    Cancelling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveToolSummary {
    pub tool_call_id: String,
    pub tool_name: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestionSummary {
    pub request_id: String,
    pub questions: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPermissionSummary {
    pub permission_id: String,
    pub info: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedSessionSummary {
    pub attached_id: String,
    pub parent_id: String,
    #[serde(default)]
    pub context_kind: Option<SessionContextKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPreflightSeverity {
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPreflightStatus {
    Ready,
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionPreflightIssue {
    pub severity: ExecutionPreflightSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionExecutionPreflightSource {
    ToolCallState,
    ToolResultPart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExecutionPreflightSummary {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub source: SessionExecutionPreflightSource,
    pub runner: String,
    pub subject: String,
    pub status: ExecutionPreflightStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ExecutionPreflightIssue>,
    #[serde(default)]
    pub attachment_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDiagnosticSeverity {
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDiagnosticSource {
    RequestValidation,
    ApiErrorRewrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDiagnosticSummary {
    pub severity: ProviderDiagnosticSeverity,
    pub source: ProviderDiagnosticSource,
    pub code: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTelemetrySnapshot {
    pub runtime: SessionRuntimeState,
    #[serde(default)]
    pub stages: Vec<StageSummary>,
    pub topology: SessionExecutionTopology,
    pub usage: SessionUsage,
    pub usage_books: SessionUsageBooks,
    #[serde(default)]
    pub memory: Option<SessionMemoryTelemetrySummary>,
    #[serde(default)]
    pub cache_evidence: Option<serde_json::Value>,
    #[serde(default)]
    pub context_explain: Option<SessionContextExplain>,
    #[serde(default)]
    pub ownership: Option<SessionOwnershipSummary>,
    #[serde(default)]
    pub context_compaction_summary: Option<ContextCompactionSummary>,
    #[serde(default)]
    pub context_compaction_lifecycle_summary: Option<ContextCompactionLifecycleSummary>,
    #[serde(default)]
    pub context_pressure_governance_summary: Option<ContextPressureGovernanceSummary>,
    #[serde(default)]
    pub cache_semantics: Option<SessionCacheSemanticsSummary>,
    #[serde(default)]
    pub context_closure_contract: Option<SessionContextClosureContract>,
    #[serde(default)]
    pub prompt_surface_evidence: Option<PromptSurfaceEvidenceSummary>,
    #[serde(default)]
    pub ingress_stabilization: Option<serde_json::Value>,
    #[serde(default)]
    pub execution_preflight_summary: Option<SessionExecutionPreflightSummary>,
    #[serde(default)]
    pub provider_diagnostic_summary: Option<ProviderDiagnosticSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEventsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryProtocolStatus {
    Running,
    AwaitingUser,
    Recoverable,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryActionKind {
    AbortRun,
    AbortStage,
    Retry,
    Resume,
    PartialReplay,
    RestartStage,
    RestartSubtask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryCheckpointInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub scheduler_profile: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub stage_index: Option<u32>,
    #[serde(default)]
    pub stage_total: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryActionInfo {
    pub kind: RecoveryActionKind,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub target_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecoveryProtocol {
    #[serde(alias = "sessionID", alias = "sessionId")]
    pub session_id: String,
    pub status: RecoveryProtocolStatus,
    pub active_execution_count: usize,
    pub pending_question_count: usize,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub last_user_prompt: Option<String>,
    #[serde(default)]
    pub actions: Vec<RecoveryActionInfo>,
    #[serde(default)]
    pub checkpoints: Vec<RecoveryCheckpointInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRecoveryRequest {
    pub action: RecoveryActionKind,
    #[serde(default)]
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOptionInfo {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionItemInfo {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOptionInfo>,
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionInfo {
    pub id: String,
    #[serde(alias = "sessionID", alias = "sessionId")]
    pub session_id: String,
    pub questions: Vec<String>,
    #[serde(default)]
    pub options: Option<Vec<Vec<String>>>,
    #[serde(default)]
    pub items: Vec<QuestionItemInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestInfo {
    pub id: String,
    #[serde(alias = "sessionID", alias = "sessionId")]
    pub session_id: String,
    pub tool: String,
    #[serde(default)]
    pub permission_class: Option<String>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub origin_tool: Option<String>,
    #[serde(default)]
    pub supported_lifetimes: Vec<String>,
    pub input: serde_json::Value,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    pub id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
    pub file: Option<FileInfo>,
    #[serde(alias = "toolCall")]
    pub tool_call: Option<ToolCall>,
    #[serde(alias = "toolResult")]
    pub tool_result: Option<ToolResult>,
    #[serde(default)]
    pub synthetic: Option<bool>,
    #[serde(default)]
    pub ignored: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub url: String,
    pub filename: String,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub state: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(alias = "toolCallId")]
    pub tool_call_id: String,
    pub content: String,
    #[serde(alias = "isError")]
    pub is_error: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    #[serde(alias = "sessionId")]
    pub session_id: String,
    pub role: String,
    pub created_at: i64,
    #[serde(default, alias = "completedAt")]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tokens: MessageTokensInfo,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub multimodal: Option<rocode_multimodal::PersistedMultimodalExplain>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageTokensInfo {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default, alias = "cacheRead")]
    pub cache_read: u64,
    #[serde(default, alias = "cacheMiss")]
    pub cache_miss: u64,
    #[serde(default, alias = "cacheWrite")]
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PromptPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_source: Option<String>,
    pub agent: Option<String>,
    pub scheduler_profile: Option<String>,
    pub model: Option<String>,
    pub variant: Option<String>,
    pub command: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteShellRequest {
    pub command: String,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub scheduler_profile: Option<String>,
    pub directory: Option<String>,
    pub project_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionExternalAdapterSessionRequest {
    pub adapter_id: String,
    pub actor_id: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub route_policy_id: Option<String>,
    #[serde(default)]
    pub scheduler_profile: Option<String>,
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionExternalAdapterSessionResponse {
    pub adapter: String,
    pub source: ExternalAdapterSource,
    pub binding: ExternalAdapterResolvedBinding,
    pub session: SessionInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderListResponse {
    pub providers: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullProviderListResponse {
    pub all: Vec<ProviderInfo>,
    #[serde(rename = "default")]
    pub default_model: HashMap<String, String>,
    #[serde(default)]
    pub connected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownProviderEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub model_count: usize,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub supports_api_key_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownProvidersResponse {
    pub providers: Vec<KnownProviderEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectProtocolOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnectSchemaResponse {
    pub providers: Vec<KnownProviderEntry>,
    #[serde(default)]
    pub protocols: Vec<ConnectProtocolOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptorResponse {
    pub provider_id: String,
    #[serde(default)]
    pub descriptor_candidate: Option<ProviderConnectionDescriptorCandidate>,
    #[serde(default)]
    pub descriptor_candidate_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConnectDraftMode {
    Known,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnectDraft {
    pub mode: ProviderConnectDraftMode,
    pub provider_id: String,
    #[serde(default)]
    pub known_provider_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub model_count: usize,
    #[serde(default)]
    pub supports_api_key_connect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveProviderConnectRequest {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveProviderConnectResponse {
    pub query: String,
    pub suggested_mode: ProviderConnectDraftMode,
    pub exact_match: bool,
    #[serde(default)]
    pub matches: Vec<KnownProviderEntry>,
    pub draft: ProviderConnectDraft,
    pub custom_draft: ProviderConnectDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCatalogRefreshStatus {
    Updated,
    NotModified,
    FallbackCached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshProviderCatalogResponse {
    pub generation_before: u64,
    pub generation_after: u64,
    pub changed: bool,
    pub status: ProviderCatalogRefreshStatus,
    #[serde(default)]
    pub error_message: Option<String>,
}

impl RefreshProviderCatalogResponse {
    pub fn status_message(&self) -> String {
        match self.status {
            ProviderCatalogRefreshStatus::Updated => format!(
                "Model catalogue refreshed (generation {} -> {}).",
                self.generation_before, self.generation_after
            ),
            ProviderCatalogRefreshStatus::NotModified => format!(
                "Model catalogue checked; no changes (generation {}).",
                self.generation_after
            ),
            ProviderCatalogRefreshStatus::FallbackCached => format!(
                "Model catalogue refresh failed; using cached snapshot: {}",
                self.error_message
                    .as_deref()
                    .unwrap_or("Unknown refresh failure")
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectProviderRequest {
    pub provider_id: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ProviderModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(
        default,
        alias = "context_window",
        alias = "contextWindow",
        alias = "contextLength"
    )]
    pub context_window: Option<u64>,
    #[serde(default, alias = "max_output_tokens", alias = "maxOutputTokens")]
    pub max_output_tokens: Option<u64>,
    #[serde(
        default,
        alias = "cost_per_million_input",
        alias = "costPerMillionInput"
    )]
    pub cost_per_million_input: Option<f64>,
    #[serde(
        default,
        alias = "cost_per_million_output",
        alias = "costPerMillionOutput"
    )]
    pub cost_per_million_output: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionModeInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub orchestrator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStatusInfo {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthStartInfo {
    pub authorization_url: String,
    pub client_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareResponse {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactResponse {
    pub success: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ContextCompactionLifecycleSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<ContextCompactionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactRequest {
    #[serde(default)]
    pub focus: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertRequest {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertResponse {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiTodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDiffEntry {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}
