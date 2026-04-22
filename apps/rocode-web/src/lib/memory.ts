export type MemoryRecordId = { 0: string } | string;

export function memoryRecordIdValue(value: MemoryRecordId | null | undefined): string {
  if (!value) return "";
  return typeof value === "string" ? value : value[0] ?? "";
}

export interface MemoryCardRecord {
  id: MemoryRecordId;
  kind: string;
  scope: string;
  status: string;
  title: string;
  summary: string;
  derived_skill_name?: string | null;
  linked_skill_name?: string | null;
  confidence?: number | null;
  validation_status: string;
  last_validated_at?: number | null;
}

export interface MemoryContractRecord {
  filter_query_parameters?: string[];
  search_fields?: string[];
  non_search_fields?: string[];
  note: string;
}

export interface MemoryListResponseRecord {
  items: MemoryCardRecord[];
  contract: MemoryContractRecord;
}

export interface MemoryEvidenceRefRecord {
  session_id?: string | null;
  message_id?: string | null;
  tool_call_id?: string | null;
  stage_id?: string | null;
  note?: string | null;
}

export interface MemoryDetailRecord {
  id: MemoryRecordId;
  kind: string;
  scope: string;
  status: string;
  title: string;
  summary: string;
  trigger_conditions?: string[];
  normalized_facts?: string[];
  boundaries?: string[];
  derived_skill_name?: string | null;
  linked_skill_name?: string | null;
  confidence?: number | null;
  evidence_refs?: MemoryEvidenceRefRecord[];
  source_session_id?: string | null;
  workspace_identity?: string | null;
  created_at: number;
  updated_at: number;
  last_validated_at?: number | null;
  validation_status: string;
}

export interface MemoryDetailResponseRecord {
  record: MemoryDetailRecord;
}

export interface MemoryValidationReportRecord {
  record_id?: MemoryRecordId | null;
  status: string;
  issues?: string[];
  checked_at: number;
}

export interface MemoryValidationReportResponseRecord {
  record_id: MemoryRecordId;
  latest?: MemoryValidationReportRecord | null;
}

export interface MemoryConflictViewRecord {
  id: string;
  record_id: MemoryRecordId;
  other_record_id: MemoryRecordId;
  conflict_kind: string;
  detail: string;
  detected_at: number;
}

export interface MemoryConflictResponseRecord {
  record_id: MemoryRecordId;
  conflicts?: MemoryConflictViewRecord[];
}

export interface MemoryRecallViewRecord {
  card: MemoryCardRecord;
  why_recalled: string;
  evidence_summary?: string | null;
}

export interface MemoryRetrievalPacketRecord {
  generated_at: number;
  snapshot: boolean;
  query?: string | null;
  scopes?: string[];
  items?: MemoryRecallViewRecord[];
  note?: string | null;
  budget_limit?: number | null;
}

export interface MemoryRetrievalPreviewResponseRecord {
  packet: MemoryRetrievalPacketRecord;
  contract: MemoryContractRecord;
}

export interface MemoryRuleDefinitionRecord {
  id: string;
  description: string;
  tags?: string[];
  promotion_target?: string | null;
}

export interface MemoryRulePackRecord {
  id: string;
  rule_pack_kind: string;
  version: string;
  rules?: MemoryRuleDefinitionRecord[];
  created_at: number;
  updated_at: number;
}

export interface MemoryRulePackListResponseRecord {
  items?: MemoryRulePackRecord[];
}

export interface MemoryRuleHitRecord {
  id: string;
  rule_pack_id?: string | null;
  memory_id?: MemoryRecordId | null;
  run_id?: string | null;
  hit_kind: string;
  detail?: string | null;
  created_at: number;
}

export interface MemoryRuleHitListResponseRecord {
  items?: MemoryRuleHitRecord[];
}

export interface MemoryConsolidationRunRecord {
  run_id: string;
  started_at: number;
  finished_at?: number | null;
  merged_count: number;
  promoted_count: number;
  conflict_count: number;
}

export interface MemoryConsolidationRunListResponseRecord {
  items?: MemoryConsolidationRunRecord[];
}

export interface MemoryConsolidationResponseRecord {
  run: MemoryConsolidationRunRecord;
  merged_record_ids?: MemoryRecordId[];
  promoted_record_ids?: MemoryRecordId[];
  archived_record_ids?: MemoryRecordId[];
  reflection_notes?: string[];
  rule_hits?: MemoryRuleHitRecord[];
}

export interface SessionMemoryTelemetryRecord {
  workspace_key: string;
  workspace_mode: string;
  allowed_scopes: string[];
  frozen_snapshot_generated_at?: number | null;
  frozen_snapshot_items: number;
  last_prefetch_generated_at?: number | null;
  last_prefetch_items: number;
  last_prefetch_query?: string | null;
  candidate_count: number;
  validated_count: number;
  rejected_count: number;
  warning_count: number;
  methodology_candidate_count: number;
  derived_skill_candidate_count: number;
  linked_skill_count: number;
  skill_feedback_lesson_count: number;
  retrieval_run_count: number;
  retrieval_hit_count: number;
  retrieval_use_count: number;
  latest_consolidation_run?: MemoryConsolidationRunRecord | null;
  recent_rule_hits: MemoryRuleHitRecord[];
}
