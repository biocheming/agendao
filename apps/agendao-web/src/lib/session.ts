export interface PersistedStageTelemetrySummary {
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
  active_agent_count?: number;
  active_tool_count?: number;
  attached_session_count?: number;
  primary_attached_session_id?: string | null;
}

export interface PersistedCompactionContinuityInspectionRecord {
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

export interface ToolTrajectoryQualityRecord {
  score: number;
  band: string;
  total_tool_calls: number;
  repaired_tool_call_count: number;
  error_tool_call_count: number;
  repair_event_count: number;
  provider_diagnostic_count: number;
  strict_would_fail_count: number;
  invalid_reroute_count: number;
  sanitizer_event_count: number;
  orphan_tool_result_count: number;
  duplicate_tool_id_count: number;
  malformed_placeholder_count: number;
  trailing_invalid_thinking_count: number;
  penalties?: Array<{ key: string; count: number; points: number }>;
  notes?: string[];
}

export interface ToolResultGovernanceRecord {
  single_result_governed_count: number;
  batch_governed_count: number;
  transcript_fallback_count: number;
  artifact_fallback_count: number;
  total_original_chars: number;
  total_displayed_chars: number;
}

export interface PersistedSessionTelemetrySnapshot {
  version: string;
  usage: {
    input_tokens: number;
    output_tokens: number;
    reasoning_tokens: number;
    cache_write_tokens: number;
    cache_read_tokens: number;
    cache_miss_tokens: number;
    context_tokens?: number;
    total_cost: number;
  };
  stage_summaries: PersistedStageTelemetrySummary[];
  compaction_continuity?: PersistedCompactionContinuityInspectionRecord | null;
  tool_trajectory_quality?: ToolTrajectoryQualityRecord | null;
  tool_result_governance?: ToolResultGovernanceRecord | null;
  pending_permission_count?: number;
  granted_by_turn_count?: number;
  granted_by_session_count?: number;
  granted_by_matcher_kind?: Record<string, number> | null;
  last_permission_matcher_kind?: string | null;
  last_permission_grant_target?: string | null;
  last_permission_miss_count?: number;
  last_run_status: string;
  updated_at: number;
}

export interface SessionListHintsRecord {
  current_model?: string | null;
  model_provider?: string | null;
  model_id?: string | null;
  scheduler_profile?: string | null;
  agent?: string | null;
}

export interface PendingCommandInvocationRecord {
  title?: string;
  command: string;
  rawArguments?: string;
  missingFields?: string[];
  schedulerProfile?: string;
  questionId?: string;
}

export interface SessionRecord {
  id: string;
  title: string;
  parent_id?: string;
  directory?: string;
  project_id?: string;
  updated?: number;
  hints?: SessionListHintsRecord | null;
  pending_command_invocation?: PendingCommandInvocationRecord | null;
  telemetry?: PersistedSessionTelemetrySnapshot | null;
  metadata?: Record<string, unknown> | null;
  time?: {
    updated?: number;
  };
}

export const OPTIMISTIC_SESSION_ID_PREFIX = "optimistic:";

export function isOptimisticSessionId(sessionId: string | null | undefined): boolean {
  return Boolean(sessionId && sessionId.startsWith(OPTIMISTIC_SESSION_ID_PREFIX));
}

export interface ExternalAdapterResolvedBindingRecord {
  session_id: string;
  actor_id: string;
  workspace_id: string;
  route_policy_id?: string | null;
}

export interface ProvisionExternalAdapterSessionRequestRecord {
  adapter_id: string;
  actor_id: string;
  workspace_id?: string | null;
  route_policy_id?: string | null;
  scheduler_profile?: string | null;
  directory?: string | null;
  project_id?: string | null;
  title?: string | null;
}

export interface ProvisionExternalAdapterSessionResponseRecord {
  adapter: string;
  source: string;
  binding: ExternalAdapterResolvedBindingRecord;
  session: SessionRecord;
}

export interface SessionListContractRecord {
  filter_query_parameters: string[];
  search_fields: string[];
  non_search_fields: string[];
  note: string;
}

export interface SessionListResponseRecord {
  items: SessionRecord[];
  contract: SessionListContractRecord;
}
