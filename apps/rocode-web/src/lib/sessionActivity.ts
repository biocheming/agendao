import type {
  MemoryCardRecord,
  MemoryRetrievalPacketRecord,
  SessionMemoryTelemetryRecord,
} from "./memory";
import type { SessionMultimodalInsight } from "./multimodal";
import type { PersistedSessionTelemetrySnapshot } from "./session";

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

export interface StageSummaryRecord {
  stage_id: string;
  stage_name: string;
  index?: number | null;
  total?: number | null;
  step?: number | null;
  step_total?: number | null;
  status: string;
  prompt_tokens?: number | null;
  context_tokens?: number | null;
  completion_tokens?: number | null;
  reasoning_tokens?: number | null;
  cache_read_tokens?: number | null;
  cache_miss_tokens?: number | null;
  cache_write_tokens?: number | null;
  focus?: string | null;
  last_event?: string | null;
  waiting_on?: string | null;
  activity?: string | null;
  estimated_context_tokens?: number | null;
  skill_tree_budget?: number | null;
  skill_tree_truncation_strategy?: string | null;
  skill_tree_truncated?: boolean | null;
  retry_attempt?: number | null;
  active_agent_count: number;
  active_tool_count: number;
  attached_session_count: number;
  primary_attached_session_id?: string | null;
}

export interface SessionInsightsMemoryRecord {
  summary: SessionMemoryTelemetryRecord;
  frozen_snapshot?: MemoryRetrievalPacketRecord | null;
  last_prefetch_packet?: MemoryRetrievalPacketRecord | null;
  recent_session_records: MemoryCardRecord[];
}

export interface SessionEffectiveSchedulerTraceStepRecord {
  kind: string;
  profile?: string | null;
  detail?: string | null;
  applied: boolean;
}

export interface SessionEffectiveSchedulerPolicyRecord {
  requested_profile?: string | null;
  effective_profile?: string | null;
  source: string;
  applied: boolean;
  mode_kind?: string | null;
  root_agent?: string | null;
  resolved_agent?: string | null;
  selection_trace: SessionEffectiveSchedulerTraceStepRecord[];
  warning?: string | null;
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

export interface SessionChildHistoryIsolationContractRecord {
  attached_subtree_session_count: number;
  owner_session_cumulative_tokens: number;
  workflow_cumulative_tokens: number;
  attached_subtree_cumulative_tokens: number;
  owner_live_context_tokens?: number | null;
  owner_local_live_prefix: boolean;
  child_history_in_live_prefix_detected: boolean;
  explanation: string;
}

export interface SessionContextClosureContractRecord {
  prefix_stability: SessionPrefixStabilityContractRecord;
  compaction_boundary: SessionCompactionBoundaryContractRecord;
  cache_explainability: SessionCacheExplainabilityContractRecord;
  child_history_isolation: SessionChildHistoryIsolationContractRecord;
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

export interface SessionTelemetrySnapshotRecord {
  runtime: SessionRuntimeRecord;
  stages: StageSummaryRecord[];
  topology: SessionExecutionTopologyRecord;
  usage: SessionUsageRecord;
  memory?: SessionMemoryTelemetryRecord | null;
  cache_evidence?: Record<string, unknown> | null;
  cache_semantics?: Record<string, unknown> | null;
  context_closure_contract?: SessionContextClosureContractRecord | null;
  compaction_continuity?: SessionCompactionContinuityInspectionRecord | null;
  prompt_surface_evidence?: Record<string, unknown> | null;
  ingress_stabilization?: Record<string, unknown> | null;
  provider_diagnostic_summary?: Record<string, unknown> | null;
}

export interface ActivityEventRecord {
  event_id?: string;
  scope?: string;
  ts?: number;
  event_type?: string;
  stage_id?: string | null;
  execution_id?: string | null;
  summary?: string | null;
  payload?: Record<string, unknown> | null;
}
