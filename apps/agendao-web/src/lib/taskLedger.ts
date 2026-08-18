// Mirror of agendao-types::task_ledger (wire contract). Only fields the
// task-state view reads; unknown fields pass through untouched.
export type TaskLedgerStatus =
  | "active"
  | "awaiting_user"
  | "blocked"
  | "interrupted"
  | "completed";

export interface SessionTaskLedger {
  session_id: string;
  revision: number;
  goal_generation?: number;
  goal: { statement: string; acceptance_criteria?: string[] } | null;
  core: Array<{ id: string; statement: string; live: boolean }>;
  verified: Array<{
    id: string;
    claim: string;
    verifier: unknown;
    coverage: { scope: string };
    goal_generation?: number;
    covered_criteria?: string[];
    superseded_by?: string | null;
  }>;
  open: Array<{
    id: string;
    question: string;
    settled_by: string;
    closed_by_checkpoint_id?: string | null;
  }>;
  next: { statement: string; provenance: { actor: string; pre_interrupt: boolean } } | null;
  status: TaskLedgerStatus;
  awaiting_interactions?: Array<{ kind: string; interaction_id: string }>;
  blocked_reason?: string | null;
  uncovered_criteria?: string[];
  updated_at: number;
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
