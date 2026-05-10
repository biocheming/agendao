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
