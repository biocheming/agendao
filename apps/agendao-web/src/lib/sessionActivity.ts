import type {
  MemoryCardRecord,
  MemoryRetrievalPacketRecord,
  SessionMemoryTelemetryRecord,
} from "./memory";
import type { SessionMultimodalInsight } from "./multimodal";
import type {
  PersistedSessionTelemetrySnapshot,
  ToolResultGovernanceRecord,
  ToolTrajectoryQualityRecord,
} from "./session";

export interface ExecutionNodeRecord {
  id: string;
  kind: string;
  status: string;
  label?: string;
  parent_id?: string;
  stage_id?: string;
  waiting_on?: string;
  recent_event?: string;
  started_at?: number;
  updated_at?: number;
  metadata?: Record<string, unknown> | null;
  children?: ExecutionNodeRecord[];
}

export interface SessionExecutionTopologyRecord {
  active_count: number;
  running_count: number;
  waiting_count: number;
  cancelling_count?: number;
  retry_count?: number;
  done_count: number;
  updated_at?: number | null;
  roots: ExecutionNodeRecord[];
}

export interface SessionUsageRecord {
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_write_tokens: number;
  cache_read_tokens: number;
  cache_miss_tokens: number;
  context_tokens?: number;
  total_cost: number;
}

export interface ToolRepairCountRecord {
  key: string;
  count: number;
}

export interface ToolRepairToolSummaryRecord {
  tool_name: string;
  call_count: number;
  repaired_call_count: number;
  error_call_count: number;
  repair_event_count: number;
  event_kinds: ToolRepairCountRecord[];
}

export interface SessionToolRepairTelemetrySummaryRecord {
  total_tool_calls: number;
  repaired_tool_call_count: number;
  error_tool_call_count: number;
  repair_event_count: number;
  event_kinds: ToolRepairCountRecord[];
  event_layers: ToolRepairCountRecord[];
  tools: ToolRepairToolSummaryRecord[];
}

export interface ModelToolRepairTelemetrySummaryRecord
  extends SessionToolRepairTelemetrySummaryRecord {
  provider_id: string;
  model_id: string;
  session_count: number;
  repaired_session_count: number;
}

export interface SessionInsightsMemoryRecord {
  summary: SessionMemoryTelemetryRecord;
  frozen_snapshot?: MemoryRetrievalPacketRecord | null;
  last_prefetch_packet?: MemoryRetrievalPacketRecord | null;
  recent_session_records: MemoryCardRecord[];
}

export interface SessionEffectiveSchedulerPolicyRecord {
  requested_kind?: string | null;
  blueprint_name?: string | null;
  blueprint_fingerprint?: string | null;
  source: string;
  applied: boolean;
  resolved_agent?: string | null;
}

export interface SessionEffectivePolicyViewRecord {
  session_id: string;
  scheduler?: SessionEffectiveSchedulerPolicyRecord | null;
  warnings: string[];
}

export interface SessionInsightsRecord {
  id: string;
  title: string;
  directory: string;
  updated: number;
  telemetry?: PersistedSessionTelemetrySnapshot | null;
  effective_policy?: SessionEffectivePolicyViewRecord | null;
  memory?: SessionInsightsMemoryRecord | null;
  multimodal?: SessionMultimodalInsight | null;
}

export interface SessionRuntimeRecord {
  session_id: string;
  run_status: string;
  current_message_id?: string | null;
  usage?: SessionUsageRecord | null;
  active_stage_id?: string | null;
  active_stage_count?: number;
}

export interface SessionPrefixStabilityContractRecord {
  basis: string;
  tracked_on_api_view: boolean;
  api_view_messages: number;
  trimmed_model_visible_messages: number;
  prefix_change_detected: boolean;
  explanation?: string | null;
}

export interface SessionCompactionBoundaryContractRecord {
  boundary_recorded: boolean;
  phase?: string | null;
  trigger?: string | null;
  reason?: string | null;
  governance_status?: string | null;
  request_pressure_percent?: number | null;
  live_pressure_percent?: number | null;
  compaction_attempted: boolean;
  compaction_succeeded: boolean;
  blocking: boolean;
}

export interface SessionCacheExplainabilityContractRecord {
  issue_present: boolean;
  explained: boolean;
  source: string;
  severity?: string | null;
  explanation?: string | null;
}

export interface SessionContextClosureContractRecord {
  prefix_stability: SessionPrefixStabilityContractRecord;
  compaction_boundary: SessionCompactionBoundaryContractRecord;
  cache_explainability: SessionCacheExplainabilityContractRecord;
}

export interface SessionCompactionContinuityInspectionRecord {
  source: string;
  summary_message_id?: string | null;
  summary_text?: string | null;
  eligible_message_count?: number | null;
  exact_recent_tail_count?: number | null;
  omitted_older_turns?: number | null;
  has_working_ledger: boolean;
  has_memory_anchors: boolean;
  recall_policy?: string | null;
}

export interface ContextCompactionSummaryRecord {
  trigger: string;
  phase?: string | null;
  reason?: string | null;
  forced: boolean;
  request_context_tokens?: number | null;
  live_context_tokens?: number | null;
  limit_tokens?: number | null;
  body_chars?: number | null;
  message_count_before?: number | null;
  compacted_message_count?: number | null;
  kept_message_count?: number | null;
  summary?: string | null;
}

export interface ContextCompactionInstalledDiagnosticsRecord {
  request_context_tokens?: number | null;
  live_context_tokens?: number | null;
  body_chars?: number | null;
  cache_explanation?: string | null;
}

export interface ContextCompactionLifecycleSummaryRecord {
  trigger: string;
  phase?: string | null;
  reason?: string | null;
  status: "started" | "installed" | "failed" | "skipped";
  forced: boolean;
  request_context_tokens?: number | null;
  live_context_tokens?: number | null;
  limit_tokens?: number | null;
  body_chars?: number | null;
  installed?: ContextCompactionInstalledDiagnosticsRecord | null;
}

export interface SessionTelemetrySnapshotRecord {
  runtime: SessionRuntimeRecord;
  topology: SessionExecutionTopologyRecord;
  usage: SessionUsageRecord;
  tool_repair_summary?: SessionToolRepairTelemetrySummaryRecord | null;
  model_tool_repair_summary?: ModelToolRepairTelemetrySummaryRecord | null;
  tool_trajectory_quality?: ToolTrajectoryQualityRecord | null;
  tool_result_governance?: ToolResultGovernanceRecord | null;
  pending_permission_count?: number;
  granted_by_turn_count?: number;
  granted_by_session_count?: number;
  granted_by_matcher_kind?: Record<string, number> | null;
  last_permission_matcher_kind?: string | null;
  last_permission_grant_target?: string | null;
  last_permission_miss_count?: number;
  memory?: SessionMemoryTelemetryRecord | null;
  cache_evidence?: Record<string, unknown> | null;
  cache_semantics?: Record<string, unknown> | null;
  context_closure_contract?: SessionContextClosureContractRecord | null;
  context_compaction_summary?: ContextCompactionSummaryRecord | null;
  context_compaction_lifecycle_summary?: ContextCompactionLifecycleSummaryRecord | null;
  compaction_continuity?: SessionCompactionContinuityInspectionRecord | null;
  prompt_surface_evidence?: Record<string, unknown> | null;
  ingress_stabilization?: Record<string, unknown> | null;
  provider_diagnostic_summary?: Record<string, unknown> | null;
  runtime_protocol?: SessionRuntimeProtocolRecord | null;
  event_bus_telemetry?: EventBusTelemetryRecord | null;
}

export interface EventBusTelemetryRecord {
  send_count: number;
  send_error_count: number;
  max_receivers: number;
  last_send_at_ms: number;
  last_send_error_at_ms: number;
}

export type PromptIngressDispositionRecord =
  | "accept_now"
  | "queue_as_steering"
  | "blocked_on_question"
  | "blocked_on_permission"
  | "awaiting_interrupt";

export interface PermissionRuntimeSummaryRecord {
  pending: boolean;
  pending_permission_id?: string | null;
  pending_since_ms?: number | null;
  pending_tool?: string | null;
  last_pending_duration_ms?: number | null;
}

export interface SteeringRuntimeSummaryRecord {
  pending_count: number;
  last_enqueued_at_ms?: number | null;
  last_consumed_at_ms?: number | null;
  last_source_session_id?: string | null;
  last_latency_ms?: number | null;
}

export interface InterruptRuntimeSummaryRecord {
  phase: "idle" | "requested";
  requested_at_ms?: number | null;
  target?: "run" | "stage" | null;
}

export interface SessionRuntimeProtocolRecord {
  prompt_ingress: PromptIngressDispositionRecord;
  permission: PermissionRuntimeSummaryRecord;
  steering: SteeringRuntimeSummaryRecord;
  interrupt: InterruptRuntimeSummaryRecord;
}
