// Mirror of agendao-types::task_ledger (wire contract). Only fields the
// task-state view reads; unknown fields pass through untouched.
export type TaskLedgerStatus =
  | "active"
  | "awaiting_user"
  | "blocked"
  | "interrupted"
  | "completed";

export type TaskLedgerActor = "user" | "model" | "evaluator" | "system";

export type TaskLedgerVerifier =
  | { evaluator: { name: string } }
  | { deterministic_check: { description: string } }
  | { user_confirmation: { actor: string } };

export interface TaskLedgerCoreConstraint {
  id: string;
  statement: string;
  live: boolean;
  set_by?: TaskLedgerActor;
  set_at?: number;
}

export interface TaskLedgerCheckpoint {
  id: string;
  claim: string;
  verifier: TaskLedgerVerifier;
  coverage: { scope: string };
  goal_generation?: number;
  covered_criteria?: string[];
  evidence_artifact_ids?: string[];
  source_stage_id?: string | null;
  superseded_by?: string | null;
}

export interface TaskLedgerOpenQuestion {
  id: string;
  question: string;
  settled_by: string;
  closed_by_checkpoint_id?: string | null;
}

export interface TaskLedgerProjection {
  live_core: TaskLedgerCoreConstraint[];
  open_questions: TaskLedgerOpenQuestion[];
  current_checkpoints: Array<TaskLedgerCheckpoint & { verifier_label: string }>;
  missing_acceptance_criteria: string[];
}

export interface SessionTaskLedger {
  session_id: string;
  revision: number;
  goal_generation?: number;
  goal: {
    statement: string;
    acceptance_criteria?: string[];
    criterion_checks?: Array<{ criterion: string; command: string }>;
    set_by: TaskLedgerActor;
    set_at: number;
  } | null;
  core: TaskLedgerCoreConstraint[];
  verified: TaskLedgerCheckpoint[];
  open: TaskLedgerOpenQuestion[];
  next: {
    statement: string;
    provenance: { actor: TaskLedgerActor; pre_interrupt: boolean; set_at?: number };
  } | null;
  status: TaskLedgerStatus;
  awaiting_interactions?: Array<{ kind: string; interaction_id: string }>;
  blocked_reason?: string | null;
  uncovered_criteria?: string[];
  updated_at: number;
  projection: TaskLedgerProjection;
}

export interface TaskLedgerWriteResponse {
  ledger: SessionTaskLedger;
  cause: string;
  metadata_key: string;
}
