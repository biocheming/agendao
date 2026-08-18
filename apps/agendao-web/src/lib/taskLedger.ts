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
  core: Array<{
    id: string;
    statement: string;
    live: boolean;
    set_by?: TaskLedgerActor;
    set_at?: number;
  }>;
  verified: Array<{
    id: string;
    claim: string;
    verifier: TaskLedgerVerifier;
    coverage: { scope: string };
    goal_generation?: number;
    covered_criteria?: string[];
    evidence_artifact_ids?: string[];
    source_stage_id?: string | null;
    superseded_by?: string | null;
  }>;
  open: Array<{
    id: string;
    question: string;
    settled_by: string;
    closed_by_checkpoint_id?: string | null;
  }>;
  next: {
    statement: string;
    provenance: { actor: TaskLedgerActor; pre_interrupt: boolean; set_at?: number };
  } | null;
  status: TaskLedgerStatus;
  awaiting_interactions?: Array<{ kind: string; interaction_id: string }>;
  blocked_reason?: string | null;
  uncovered_criteria?: string[];
  updated_at: number;
}

export interface TaskLedgerWriteResponse {
  ledger: SessionTaskLedger;
  cause: string;
  metadata_key: string;
}

export function openQuestions(ledger: SessionTaskLedger) {
  return ledger.open.filter((question) => !question.closed_by_checkpoint_id);
}

export function activeCheckpoints(ledger: SessionTaskLedger) {
  return ledger.verified.filter(
    (checkpoint) =>
      !checkpoint.superseded_by &&
      (checkpoint.goal_generation ?? 0) === (ledger.goal_generation ?? 0),
  );
}
