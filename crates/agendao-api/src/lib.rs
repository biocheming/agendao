use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBlueprintView {
    pub blueprint: agendao_orchestrator::blueprint::SchedulerBlueprint,
    pub generated_agents: Vec<agendao_orchestrator::selector::GeneratedAgentSpec>,
    pub fingerprint: String,
    pub selection_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionBlueprintRequest {
    pub blueprint: agendao_orchestrator::blueprint::SchedulerBlueprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectSessionBlueprintResponse {
    pub rejected_fingerprint: String,
}

pub use agendao_multimodal::{
    ModalityKind, ModalityPreflightResult, MultimodalCapabilitiesResponse,
    MultimodalPolicyResponse, MultimodalPreflightRequest, MultimodalPreflightResponse,
    PreflightCapabilityView, PreflightInputPart,
};
pub use agendao_types::{
    ConfigPolicyValidationEffect, ConfigPolicyValidationItem, ConfigPolicyValidationOwner,
    ConfigPolicyValidationScope, ConfigPolicyValidationScopeKind, ConfigPolicyValidationSeverity,
    ConfigPolicyValidationSnapshot, ContextCompactionLifecycleSummary, ContextCompactionSummary,
    ContextPressureGovernanceSummary, ExternalAdapterResolvedBinding, ExternalAdapterSource,
    ManagedSkillRecord, MemoryConflictResponse, MemoryConsolidationRequest,
    MemoryConsolidationResponse, MemoryConsolidationRunListResponse, MemoryConsolidationRunQuery,
    MemoryDetailView, MemoryListQuery, MemoryListResponse, MemoryRetrievalPreviewResponse,
    MemoryRetrievalQuery, MemoryRuleHitListResponse, MemoryRuleHitQuery,
    MemoryRulePackListResponse, MemoryScope, MemoryValidationReportResponse,
    ModelRepairQuerySummary, ModelToolRepairTelemetrySummary, PermissionRulesetInfo, PromptPart,
    PromptSurfaceEvidenceSummary, ProposalStatus, ProviderConnectionDescriptorCandidate,
    ProviderProfileDescriptorView, RepairAggregateRow, RepairKind, RepairOutcomeKind, RepairQuery,
    RepairQueryResponse, RepairSample, SessionCacheSemanticsSummary,
    SessionCompactionContinuityInspection, SessionContextClosureContract, SessionContextExplain,
    SessionContextKind, SessionEffectiveCompactionPolicy, SessionEffectiveExternalAdapterPolicy,
    SessionEffectiveMemoryPolicy, SessionEffectivePolicyView, SessionEffectiveProviderPolicy,
    SessionEffectiveProviderRuntimeProfile, SessionEffectiveSchedulerPolicy, SessionForkExplain,
    SessionForkHistoryMode, SessionInfo, SessionInsightsResponse, SessionListContract,
    SessionListHints, SessionListItem, SessionListResponse, SessionMemoryTelemetrySummary,
    SessionOwnershipSummary, SessionPermissionMode, SessionRepairQuerySnapshot,
    SessionRepairQuerySummary, SessionRevertInfo, SessionShareInfo, SessionSummaryInfo,
    SessionTimeInfo, SessionToolRepairTelemetrySummary, SessionUsage, SessionUsageBooks,
    SkillArtifactCacheEntry, SkillAuditEvent, SkillDistributionRecord, SkillEvolutionProposal,
    SkillEvolutionProposalKind, SkillGovernanceDiagnosticSeverity, SkillGovernanceTimelineEntry,
    SkillGovernanceTimelineStatus, SkillGovernanceWriteResult, SkillGuardReport, SkillGuardStatus,
    SkillHubArtifactCacheResponse, SkillHubAuditResponse, SkillHubDistributionResponse,
    SkillHubGuardRunRequest, SkillHubGuardRunResponse, SkillHubIndexRefreshRequest,
    SkillHubIndexRefreshResponse, SkillHubIndexResponse, SkillHubLifecycleResponse,
    SkillHubManagedDetachRequest, SkillHubManagedDetachResponse, SkillHubManagedRemoveRequest,
    SkillHubManagedRemoveResponse, SkillHubManagedResponse, SkillHubNegativeEntropyResponse,
    SkillHubPolicy, SkillHubPolicyResponse, SkillHubRemoteInstallApplyRequest,
    SkillHubRemoteInstallPlanRequest, SkillHubRemoteUpdateApplyRequest,
    SkillHubRemoteUpdatePlanRequest, SkillHubReviewCandidatesSyncRequest,
    SkillHubReviewCandidatesSyncResponse, SkillHubSemanticConflictResponse,
    SkillHubSyncApplyRequest, SkillHubSyncPlanRequest, SkillHubSyncPlanResponse,
    SkillHubTimelineQuery, SkillHubTimelineResponse, SkillHubUsageLedgerResponse,
    SkillHubVitalityUpdateRequest, SkillHubVitalityUpdateResponse, SkillManagedLifecycleRecord,
    SkillNegativeEntropyDiagnostic, SkillNegativeEntropySignal, SkillOperationalSnapshot,
    SkillOperationalSourceScope, SkillRemoteInstallAction, SkillRemoteInstallEntry,
    SkillRemoteInstallPlan, SkillRemoteInstallResponse, SkillRetirementReason,
    SkillRetirementReasonKind, SkillSemanticConflictDiagnostic, SkillSemanticConflictKind,
    SkillSourceIndexSnapshot, SkillSourceKind, SkillSourceRef, SkillSyncPlan,
    SkillUsageLedgerEntry, SkillVitalityRecord, SkillVitalityState, SkillWriteLedgerAction,
    SkillWriteLedgerEntry, ToolResultGovernanceSummary, ToolTrajectoryQualitySummary,
};

pub type SessionListTime = agendao_types::SessionTime;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStatusInfo {
    pub status: String,
    #[serde(default)]
    pub idle: bool,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub next: Option<i64>,
}

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
    /// True when the skill matches `skills.disabled` (exact name or
    /// `category/*` wildcard). Only populated when the query sets
    /// `include_disabled`; runtime-facing catalog reads keep filtering
    /// disabled skills out entirely.
    #[serde(default)]
    pub disabled: bool,
}

/// Settings→Tools 列表行（GET `/tool/catalog` / `local_list_tools`）。
/// 与 skill catalog 不同：disabled tools 仍列出（`disabled` 标记），
/// 否则 UI 无法提供 re-enable 入口。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolListEntry {
    pub id: String,
    pub description: String,
    /// Catalog metadata family（`family/*` 通配禁用的类目 key）；无 metadata
    /// 的 tool 为 `None`，只能按精确名禁用。
    #[serde(default)]
    pub family: Option<String>,
    /// Facade/bridge 工具（`tool_catalog_*`/`skills_*`/`skill`/`skill_view`
    /// 及 legacy 别名）——禁用它们会切断模型对其它一切工具的触达，
    /// registry 过滤对它们豁免，UI 侧开关锁定。
    #[serde(default)]
    pub protected: bool,
    /// True when the tool matches `disabled_tools`（精确 id 或 `family/*`）。
    #[serde(default)]
    pub disabled: bool,
}

/// PUT `/config/disabled` 请求体：`Some(vec)` = 整体替换对应 disabled 列表
/// （允许空 vec 清空——`PATCH /config` 的 merge 语义无法表达清空）；
/// `None`/缺省 = 不动该列表。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisabledConfigUpdate {
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// 顶层 `disabled_plugins`（精确名或 `前缀/*` 通配）。
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
}

/// Settings→Plugins 列表行（GET `/config/plugins` / `local_list_plugins`）。
/// 数据源 = config.plugin（managed）+ discovery 目录扫描（discovered）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginListEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub plugin_type: String,
    /// `"managed"`（config 声明）/ `"discovered"`（插件根目录扫描）。
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 安装途径标签（config 声明 / user (~/.agendao/plugins) /
    /// project (.agendao/plugins) / external (config plugin_paths)）。
    pub origin: String,
    /// True when the plugin matches `disabled_plugins`（精确名或 `前缀/*`）。
    #[serde(default)]
    pub disabled: bool,
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
    pub vitality_state: agendao_types::SkillVitalityState,
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
    pub methodology: Option<agendao_skill::SkillMethodologyTemplate>,
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
    pub category: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub toolsets: Vec<String>,
    /// Include skills matched by `skills.disabled` in the response (flagged
    /// via `SkillCatalogEntry.disabled`) instead of filtering them out.
    /// Inspection/Settings surface only; runtime resolution keeps filtering.
    #[serde(default)]
    pub include_disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillDetailQuery {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
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
    pub queued_count: Option<u64>,
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
    #[serde(rename = "scheduler", default)]
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    #[serde(rename = "questionId", default)]
    pub question_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    PromptRun,
    SchedulerRun,
    SchedulerNode,
    ToolCall,
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
#[serde(deny_unknown_fields)]
pub struct SessionExecutionTopology {
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
    pub pending_followup_count: u64,
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
    Blocked,
    Sleeping,
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
    pub requested_at: i64,
    #[serde(default)]
    pub tool: Option<String>,
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
    pub topology: SessionExecutionTopology,
    pub usage: SessionUsage,
    pub usage_books: SessionUsageBooks,
    #[serde(default)]
    pub tool_repair_summary: Option<SessionToolRepairTelemetrySummary>,
    #[serde(default)]
    pub model_tool_repair_summary: Option<ModelToolRepairTelemetrySummary>,
    #[serde(default)]
    pub repair_query_snapshot: Option<SessionRepairQuerySnapshot>,
    #[serde(default)]
    pub tool_trajectory_quality: Option<ToolTrajectoryQualitySummary>,
    #[serde(default)]
    pub tool_result_governance: Option<ToolResultGovernanceSummary>,
    #[serde(default)]
    pub pending_permission_count: u64,
    #[serde(default)]
    pub granted_by_turn_count: u64,
    #[serde(default)]
    pub granted_by_session_count: u64,
    #[serde(default)]
    pub granted_by_matcher_kind: BTreeMap<String, u64>,
    #[serde(default)]
    pub last_permission_matcher_kind: Option<String>,
    #[serde(default)]
    pub last_permission_grant_target: Option<String>,
    #[serde(default)]
    pub last_permission_miss_count: u64,
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
    pub compaction_continuity: Option<SessionCompactionContinuityInspection>,
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
    #[serde(default)]
    pub runtime_protocol: Option<SessionRuntimeProtocolSummary>,
    #[serde(default)]
    pub event_bus_telemetry: Option<EventBusTelemetrySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventBusTelemetrySummary {
    pub send_count: u64,
    pub send_error_count: u64,
    pub max_receivers: u64,
    pub last_send_at_ms: u64,
    pub last_send_error_at_ms: u64,
    /// LiveSnapshotCoalescer: number of deltas accumulated into snapshots.
    #[serde(default)]
    pub coalesced_snapshot_count: u64,
    /// Output blocks received without live_identity (legacy passthrough).
    #[serde(default)]
    pub identity_missing_count: u64,
    /// Coalesced full snapshots emitted to frontends.
    #[serde(default)]
    pub full_snapshot_emitted_count: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptIngressDisposition {
    AcceptNow,
    QueueAsSteering,
    BlockedOnQuestion,
    BlockedOnPermission,
    AwaitingInterrupt,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptPhase {
    #[default]
    Idle,
    Requested,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptTarget {
    Run,
    Stage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRuntimeSummary {
    pub pending: bool,
    #[serde(default)]
    pub pending_permission_id: Option<String>,
    #[serde(default)]
    pub pending_since_ms: Option<i64>,
    #[serde(default)]
    pub pending_tool: Option<String>,
    #[serde(default)]
    pub last_pending_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteeringRuntimeSummary {
    pub pending_count: u64,
    #[serde(default)]
    pub last_enqueued_at_ms: Option<i64>,
    #[serde(default)]
    pub last_consumed_at_ms: Option<i64>,
    #[serde(default)]
    pub last_source_session_id: Option<String>,
    #[serde(default)]
    pub last_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptRuntimeSummary {
    pub phase: InterruptPhase,
    #[serde(default)]
    pub requested_at_ms: Option<i64>,
    #[serde(default)]
    pub target: Option<InterruptTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRuntimeProtocolSummary {
    pub prompt_ingress: PromptIngressDisposition,
    pub permission: PermissionRuntimeSummary,
    pub steering: SteeringRuntimeSummary,
    pub interrupt: InterruptRuntimeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRepairSummaryResponse {
    pub session_id: String,
    #[serde(default)]
    pub snapshot: Option<SessionRepairQuerySnapshot>,
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
    Retry,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryActionInfo {
    pub kind: RecoveryActionKind,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecoveryProtocol {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRecoveryRequest {
    pub action: RecoveryActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionOptionInfo {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct QuestionInfo {
    pub id: String,
    pub session_id: String,
    pub items: Vec<QuestionItemInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequestInfo {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    #[serde(default)]
    pub permission_class: Option<String>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub scope_label: Option<String>,
    #[serde(default)]
    pub origin_tool: Option<String>,
    #[serde(default)]
    pub supported_lifetimes: Vec<String>,
    #[serde(default)]
    pub matcher_kind: Option<String>,
    #[serde(default)]
    pub matcher_key: Option<String>,
    #[serde(default)]
    pub matcher_label: Option<String>,
    #[serde(default)]
    pub grant_target_summary: Option<String>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    pub input: serde_json::Value,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagePart {
    pub id: String,
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
    pub file: Option<FileInfo>,
    pub tool_call: Option<ToolCall>,
    pub tool_result: Option<ToolResult>,
    #[serde(default)]
    pub output_block: Option<serde_json::Value>,
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
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub attachments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInfo {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub created_at: i64,
    #[serde(default)]
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
    pub multimodal: Option<agendao_multimodal::PersistedMultimodalExplain>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageTokensInfo {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub context: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_miss: u64,
    #[serde(default)]
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Prompt request from a client (CLI, TUI, Web, API).
///
/// # Command/input authority (AgenDao 木律, P2.3)
///
/// `message` and `command` coexist in this struct:
/// - `message`: pre-formatted text (may contain `/command args` from CLI)
/// - `command`: structured command hint for diagnostics/routing
///
/// **Precedence**: `message` takes precedence for model-visible text.
/// The session ingress layer is the canonical authority.
///
/// **Transport coverage**:
/// - Direct (in-process): both fields reach the orchestrator.
/// - HTTP / Unix: `message` and `command` both arrive when the client
///   transport provides them.
///
/// **Future**: make the session ingress layer own command→message
/// concatenation so adapters no longer need to pre-format `/command args`.
pub struct PromptRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PromptPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_origin: Option<agendao_types::MessageSourceOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_surface: Option<agendao_types::MessageSourceSurface>,
    pub agent: Option<String>,
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
    pub model: Option<String>,
    pub variant: Option<String>,
    /// Structured command hint (e.g. "run", "tui").  For diagnostics and
    /// routing; not directly part of the model-visible text.
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
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
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
    pub scheduler: Option<agendao_orchestrator::selector::SchedulerChoice>,
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
pub struct UpdateSessionPermissionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SessionPermissionMode>,
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
#[serde(deny_unknown_fields)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub models: Vec<ProviderModelInfo>,
    /// Provider HTTP endpoint(用户 .agendao/providers.toml 或全局 catalog 配置)。
    /// 阴面记账(土律):server 端唯一权威,TUI/web 只读消费;
    /// `None` = 该 provider 未配 base_url(rare,通常意味是 SDK-managed)。
    #[serde(default)]
    pub base_url: Option<String>,
    /// Wire protocol: OpenAI Responses, OpenAI Chat Completions, or Anthropic Messages.
    /// The server resolves a complete configured profile first and accepts only the three
    /// runtime-supported catalog SDK shapes as a display fallback.
    /// `None` means the protocol is absent or unsupported.
    #[serde(default)]
    pub protocol: Option<String>,
    /// 是否被用户禁用(`config.disabled_providers` 成员)。
    #[serde(default)]
    pub disabled: bool,
}

/// `POST /provider/{id}/test` 的响应：连接测试结果（只读探测）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestProviderConnectionResponse {
    pub ok: bool,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub latency_ms: u128,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    /// Whether this exact provider/model pair is registered in the live
    /// runtime and can be used for a prompt. `None` preserves compatibility
    /// with older servers that only exposed provider-level connectivity.
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub cost_per_million_input: Option<f64>,
    #[serde(default)]
    pub cost_per_million_output: Option<f64>,
    /// Provider-specific capability detail projected by the server. Kept as
    /// structured JSON so clients remain forward-compatible as the catalog
    /// adds capability fields.
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
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

// ── P3-A: Live Identity Contract ─────────────────────────────────────────
// Defined in agendao-types; re-exported here for frontend consumption.

pub use agendao_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};

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

// ── Canonical Runtime Event Surface (P1-1) ─────────────────────────────────
// Authority definition shared with agendao-server::session_runtime::events.
// Frontends (CLI/TUI/Web) reference this enum to negotiate subscription tiers
// and decide how to consume each event kind.

/// Authority enum for every event that crosses the server→frontend boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalEventKind {
    MessageDelta,
    MessageCompleted,
    ToolCallStarted,
    ToolCallDelta,
    ToolCallCompleted,
    PermissionPending,
    PermissionResolved,
    SteeringQueued,
    SteeringConsumed,
    RuntimeStatusChanged,
    SessionReconcile,
}

impl CanonicalEventKind {
    pub fn is_high_frequency(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    pub fn is_mergeable(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    pub fn is_droppable(self) -> bool {
        matches!(self, Self::MessageDelta | Self::ToolCallDelta)
    }

    pub fn is_must_deliver(self) -> bool {
        !self.is_droppable()
    }
}

// ── Frontend Subscription Capability Model (P2-1) ────────────────────────
// Authority for per-frontend event subscription negotiation.
// Defined once in agendao-api; no frontend may hardcode its own copy of these
// defaults or capability flags.

/// Per-frontend subscription capabilities.
///
/// Each field corresponds to one category of server→frontend event traffic.
/// A `false` value means the frontend does NOT need this category; the server
/// may skip or coalesce corresponding events for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct FrontendSubscriptionCapabilities {
    /// Streaming text deltas for assistant messages.
    pub message_text_delta: bool,
    /// Streaming output / progress from running tools.
    pub tool_progress: bool,
    /// Reasoning / chain-of-thought deltas.
    pub reasoning_delta: bool,
    /// High-frequency runtime telemetry live view (topology and counters).
    pub runtime_live_view: bool,
    /// Final-only mode: only non-droppable events (message completed,
    /// tool completed, permission resolved, runtime status, reconcile).
    /// When true, the server skips all delta events regardless of other flags.
    pub final_only: bool,
}

impl Default for FrontendSubscriptionCapabilities {
    fn default() -> Self {
        FrontendSubscriptionTier::TuiHighFrequency.default_capabilities()
    }
}

/// Subscription tier: a named capability bundle.
///
/// Three tiers are defined. Each frontend picks the tier that matches its
/// rendering model and user experience requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendSubscriptionTier {
    /// TUI: high-frequency incremental rendering with live view.
    TuiHighFrequency,
    /// Web: medium-frequency, local coalescing of tool progress.
    WebMediumFrequency,
    /// CLI: low-frequency summary with must-deliver events only.
    CliLowFrequency,
}

impl FrontendSubscriptionTier {
    /// Canonical default capabilities for each tier.
    /// These are the single authority — frontends reference these, they do not
    /// define their own copies.
    pub fn default_capabilities(self) -> FrontendSubscriptionCapabilities {
        match self {
            Self::TuiHighFrequency => FrontendSubscriptionCapabilities {
                message_text_delta: true,
                tool_progress: true,
                reasoning_delta: true,
                runtime_live_view: true,
                final_only: false,
            },
            Self::WebMediumFrequency => FrontendSubscriptionCapabilities {
                message_text_delta: true,
                tool_progress: true,
                reasoning_delta: false,
                runtime_live_view: true,
                final_only: false,
            },
            Self::CliLowFrequency => FrontendSubscriptionCapabilities {
                message_text_delta: false,
                tool_progress: false,
                reasoning_delta: false,
                runtime_live_view: false,
                final_only: true,
            },
        }
    }
}

/// Resolved subscription for one explicit frontend tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFrontendSubscription {
    pub capabilities: FrontendSubscriptionCapabilities,
    pub tier: FrontendSubscriptionTier,
}

impl ResolvedFrontendSubscription {
    /// Canonical constructor for internal callers that already own a typed tier.
    pub fn from_tier(tier: FrontendSubscriptionTier) -> Self {
        Self {
            capabilities: tier.default_capabilities(),
            tier,
        }
    }

    /// Strict wire-format entry point. Missing and unknown tiers are invalid.
    pub fn from_wire_tier(wire: Option<&str>) -> Result<Self, String> {
        match wire {
            Some("tui") => Ok(Self::from_tier(FrontendSubscriptionTier::TuiHighFrequency)),
            Some("web") => Ok(Self::from_tier(
                FrontendSubscriptionTier::WebMediumFrequency,
            )),
            Some("cli") => Ok(Self::from_tier(FrontendSubscriptionTier::CliLowFrequency)),
            Some(other) => Err(format!(
                "unknown subscription tier `{other}`; expected `tui`, `web`, or `cli`"
            )),
            None => Err("missing required subscription tier".to_string()),
        }
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;

    #[test]
    fn default_caps_match_tui_tier() {
        let caps = FrontendSubscriptionCapabilities::default();
        assert!(caps.message_text_delta);
        assert!(caps.tool_progress);
        assert!(!caps.final_only);
    }

    #[test]
    fn cli_tier_is_final_only_and_skips_deltas() {
        let caps = FrontendSubscriptionTier::CliLowFrequency.default_capabilities();
        assert!(!caps.message_text_delta);
        assert!(!caps.tool_progress);
        assert!(!caps.reasoning_delta);
        assert!(caps.final_only);
    }

    #[test]
    fn tui_tier_gets_full_capabilities() {
        let caps = FrontendSubscriptionTier::TuiHighFrequency.default_capabilities();
        assert!(caps.message_text_delta);
        assert!(caps.tool_progress);
        assert!(caps.reasoning_delta);
        assert!(!caps.final_only);
    }

    #[test]
    fn web_tier_excludes_reasoning_delta() {
        let caps = FrontendSubscriptionTier::WebMediumFrequency.default_capabilities();
        assert!(caps.message_text_delta);
        assert!(!caps.reasoning_delta);
        assert!(!caps.final_only);
    }

    #[test]
    fn resolved_from_tier_cli_is_final_only() {
        let sub =
            ResolvedFrontendSubscription::from_tier(FrontendSubscriptionTier::CliLowFrequency);
        assert!(sub.capabilities.final_only);
    }

    #[test]
    fn from_wire_tier_is_strict_wire_parsing_authority() {
        let sub = ResolvedFrontendSubscription::from_wire_tier(Some("cli")).unwrap();
        assert!(sub.capabilities.final_only);

        let sub = ResolvedFrontendSubscription::from_wire_tier(Some("web")).unwrap();
        assert!(!sub.capabilities.reasoning_delta);

        let sub = ResolvedFrontendSubscription::from_wire_tier(Some("tui")).unwrap();
        assert!(sub.capabilities.reasoning_delta);

        assert!(ResolvedFrontendSubscription::from_wire_tier(None).is_err());
        assert!(ResolvedFrontendSubscription::from_wire_tier(Some("unknown")).is_err());
    }

    #[test]
    fn subscription_capabilities_roundtrip_via_json() {
        let caps = FrontendSubscriptionTier::WebMediumFrequency.default_capabilities();
        let json = serde_json::to_value(caps).expect("serialize");
        let parsed: FrontendSubscriptionCapabilities =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed, caps);
    }

    #[test]
    fn resolved_subscription_roundtrip_via_json() {
        let sub =
            ResolvedFrontendSubscription::from_tier(FrontendSubscriptionTier::TuiHighFrequency);
        let json = serde_json::to_value(&sub).expect("serialize");
        let parsed: ResolvedFrontendSubscription =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed.capabilities, sub.capabilities);
        assert_eq!(parsed.tier, sub.tier);
    }
}

#[cfg(test)]
mod canonical_wire_tests {
    use super::*;

    #[test]
    fn message_info_accepts_server_context_token_count() {
        let message = serde_json::json!({
            "id": "message-1",
            "session_id": "session-1",
            "role": "assistant",
            "created_at": 1,
            "tokens": {
                "input": 10,
                "context": 4096,
                "output": 20,
                "reasoning": 0,
                "cache_read": 128,
                "cache_miss": 0,
                "cache_write": 0
            }
        });

        let parsed = serde_json::from_value::<MessageInfo>(message)
            .expect("server message payload should match the public API type");
        assert_eq!(parsed.tokens.context, 4096);
    }

    #[test]
    fn session_and_message_types_reject_removed_camel_case_fields() {
        let topology = serde_json::json!({
            "session_id": "session-1",
            "sessionID": "legacy",
            "active_count": 0,
            "running_count": 0,
            "waiting_count": 0,
            "cancelling_count": 0,
            "retry_count": 0
        });
        assert!(serde_json::from_value::<SessionExecutionTopology>(topology).is_err());

        let message = serde_json::json!({
            "id": "message-1",
            "session_id": "session-1",
            "sessionId": "legacy",
            "role": "assistant",
            "created_at": 1
        });
        assert!(serde_json::from_value::<MessageInfo>(message).is_err());

        let tokens = serde_json::json!({"cacheRead": 1});
        assert!(serde_json::from_value::<MessageTokensInfo>(tokens).is_err());
    }

    #[test]
    fn provider_types_reject_removed_camel_case_fields() {
        let provider = serde_json::json!({
            "id": "openai",
            "name": "OpenAI",
            "models": [],
            "baseUrl": "https://example.com"
        });
        assert!(serde_json::from_value::<ProviderInfo>(provider).is_err());

        let model = serde_json::json!({
            "id": "gpt-5",
            "name": "GPT-5",
            "provider": "openai",
            "contextWindow": 128000
        });
        assert!(serde_json::from_value::<ProviderModelInfo>(model).is_err());
    }
}
