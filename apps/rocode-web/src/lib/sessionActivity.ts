import type {
  MemoryCardRecord,
  MemoryRetrievalPacketRecord,
  SessionMemoryTelemetryRecord,
} from "./memory";
import type { SessionMultimodalInsight } from "./multimodal";

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
  completion_tokens?: number | null;
  reasoning_tokens?: number | null;
  cache_read_tokens?: number | null;
  cache_write_tokens?: number | null;
  focus?: string | null;
  last_event?: string | null;
  waiting_on?: string | null;
  estimated_context_tokens?: number | null;
  skill_tree_budget?: number | null;
  skill_tree_truncation_strategy?: string | null;
  skill_tree_truncated?: boolean | null;
  retry_attempt?: number | null;
  active_agent_count: number;
  active_tool_count: number;
  child_session_count: number;
  primary_child_session_id?: string | null;
}

export interface SessionInsightsTelemetryRecord {
  version: string;
  usage: SessionUsageRecord;
  stage_summaries: StageSummaryRecord[];
  memory?: SessionMemoryTelemetryRecord | null;
  last_run_status: string;
  updated_at: number;
}

export interface SessionInsightsMemoryRecord {
  summary: SessionMemoryTelemetryRecord;
  frozen_snapshot?: MemoryRetrievalPacketRecord | null;
  last_prefetch_packet?: MemoryRetrievalPacketRecord | null;
  recent_session_records: MemoryCardRecord[];
}

export interface SessionInsightsRecord {
  id: string;
  title: string;
  directory: string;
  updated: number;
  telemetry?: SessionInsightsTelemetryRecord | null;
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

export interface SessionTelemetrySnapshotRecord {
  runtime: SessionRuntimeRecord;
  stages: StageSummaryRecord[];
  topology: SessionExecutionTopologyRecord;
  usage: SessionUsageRecord;
  memory?: SessionMemoryTelemetryRecord | null;
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
