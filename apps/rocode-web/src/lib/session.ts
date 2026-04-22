export interface PersistedStageTelemetrySummary {
  stage_id: string;
  stage_name: string;
  status: string;
}

export interface PersistedSessionTelemetrySnapshot {
  version: string;
  usage: {
    input_tokens: number;
    output_tokens: number;
    reasoning_tokens: number;
    cache_write_tokens: number;
    cache_read_tokens: number;
    total_cost: number;
  };
  stage_summaries: PersistedStageTelemetrySummary[];
  last_run_status: string;
  updated_at: number;
}

export interface SessionListHintsRecord {
  current_model?: string | null;
  model_provider?: string | null;
  model_id?: string | null;
  scheduler_profile?: string | null;
  resolved_scheduler_profile?: string | null;
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
