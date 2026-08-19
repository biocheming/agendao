//! Session task ledger — the server-authoritative task governance contract.
//!
//! The ledger answers a different question than the todo list: not "what work
//! is left" but "what is now known" — the goal, the constraints that must stay
//! consistent, verified conclusions with their verifier and coverage, open
//! questions with their settling evidence, and the single next action.
//!
//! Invariants (j-space governance plan §Phase 2; none may be weakened):
//! 1. `verified` is append-only; corrections supersede, never overwrite.
//! 2. A checkpoint requires claim + verifier + coverage; "tests pass" alone is
//!    not a valid checkpoint.
//! 3. An open question has a stable id and a settling condition; it closes
//!    only against a checkpoint created in the same transaction.
//! 4. `active`/`blocked` require a non-empty `next`; `completed` may clear it.
//! 5. `awaiting_user` carries the interaction kind/id and keeps a `next`.
//! 6. `interrupted` retains the pre-interrupt `next`, marked as such.
//! 7. At most two core constraints are live; swaps bump the revision.
//! 8. Every write carries an expected revision (CAS).
//! 9. Ledgers are per-session; this module never keys by path.

use serde::{Deserialize, Serialize};

/// Session metadata key under which the server authority persists the
/// ledger snapshot. Defined here so every layer (session fork, server
/// authority, future tooling) shares one spelling.
pub const TASK_LEDGER_METADATA_KEY: &str = "task_ledger";

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTaskLedger {
    pub session_id: String,
    pub revision: u64,
    /// Monotonic identity for the current goal. Checkpoints only satisfy
    /// acceptance criteria from the generation in which they were created.
    #[serde(default)]
    pub goal_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<TaskGoal>,
    #[serde(default)]
    pub core: Vec<CoreConstraint>,
    #[serde(default)]
    pub verified: Vec<VerifiedCheckpoint>,
    #[serde(default)]
    pub open: Vec<OpenQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<NextAction>,
    #[serde(default = "default_status")]
    pub status: TaskLedgerStatus,
    #[serde(default)]
    pub awaiting_interactions: Vec<AwaitingInteractionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// Criteria the completion explicitly left uncovered — persisted so the
    /// audit of "what was declared done without evidence" survives restarts.
    #[serde(default)]
    pub uncovered_criteria: Vec<String>,
    pub updated_at: i64,
}

fn default_status() -> TaskLedgerStatus {
    TaskLedgerStatus::Active
}

impl SessionTaskLedger {
    /// Read model for sessions that never entered structured governance: an
    /// empty snapshot, not fabricated state.
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            revision: 0,
            goal_generation: 0,
            goal: None,
            core: Vec::new(),
            verified: Vec::new(),
            open: Vec::new(),
            next: None,
            status: TaskLedgerStatus::Active,
            awaiting_interactions: Vec::new(),
            blocked_reason: None,
            uncovered_criteria: Vec::new(),
            updated_at: 0,
        }
    }

    pub fn live_core(&self) -> Vec<&CoreConstraint> {
        self.core.iter().filter(|entry| entry.live).collect()
    }

    pub fn open_questions(&self) -> Vec<&OpenQuestion> {
        self.open.iter().filter(|q| q.is_open()).collect()
    }

    pub fn apply(
        &mut self,
        expected_revision: u64,
        op: TaskLedgerOp,
        now_ms: i64,
    ) -> Result<u64, TaskLedgerError> {
        self.apply_batch(expected_revision, vec![op], now_ms)
    }

    /// Apply a seam's candidate set as ONE committed write: all ops validate
    /// and apply, or nothing applies. A single revision bump means one
    /// canonical replacement event per seam, and a rejected candidate can
    /// never leave a half-applied transaction behind.
    pub fn apply_batch(
        &mut self,
        expected_revision: u64,
        ops: Vec<TaskLedgerOp>,
        now_ms: i64,
    ) -> Result<u64, TaskLedgerError> {
        if self.revision != expected_revision {
            return Err(TaskLedgerError::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        let mut staged = self.clone();
        for op in ops {
            apply_op(&mut staged, op, now_ms)?;
        }
        validate_snapshot(&staged)?;
        staged.revision = self.revision + 1;
        staged.updated_at = now_ms;
        *self = staged;
        Ok(self.revision)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskLedgerStatus {
    #[default]
    Active,
    AwaitingUser,
    Blocked,
    Interrupted,
    Completed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AwaitingInteractionKind {
    Permission,
    Question,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AwaitingInteractionRef {
    pub kind: AwaitingInteractionKind,
    pub interaction_id: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLedgerActor {
    User,
    Model,
    Evaluator,
    #[default]
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskGoal {
    pub statement: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    /// Deterministic checks bound to named criteria. The command runs in
    /// the session workspace at the final-response seam; exit 0 is the
    /// only thing that can produce criterion-covering evidence (model
    /// judges never can). Set by the user through the authority API — the
    /// explicit opt-in that makes server-side execution legitimate.
    #[serde(default)]
    pub criterion_checks: Vec<CriterionCheck>,
    pub set_by: TaskLedgerActor,
    pub set_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CriterionCheck {
    /// Must match an entry in `acceptance_criteria` verbatim.
    pub criterion: String,
    /// Shell command; exit 0 = the criterion is met.
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoreConstraint {
    pub id: String,
    pub statement: String,
    /// At most two entries are live at once; the rest are parked and reloaded
    /// on demand.
    pub live: bool,
    /// Provenance is stored by the authority rather than inferred from the
    /// surface that happens to render the constraint. Legacy snapshots use
    /// system/zero through serde defaults.
    #[serde(default)]
    pub set_by: TaskLedgerActor,
    #[serde(default)]
    pub set_at: i64,
}

/// What established a checkpoint. Only evaluators, deterministic checks, or
/// explicit user confirmation create verified state — a model's own claim is
/// never a verifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VerifierRef {
    Evaluator { name: String },
    DeterministicCheck { description: String },
    UserConfirmation { actor: String },
}

impl VerifierRef {
    pub fn describe(&self) -> String {
        match self {
            Self::Evaluator { name } => format!("evaluator:{name}"),
            Self::DeterministicCheck { description } => format!("check:{description}"),
            Self::UserConfirmation { actor } => format!("user:{actor}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationCoverage {
    /// What the verification covered — stated with the conclusion, never
    /// implied by a bare "pass".
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiedCheckpoint {
    pub id: String,
    pub claim: String,
    pub verifier: VerifierRef,
    pub coverage: VerificationCoverage,
    /// Goal generation this evidence was verified against. Assigned by the
    /// ledger authority, never supplied by a caller.
    #[serde(default)]
    pub goal_generation: u64,
    #[serde(default)]
    pub evidence_artifact_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_stage_id: Option<String>,
    /// Set when this checkpoint corrects an earlier one; the earlier entry
    /// stays in the append-only history and points forward here.
    /// Acceptance criteria (verbatim goal strings) this checkpoint is
    /// evidence FOR. The completion gate matches criteria to evidence
    /// through this field only — an unrelated checkpoint covers nothing.
    #[serde(default)]
    pub covered_criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenQuestion {
    /// Stable, never-reused id (`open-01`, `open-02`, …).
    pub id: String,
    pub question: String,
    /// The cheapest evidence that could settle the question.
    pub settled_by: String,
    pub opened_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_by_checkpoint_id: Option<String>,
}

impl OpenQuestion {
    pub fn is_open(&self) -> bool {
        self.closed_by_checkpoint_id.is_none()
    }
}

/// Transport-only rendering projection. The raw ledger remains the sole
/// persisted authority; this view is rebuilt at API/event boundaries. The
/// projection describes derived UI state only: it does not authorize writes,
/// create evidence, or prove that an acceptance criterion is satisfied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TaskLedgerProjection {
    #[serde(default)]
    pub live_core: Vec<CoreConstraint>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
    #[serde(default)]
    pub current_checkpoints: Vec<TaskLedgerCheckpointProjection>,
    #[serde(default)]
    pub missing_acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskLedgerCheckpointProjection {
    #[serde(flatten)]
    pub checkpoint: VerifiedCheckpoint,
    pub verifier_label: String,
}

/// Backward-compatible wire view: raw ledger fields stay at the top level,
/// while frontends consume the server-derived `projection` field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTaskLedgerView {
    #[serde(flatten)]
    pub ledger: SessionTaskLedger,
    #[serde(default)]
    pub projection: TaskLedgerProjection,
}

impl From<&SessionTaskLedger> for TaskLedgerProjection {
    fn from(ledger: &SessionTaskLedger) -> Self {
        Self {
            live_core: ledger.live_core().into_iter().cloned().collect(),
            open_questions: ledger.open_questions().into_iter().cloned().collect(),
            current_checkpoints: current_checkpoints(ledger)
                .into_iter()
                .map(|checkpoint| TaskLedgerCheckpointProjection {
                    verifier_label: checkpoint.verifier.describe(),
                    checkpoint: checkpoint.clone(),
                })
                .collect(),
            missing_acceptance_criteria: missing_acceptance_criteria(
                ledger,
                &ledger.uncovered_criteria,
            ),
        }
    }
}

impl From<&SessionTaskLedger> for SessionTaskLedgerView {
    fn from(ledger: &SessionTaskLedger) -> Self {
        Self {
            ledger: ledger.clone(),
            projection: TaskLedgerProjection::from(ledger),
        }
    }
}

impl From<SessionTaskLedger> for SessionTaskLedgerView {
    fn from(ledger: SessionTaskLedger) -> Self {
        let projection = TaskLedgerProjection::from(&ledger);
        Self { ledger, projection }
    }
}

impl std::ops::Deref for SessionTaskLedgerView {
    type Target = SessionTaskLedger;

    fn deref(&self) -> &Self::Target {
        &self.ledger
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NextAction {
    pub statement: String,
    pub provenance: NextActionProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NextActionProvenance {
    pub actor: TaskLedgerActor,
    /// True while this `next` survives only because the run was interrupted;
    /// it is the pre-interrupt plan, not a post-recovery decision.
    #[serde(default)]
    pub pre_interrupt: bool,
    pub set_at: i64,
}

/// Typed cause carried by the canonical `task-ledger.replaced` event so
/// consumers can filter without parsing prose.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLedgerCause {
    Created,
    GoalUpdated,
    CoreUpdated,
    CheckpointAdded,
    OpenAdded,
    OpenClosed,
    NextUpdated,
    StatusChanged,
    Recovery,
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum TaskLedgerOp {
    Create {
        goal: TaskGoal,
        next_statement: String,
    },
    SetGoal {
        goal: TaskGoal,
    },
    AddCore {
        statement: String,
        /// When true the entry joins the live set (displacing nothing);
        /// when the live set is full it is parked with an error instead.
        live: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<TaskLedgerActor>,
    },
    SwapCoreLive {
        /// 1-based live slot (1 or 2).
        slot: u8,
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<TaskLedgerActor>,
    },
    AddCheckpoint {
        claim: String,
        verifier: VerifierRef,
        coverage: VerificationCoverage,
        #[serde(default)]
        covered_criteria: Vec<String>,
        #[serde(default)]
        evidence_artifact_ids: Vec<String>,
        #[serde(default)]
        source_stage_id: Option<String>,
        #[serde(default)]
        supersedes: Option<String>,
    },
    OpenQuestion {
        question: String,
        settled_by: String,
    },
    /// Closes an open question; the checkpoint payload is created in the same
    /// transaction and is the only way an open question closes.
    CloseOpenWithCheckpoint {
        open_id: String,
        claim: String,
        verifier: VerifierRef,
        coverage: VerificationCoverage,
        #[serde(default)]
        covered_criteria: Vec<String>,
        #[serde(default)]
        evidence_artifact_ids: Vec<String>,
        #[serde(default)]
        source_stage_id: Option<String>,
    },
    /// Remove one resolved interaction; the ledger leaves awaiting_user
    /// only when no interaction remains waited on.
    ResolveInteraction {
        kind: AwaitingInteractionKind,
        interaction_id: String,
    },
    SetNext {
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<TaskLedgerActor>,
    },
    SetStatus {
        status: TaskLedgerStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        awaiting: Option<AwaitingInteractionRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_reason: Option<String>,
    },
    Interrupt,
    /// Completing is a gated claim, not a label: open questions must be
    /// closed, and a goal with acceptance criteria needs either verified
    /// checkpoints or an explicit list of criteria left uncovered.
    Complete {
        #[serde(default)]
        uncovered: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLedgerError {
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    NotCreated,
    AlreadyCreated,
    EmptyGoalStatement,
    EmptyStatement,
    EmptyClaim,
    EmptyCoverageScope,
    EmptyVerifierDetail,
    EmptyQuestion,
    EmptySettledBy,
    EmptyBlockedReason,
    AwaitingRequiresInteraction,
    StatusRequiresNext {
        status: TaskLedgerStatus,
    },
    UnknownOpenQuestion {
        open_id: String,
    },
    OpenQuestionAlreadyClosed {
        open_id: String,
    },
    UnknownCheckpoint {
        checkpoint_id: String,
    },
    LiveSetFull,
    LiveSlotOutOfBounds {
        slot: u8,
    },
    DuplicateSupersede {
        checkpoint_id: String,
    },
    CompleteWithOpenQuestions {
        count: usize,
    },
    CompleteWithoutEvidence,
    CriterionNotCovered {
        criterion: String,
    },
    UnknownAcceptanceCriterion {
        criterion: String,
    },
    StatusConflictsWithAwaitingInteractions {
        status: TaskLedgerStatus,
        count: usize,
    },
    CompleteWithAwaitingInteractions {
        count: usize,
    },
    CompletedWithNext,
    UnknownAwaitingInteraction {
        interaction_id: String,
    },
}

impl std::fmt::Display for TaskLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RevisionConflict { expected, actual } => {
                write!(f, "revision conflict: expected {expected}, ledger is at {actual}")
            }
            Self::NotCreated => write!(f, "ledger not created for this session"),
            Self::AlreadyCreated => write!(f, "ledger already created for this session"),
            Self::EmptyGoalStatement => write!(f, "goal statement must not be empty"),
            Self::EmptyStatement => write!(f, "statement must not be empty"),
            Self::EmptyClaim => write!(f, "checkpoint claim must not be empty"),
            Self::EmptyCoverageScope => {
                write!(f, "checkpoint coverage scope must not be empty")
            }
            Self::EmptyVerifierDetail => write!(f, "verifier detail must not be empty"),
            Self::EmptyQuestion => write!(f, "open question must not be empty"),
            Self::EmptySettledBy => {
                write!(f, "open question requires a settling condition")
            }
            Self::EmptyBlockedReason => {
                write!(f, "blocked status requires a reason")
            }
            Self::AwaitingRequiresInteraction => write!(
                f,
                "awaiting_user status requires an interaction kind and id"
            ),
            Self::StatusRequiresNext { status } => {
                write!(f, "status {status:?} requires a non-empty next action")
            }
            Self::UnknownOpenQuestion { open_id } => {
                write!(f, "unknown open question {open_id}")
            }
            Self::OpenQuestionAlreadyClosed { open_id } => {
                write!(f, "open question {open_id} is already closed")
            }
            Self::UnknownCheckpoint { checkpoint_id } => {
                write!(f, "unknown checkpoint {checkpoint_id}")
            }
            Self::LiveSetFull => write!(f, "core live set already holds two entries"),
            Self::LiveSlotOutOfBounds { slot } => {
                write!(f, "live core slot must be 1 or 2, got {slot}")
            }
            Self::DuplicateSupersede { checkpoint_id } => {
                write!(f, "checkpoint {checkpoint_id} is already superseded")
            }
            Self::CompleteWithOpenQuestions { count } => write!(
                f,
                "cannot complete with {count} open question(s); close them with checkpoints or keep the ledger active"
            ),
            Self::CompleteWithoutEvidence => write!(
                f,
                "cannot complete a goal with acceptance criteria and no verified checkpoints; pass `uncovered` explicitly instead"
            ),
            Self::CriterionNotCovered { criterion } => write!(
                f,
                "acceptance criterion not covered by any checkpoint and not declared uncovered: {criterion}"
            ),
            Self::UnknownAcceptanceCriterion { criterion } => {
                write!(f, "unknown acceptance criterion: {criterion}")
            }
            Self::StatusConflictsWithAwaitingInteractions { status, count } => write!(
                f,
                "status {status:?} conflicts with {count} pending interaction(s); resolve them or interrupt the run"
            ),
            Self::CompleteWithAwaitingInteractions { count } => write!(
                f,
                "cannot complete with {count} pending interaction(s)"
            ),
            Self::CompletedWithNext => {
                write!(f, "completed status cannot retain a next action")
            }
            Self::UnknownAwaitingInteraction { interaction_id } => {
                write!(f, "no awaited interaction with id {interaction_id}")
            }
        }
    }
}

impl std::error::Error for TaskLedgerError {}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn next_sequential_id<'a>(existing: impl IntoIterator<Item = &'a str>, prefix: &str) -> String {
    let next = existing
        .into_iter()
        .filter_map(|id| id.strip_prefix(prefix))
        .filter_map(|tail| tail.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    format!("{prefix}{next:02}")
}

fn validate_goal(goal: &TaskGoal) -> Result<(), TaskLedgerError> {
    if !non_empty(&goal.statement) {
        return Err(TaskLedgerError::EmptyGoalStatement);
    }
    for check in &goal.criterion_checks {
        if !non_empty(&check.command) {
            return Err(TaskLedgerError::EmptyStatement);
        }
        if !goal
            .acceptance_criteria
            .iter()
            .any(|c| c == &check.criterion)
        {
            return Err(TaskLedgerError::UnknownAcceptanceCriterion {
                criterion: check.criterion.clone(),
            });
        }
    }
    Ok(())
}

fn validated_verifier(verifier: &VerifierRef) -> Result<(), TaskLedgerError> {
    let detail = match verifier {
        VerifierRef::Evaluator { name } => name,
        VerifierRef::DeterministicCheck { description } => description,
        VerifierRef::UserConfirmation { actor } => actor,
    };
    if non_empty(detail) {
        Ok(())
    } else {
        Err(TaskLedgerError::EmptyVerifierDetail)
    }
}

fn validated_checkpoint_fields(
    claim: &str,
    verifier: &VerifierRef,
    coverage: &VerificationCoverage,
) -> Result<(), TaskLedgerError> {
    if !non_empty(claim) {
        return Err(TaskLedgerError::EmptyClaim);
    }
    validated_verifier(verifier)?;
    if !non_empty(&coverage.scope) {
        return Err(TaskLedgerError::EmptyCoverageScope);
    }
    Ok(())
}

fn require_next_for_status(
    ledger: &SessionTaskLedger,
    status: &TaskLedgerStatus,
) -> Result<(), TaskLedgerError> {
    match status {
        TaskLedgerStatus::Active | TaskLedgerStatus::Blocked => {
            let has_next = ledger
                .next
                .as_ref()
                .is_some_and(|next| non_empty(&next.statement));
            if has_next {
                Ok(())
            } else {
                Err(TaskLedgerError::StatusRequiresNext { status: *status })
            }
        }
        _ => Ok(()),
    }
}

fn validate_completion(
    ledger: &SessionTaskLedger,
    uncovered: &[String],
) -> Result<(), TaskLedgerError> {
    let open = ledger.open_questions().len();
    if open > 0 {
        return Err(TaskLedgerError::CompleteWithOpenQuestions { count: open });
    }
    if !ledger.awaiting_interactions.is_empty() {
        return Err(TaskLedgerError::CompleteWithAwaitingInteractions {
            count: ledger.awaiting_interactions.len(),
        });
    }
    let criteria = ledger
        .goal
        .as_ref()
        .map(|goal| goal.acceptance_criteria.clone())
        .unwrap_or_default();
    validate_criteria_references(&criteria, uncovered)?;
    let checkpoints = current_checkpoints(ledger);
    for checkpoint in &checkpoints {
        validate_criteria_references(&criteria, &checkpoint.covered_criteria)?;
    }
    if criteria.is_empty() {
        // A goal without explicit criteria still needs current, non-superseded
        // evidence. Arbitrary `uncovered` strings cannot stand in for criteria
        // that do not exist.
        if checkpoints.is_empty() {
            return Err(TaskLedgerError::CompleteWithoutEvidence);
        }
        return Ok(());
    }
    let covered: std::collections::HashSet<&String> = ledger
        .verified
        .iter()
        .filter(|checkpoint| {
            checkpoint.goal_generation == ledger.goal_generation
                && checkpoint.superseded_by.is_none()
        })
        .flat_map(|checkpoint| checkpoint.covered_criteria.iter())
        .collect();
    for criterion in &criteria {
        let declared = uncovered.iter().any(|left| left == criterion);
        if !covered.contains(criterion) && !declared {
            return Err(TaskLedgerError::CriterionNotCovered {
                criterion: criterion.clone(),
            });
        }
    }
    Ok(())
}

/// Whether the ledger may complete right now with no explicit uncovered
/// declarations — used by the final-response seam to auto-complete only on
/// fully evidenced goals.
pub fn completion_ready(ledger: &SessionTaskLedger) -> bool {
    validate_completion(ledger, &[]).is_ok()
}

/// Current evidence excludes historical goal generations and superseded
/// claims. History remains append-only, but history is not authority.
pub fn current_checkpoints(ledger: &SessionTaskLedger) -> Vec<&VerifiedCheckpoint> {
    ledger
        .verified
        .iter()
        .filter(|checkpoint| {
            checkpoint.goal_generation == ledger.goal_generation
                && checkpoint.superseded_by.is_none()
        })
        .collect()
}

/// Acceptance criteria that have neither current evidence nor an explicit
/// uncovered declaration.
pub fn missing_acceptance_criteria(
    ledger: &SessionTaskLedger,
    uncovered: &[String],
) -> Vec<String> {
    let Some(goal) = ledger.goal.as_ref() else {
        return Vec::new();
    };
    let covered: std::collections::HashSet<&str> = current_checkpoints(ledger)
        .into_iter()
        .flat_map(|checkpoint| checkpoint.covered_criteria.iter().map(String::as_str))
        .collect();
    goal.acceptance_criteria
        .iter()
        .filter(|criterion| {
            !covered.contains(criterion.as_str())
                && !uncovered.iter().any(|item| item == *criterion)
        })
        .cloned()
        .collect()
}

fn validate_criteria_references(
    criteria: &[String],
    references: &[String],
) -> Result<(), TaskLedgerError> {
    for reference in references {
        if !criteria.iter().any(|criterion| criterion == reference) {
            return Err(TaskLedgerError::UnknownAcceptanceCriterion {
                criterion: reference.clone(),
            });
        }
    }
    Ok(())
}

fn validate_snapshot(ledger: &SessionTaskLedger) -> Result<(), TaskLedgerError> {
    require_next_for_status(ledger, &ledger.status)?;
    match ledger.status {
        TaskLedgerStatus::AwaitingUser => {
            if ledger.awaiting_interactions.is_empty() {
                return Err(TaskLedgerError::AwaitingRequiresInteraction);
            }
        }
        TaskLedgerStatus::Completed => {
            if ledger.next.is_some() {
                return Err(TaskLedgerError::CompletedWithNext);
            }
            validate_completion(ledger, &ledger.uncovered_criteria)?;
        }
        status if !ledger.awaiting_interactions.is_empty() => {
            return Err(TaskLedgerError::StatusConflictsWithAwaitingInteractions {
                status,
                count: ledger.awaiting_interactions.len(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn apply_op(
    ledger: &mut SessionTaskLedger,
    op: TaskLedgerOp,
    now_ms: i64,
) -> Result<(), TaskLedgerError> {
    match op {
        TaskLedgerOp::Create {
            goal,
            next_statement,
        } => {
            if ledger.revision != 0 || ledger.goal.is_some() {
                return Err(TaskLedgerError::AlreadyCreated);
            }
            validate_goal(&goal)?;
            if !non_empty(&next_statement) {
                return Err(TaskLedgerError::EmptyStatement);
            }
            ledger.goal = Some(goal);
            ledger.goal_generation = 1;
            ledger.next = Some(next_from_statement(
                next_statement,
                TaskLedgerActor::User,
                now_ms,
            ));
            ledger.status = TaskLedgerStatus::Active;
            Ok(())
        }
        TaskLedgerOp::SetGoal { goal } => {
            ensure_created(ledger)?;
            validate_goal(&goal)?;
            ledger.goal = Some(goal);
            ledger.goal_generation = ledger.goal_generation.saturating_add(1).max(1);
            ledger.uncovered_criteria.clear();
            Ok(())
        }
        TaskLedgerOp::AddCore {
            statement,
            live,
            actor,
        } => {
            ensure_created(ledger)?;
            if !non_empty(&statement) {
                return Err(TaskLedgerError::EmptyStatement);
            }
            if live && ledger.live_core().len() >= 2 {
                return Err(TaskLedgerError::LiveSetFull);
            }
            let id = next_sequential_id(ledger.core.iter().map(|entry| entry.id.as_str()), "core-");
            ledger.core.push(CoreConstraint {
                id,
                statement,
                live,
                set_by: actor.unwrap_or(TaskLedgerActor::System),
                set_at: now_ms,
            });
            Ok(())
        }
        TaskLedgerOp::SwapCoreLive {
            slot,
            statement,
            actor,
        } => {
            ensure_created(ledger)?;
            if !non_empty(&statement) {
                return Err(TaskLedgerError::EmptyStatement);
            }
            if slot != 1 && slot != 2 {
                return Err(TaskLedgerError::LiveSlotOutOfBounds { slot });
            }
            // Slot order is positional among live entries; the incoming
            // statement takes the displaced entry's place so slot 1/2 keep
            // meaning across swaps.
            let live_positions: Vec<usize> = ledger
                .core
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.live)
                .map(|(index, _)| index)
                .collect();
            let displaced_position = live_positions.get(slot as usize - 1).copied();
            if let Some(position) = displaced_position {
                ledger.core[position].live = false;
            }
            let existing_position = ledger
                .core
                .iter()
                .position(|entry| entry.statement == statement);
            let insert_at = displaced_position.unwrap_or(ledger.core.len());
            let entry = match existing_position {
                Some(position) => {
                    let mut entry = ledger.core.remove(position);
                    entry.live = true;
                    entry.set_by = actor.unwrap_or(TaskLedgerActor::System);
                    entry.set_at = now_ms;
                    entry
                }
                None => {
                    let id = next_sequential_id(
                        ledger.core.iter().map(|entry| entry.id.as_str()),
                        "core-",
                    );
                    CoreConstraint {
                        id,
                        statement,
                        live: true,
                        set_by: actor.unwrap_or(TaskLedgerActor::System),
                        set_at: now_ms,
                    }
                }
            };
            let insert_at = insert_at.min(ledger.core.len());
            ledger.core.insert(insert_at, entry);
            Ok(())
        }
        TaskLedgerOp::AddCheckpoint {
            claim,
            verifier,
            coverage,
            covered_criteria,
            evidence_artifact_ids,
            source_stage_id,
            supersedes,
        } => {
            ensure_created(ledger)?;
            validated_checkpoint_fields(&claim, &verifier, &coverage)?;
            let criteria = &ledger
                .goal
                .as_ref()
                .expect("created ledger has goal")
                .acceptance_criteria;
            validate_criteria_references(criteria, &covered_criteria)?;
            let id = next_sequential_id(
                ledger
                    .verified
                    .iter()
                    .map(|checkpoint| checkpoint.id.as_str()),
                "chk-",
            );
            if let Some(superseded_id) = supersedes.as_deref() {
                let target = ledger
                    .verified
                    .iter_mut()
                    .find(|checkpoint| checkpoint.id == superseded_id)
                    .ok_or(TaskLedgerError::UnknownCheckpoint {
                        checkpoint_id: superseded_id.to_string(),
                    })?;
                if target.superseded_by.is_some() {
                    return Err(TaskLedgerError::DuplicateSupersede {
                        checkpoint_id: superseded_id.to_string(),
                    });
                }
                target.superseded_by = Some(id.clone());
            }
            ledger.verified.push(VerifiedCheckpoint {
                id,
                claim,
                verifier,
                coverage,
                goal_generation: ledger.goal_generation,
                covered_criteria,
                evidence_artifact_ids,
                source_stage_id,
                supersedes,
                superseded_by: None,
                created_at: now_ms,
            });
            Ok(())
        }
        TaskLedgerOp::OpenQuestion {
            question,
            settled_by,
        } => {
            ensure_created(ledger)?;
            if !non_empty(&question) {
                return Err(TaskLedgerError::EmptyQuestion);
            }
            if !non_empty(&settled_by) {
                return Err(TaskLedgerError::EmptySettledBy);
            }
            let id = next_sequential_id(
                ledger.open.iter().map(|question| question.id.as_str()),
                "open-",
            );
            ledger.open.push(OpenQuestion {
                id,
                question,
                settled_by,
                opened_at: now_ms,
                closed_by_checkpoint_id: None,
            });
            Ok(())
        }
        TaskLedgerOp::CloseOpenWithCheckpoint {
            open_id,
            claim,
            verifier,
            coverage,
            covered_criteria,
            evidence_artifact_ids,
            source_stage_id,
        } => {
            ensure_created(ledger)?;
            validated_checkpoint_fields(&claim, &verifier, &coverage)?;
            let criteria = &ledger
                .goal
                .as_ref()
                .expect("created ledger has goal")
                .acceptance_criteria;
            validate_criteria_references(criteria, &covered_criteria)?;
            let target = ledger
                .open
                .iter()
                .find(|question| question.id == open_id)
                .ok_or(TaskLedgerError::UnknownOpenQuestion {
                    open_id: open_id.clone(),
                })?;
            if !target.is_open() {
                return Err(TaskLedgerError::OpenQuestionAlreadyClosed { open_id });
            }
            let checkpoint_id = next_sequential_id(
                ledger
                    .verified
                    .iter()
                    .map(|checkpoint| checkpoint.id.as_str()),
                "chk-",
            );
            ledger.verified.push(VerifiedCheckpoint {
                id: checkpoint_id.clone(),
                claim,
                verifier,
                coverage,
                goal_generation: ledger.goal_generation,
                covered_criteria,
                evidence_artifact_ids,
                source_stage_id,
                supersedes: None,
                superseded_by: None,
                created_at: now_ms,
            });
            if let Some(question) = ledger
                .open
                .iter_mut()
                .find(|question| question.id == open_id)
            {
                question.closed_by_checkpoint_id = Some(checkpoint_id);
            }
            Ok(())
        }
        TaskLedgerOp::SetNext { statement, actor } => {
            ensure_created(ledger)?;
            if !non_empty(&statement) {
                return Err(TaskLedgerError::EmptyStatement);
            }
            let actor = actor.unwrap_or(TaskLedgerActor::User);
            ledger.next = Some(next_from_statement(statement, actor, now_ms));
            Ok(())
        }
        TaskLedgerOp::SetStatus {
            status,
            awaiting,
            blocked_reason,
        } => {
            ensure_created(ledger)?;
            match status {
                TaskLedgerStatus::AwaitingUser => {
                    let Some(awaiting) = awaiting else {
                        return Err(TaskLedgerError::AwaitingRequiresInteraction);
                    };
                    if !non_empty(&awaiting.interaction_id) {
                        return Err(TaskLedgerError::AwaitingRequiresInteraction);
                    }
                    // Multi-slot: concurrent waits (permission + question,
                    // several permissions) all stay tracked; resolution is
                    // per-interaction via ResolveInteraction.
                    let already = ledger.awaiting_interactions.contains(&awaiting.clone());
                    if !already {
                        ledger.awaiting_interactions.push(awaiting.clone());
                    }
                    // The wait itself is the next action; it must stay named.
                    require_next_for_status(ledger, &TaskLedgerStatus::Active)?;
                }
                TaskLedgerStatus::Blocked => {
                    let Some(reason) = blocked_reason.as_deref() else {
                        return Err(TaskLedgerError::EmptyBlockedReason);
                    };
                    if !non_empty(reason) {
                        return Err(TaskLedgerError::EmptyBlockedReason);
                    }
                    ledger.blocked_reason = blocked_reason;
                }
                TaskLedgerStatus::Active => {
                    if !ledger.awaiting_interactions.is_empty() {
                        return Err(TaskLedgerError::StatusConflictsWithAwaitingInteractions {
                            status,
                            count: ledger.awaiting_interactions.len(),
                        });
                    }
                    ledger.blocked_reason = None;
                }
                TaskLedgerStatus::Interrupted => {
                    // Same semantics as the Interrupt op: keep the
                    // pre-interrupt next, marked as such.
                    if let Some(next) = ledger.next.as_mut() {
                        next.provenance.pre_interrupt = true;
                    }
                    ledger.awaiting_interactions.clear();
                }
                TaskLedgerStatus::Completed => {
                    // Same gate, same clearing semantics as the Complete op —
                    // there is no weaker side door through SetStatus.
                    validate_completion(ledger, &ledger.uncovered_criteria.clone())?;
                    ledger.blocked_reason = None;
                    ledger.next = None;
                }
            }
            if ledger.status == TaskLedgerStatus::Completed && status != TaskLedgerStatus::Completed
            {
                ledger.uncovered_criteria.clear();
            }
            require_next_for_status(ledger, &status)?;
            ledger.status = status;
            Ok(())
        }
        TaskLedgerOp::ResolveInteraction {
            kind,
            interaction_id,
        } => {
            ensure_created(ledger)?;
            let before = ledger.awaiting_interactions.len();
            ledger.awaiting_interactions.retain(|current| {
                !(current.kind == kind && current.interaction_id == interaction_id)
            });
            if ledger.awaiting_interactions.len() == before {
                return Err(TaskLedgerError::UnknownAwaitingInteraction { interaction_id });
            }
            // Still waiting on something else? Stay awaiting; otherwise the
            // run resumes.
            if ledger.awaiting_interactions.is_empty()
                && ledger.status == TaskLedgerStatus::AwaitingUser
            {
                ledger.status = TaskLedgerStatus::Active;
            }
            Ok(())
        }
        TaskLedgerOp::Interrupt => {
            ensure_created(ledger)?;
            if let Some(next) = ledger.next.as_mut() {
                next.provenance.pre_interrupt = true;
            }
            ledger.status = TaskLedgerStatus::Interrupted;
            ledger.awaiting_interactions.clear();
            Ok(())
        }
        TaskLedgerOp::Complete { uncovered } => {
            ensure_created(ledger)?;
            validate_completion(ledger, &uncovered)?;
            ledger.uncovered_criteria = uncovered;
            ledger.status = TaskLedgerStatus::Completed;
            ledger.blocked_reason = None;
            ledger.next = None;
            Ok(())
        }
    }
}

fn ensure_created(ledger: &SessionTaskLedger) -> Result<(), TaskLedgerError> {
    if ledger.goal.is_some() {
        Ok(())
    } else {
        Err(TaskLedgerError::NotCreated)
    }
}

fn next_from_statement(statement: String, actor: TaskLedgerActor, now_ms: i64) -> NextAction {
    NextAction {
        statement,
        provenance: NextActionProvenance {
            actor,
            pre_interrupt: false,
            set_at: now_ms,
        },
    }
}

/// Typed execution facts the server can determine on its own — the only
/// seam inputs the ledger reducer accepts (no model-authored prose).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "seam")]
pub enum TaskLedgerSeamFact {
    RunStarted,
    ToolBatchCompleted {
        summary: crate::repair::ToolBatchSummary,
    },
    RecoveryInterrupted,
    FinalResponseCommitted,
    InteractionAwaiting {
        kind: AwaitingInteractionKind,
        interaction_id: String,
    },
    InteractionResolved {
        kind: AwaitingInteractionKind,
        interaction_id: String,
    },
    /// A scheduler evaluation gate finished. A pass is current-generation
    /// evidence (criterion coverage still requires an explicit mapping —
    /// the evaluator validates the node, not named acceptance criteria).
    EvaluatorGateCompleted {
        node_path: String,
        passed: bool,
        /// Goal generation actually presented to the evaluator. The reducer
        /// rejects the evidence if the ledger changed meanwhile. A model
        /// evaluator cannot itself cover named acceptance criteria.
        goal_generation: u64,
    },
}

// ---------------------------------------------------------------------------
// Tests — every invariant, by name
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(statement: &str) -> TaskGoal {
        TaskGoal {
            statement: statement.to_string(),
            acceptance_criteria: vec!["all tests pass".to_string()],
            criterion_checks: vec![],
            set_by: TaskLedgerActor::User,
            set_at: 1_000,
        }
    }

    fn created() -> SessionTaskLedger {
        let mut ledger = SessionTaskLedger::empty("ses_test");
        ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: goal("ship median with tests"),
                    next_statement: "write median".to_string(),
                },
                1_000,
            )
            .expect("create");
        ledger
    }

    fn checkpoint_op(claim: &str) -> TaskLedgerOp {
        TaskLedgerOp::AddCheckpoint {
            claim: claim.to_string(),
            verifier: VerifierRef::DeterministicCheck {
                description: "python3 -m unittest discover".to_string(),
            },
            coverage: VerificationCoverage {
                scope: "3 unit cases incl. empty input".to_string(),
            },
            covered_criteria: vec![],
            evidence_artifact_ids: vec![],
            source_stage_id: None,
            supersedes: None,
        }
    }

    #[test]
    fn transport_view_projects_rendering_state_and_preserves_wire_compatibility() {
        let mut ledger = created();
        ledger
            .apply_batch(
                1,
                vec![
                    TaskLedgerOp::AddCore {
                        statement: "preserve API compatibility".to_string(),
                        live: true,
                        actor: Some(TaskLedgerActor::System),
                    },
                    TaskLedgerOp::OpenQuestion {
                        question: "does the old client still decode?".to_string(),
                        settled_by: "serde compatibility test".to_string(),
                    },
                    checkpoint_op("transport view generated"),
                ],
                2_000,
            )
            .unwrap();

        let view = SessionTaskLedgerView::from(&ledger);
        assert_eq!(view.projection.live_core[0].id, "core-01");
        assert_eq!(view.projection.open_questions[0].id, "open-01");
        assert_eq!(
            view.projection.current_checkpoints[0].checkpoint.id,
            "chk-01"
        );
        assert_eq!(
            view.projection.current_checkpoints[0].verifier_label,
            "check:python3 -m unittest discover"
        );
        assert_eq!(
            view.projection.missing_acceptance_criteria,
            vec!["all tests pass"]
        );

        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["session_id"], "ses_test");
        assert!(value.get("projection").is_some());
        let old_client: SessionTaskLedger = serde_json::from_value(value).unwrap();
        assert_eq!(old_client, ledger);
    }

    #[test]
    fn invariant_1_verified_is_append_only_with_supersede() {
        let mut ledger = created();
        ledger
            .apply(1, checkpoint_op("median correct"), 2_000)
            .unwrap();
        ledger
            .apply(
                2,
                TaskLedgerOp::AddCheckpoint {
                    claim: "median correct incl. floats".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "python3 -m unittest discover -p float".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "adds float edge cases".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: Some("chk-01".to_string()),
                },
                3_000,
            )
            .unwrap();
        assert_eq!(ledger.verified.len(), 2, "history stays");
        assert_eq!(ledger.verified[0].superseded_by.as_deref(), Some("chk-02"));
        assert_eq!(ledger.verified[1].supersedes.as_deref(), Some("chk-01"));
        // Double supersede is rejected.
        let err = ledger
            .apply(
                3,
                TaskLedgerOp::AddCheckpoint {
                    claim: "again".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "re-check".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "scope".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: Some("chk-01".to_string()),
                },
                4_000,
            )
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::DuplicateSupersede {
                checkpoint_id: "chk-01".to_string()
            }
        );
    }

    #[test]
    fn invariant_2_checkpoint_requires_claim_verifier_coverage() {
        let mut ledger = created();
        let err = ledger
            .apply(
                1,
                TaskLedgerOp::AddCheckpoint {
                    claim: "  ".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "run tests".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "3 cases".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: None,
                },
                2_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyClaim);

        let err = ledger
            .apply(
                1,
                TaskLedgerOp::AddCheckpoint {
                    claim: "works".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: " ".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "3 cases".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: None,
                },
                2_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyVerifierDetail);

        let err = ledger
            .apply(
                1,
                TaskLedgerOp::AddCheckpoint {
                    claim: "works".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "run tests".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: String::new(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: None,
                },
                2_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyCoverageScope);
    }

    #[test]
    fn invariant_3_open_closes_only_against_same_transaction_checkpoint() {
        let mut ledger = created();
        ledger
            .apply(
                1,
                TaskLedgerOp::OpenQuestion {
                    question: "even case rounding?".to_string(),
                    settled_by: "differential test vs sorted brute".to_string(),
                },
                2_000,
            )
            .unwrap();
        // Unknown open id.
        let err = ledger
            .apply(
                2,
                TaskLedgerOp::CloseOpenWithCheckpoint {
                    open_id: "open-99".to_string(),
                    claim: "settled".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "brute".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "n<=4".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                },
                3_000,
            )
            .unwrap_err();
        assert!(matches!(err, TaskLedgerError::UnknownOpenQuestion { .. }));
        // Close with invalid checkpoint payload still rejected.
        let err = ledger
            .apply(
                2,
                TaskLedgerOp::CloseOpenWithCheckpoint {
                    open_id: "open-01".to_string(),
                    claim: " ".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "brute".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "n<=4".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                },
                3_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyClaim);
        // Valid close links both sides and keeps the question in history.
        ledger
            .apply(
                2,
                TaskLedgerOp::CloseOpenWithCheckpoint {
                    open_id: "open-01".to_string(),
                    claim: "even case returns mean of middles".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "brute".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "n<=4".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                },
                3_000,
            )
            .unwrap();
        assert_eq!(ledger.verified.last().unwrap().id, "chk-01");
        assert_eq!(
            ledger.open[0].closed_by_checkpoint_id.as_deref(),
            Some("chk-01")
        );
        assert!(ledger.open_questions().is_empty());
        // Ids are never reused.
        ledger
            .apply(
                3,
                TaskLedgerOp::OpenQuestion {
                    question: "next".to_string(),
                    settled_by: "x".to_string(),
                },
                4_000,
            )
            .unwrap();
        assert_eq!(ledger.open[1].id, "open-02");
    }

    #[test]
    fn invariant_4_active_and_blocked_require_next() {
        let mut ledger = created();
        // Completed may clear next… The per-criterion gate names the first
        // uncovered criterion instead of a generic no-evidence error.
        let err = ledger
            .apply(1, TaskLedgerOp::Complete { uncovered: vec![] }, 2_000)
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::CriterionNotCovered {
                criterion: "all tests pass".to_string()
            }
        );
        ledger
            .apply(
                1,
                TaskLedgerOp::Complete {
                    uncovered: vec!["all tests pass".to_string()],
                },
                2_000,
            )
            .unwrap();
        assert!(ledger.next.is_none());
        assert_eq!(
            ledger.uncovered_criteria,
            vec!["all tests pass".to_string()]
        );
        // …but returning to active requires one.
        let err = ledger
            .apply(
                2,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::Active,
                    awaiting: None,
                    blocked_reason: None,
                },
                3_000,
            )
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::StatusRequiresNext {
                status: TaskLedgerStatus::Active
            }
        );
        // Blocked requires both next and a reason.
        ledger
            .apply_batch(
                2,
                vec![
                    TaskLedgerOp::SetNext {
                        statement: "wait for key".to_string(),
                        actor: None,
                    },
                    TaskLedgerOp::SetStatus {
                        status: TaskLedgerStatus::Blocked,
                        awaiting: None,
                        blocked_reason: Some("missing key".to_string()),
                    },
                ],
                3_000,
            )
            .unwrap();
        let err = ledger
            .apply(
                3,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::Blocked,
                    awaiting: None,
                    blocked_reason: None,
                },
                4_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyBlockedReason);
    }

    #[test]
    fn invariant_5_awaiting_user_requires_typed_interaction_and_next() {
        let mut ledger = created();
        let err = ledger
            .apply(
                1,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::AwaitingUser,
                    awaiting: None,
                    blocked_reason: None,
                },
                2_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::AwaitingRequiresInteraction);

        ledger
            .apply(
                1,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::AwaitingUser,
                    awaiting: Some(AwaitingInteractionRef {
                        kind: AwaitingInteractionKind::Permission,
                        interaction_id: "permission_abc".to_string(),
                    }),
                    blocked_reason: None,
                },
                2_000,
            )
            .unwrap();
        assert_eq!(ledger.status, TaskLedgerStatus::AwaitingUser);
        assert!(ledger.next.is_some(), "wait itself stays named");
        // A different concurrent interaction cannot silently replace it.
        // Multi-slot: a DIFFERENT concurrent interaction is tracked too —
        // both stay awaited, and only resolving BOTH leaves awaiting_user.
        ledger
            .apply(
                2,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::AwaitingUser,
                    awaiting: Some(AwaitingInteractionRef {
                        kind: AwaitingInteractionKind::Question,
                        interaction_id: "question_xyz".to_string(),
                    }),
                    blocked_reason: None,
                },
                3_000,
            )
            .unwrap();
        assert_eq!(ledger.awaiting_interactions.len(), 2);
        // Resolving the first leaves the second waited on.
        ledger
            .apply(
                3,
                TaskLedgerOp::ResolveInteraction {
                    kind: AwaitingInteractionKind::Permission,
                    interaction_id: "permission_abc".to_string(),
                },
                4_000,
            )
            .unwrap();
        assert_eq!(ledger.status, TaskLedgerStatus::AwaitingUser);
        assert_eq!(ledger.awaiting_interactions.len(), 1);
        // Resolving the last one resumes the run.
        ledger
            .apply(
                4,
                TaskLedgerOp::ResolveInteraction {
                    kind: AwaitingInteractionKind::Question,
                    interaction_id: "question_xyz".to_string(),
                },
                5_000,
            )
            .unwrap();
        assert_eq!(ledger.status, TaskLedgerStatus::Active);
        assert!(ledger.awaiting_interactions.is_empty());
    }

    #[test]
    fn invariant_6_interrupt_marks_pre_interrupt_next() {
        let mut ledger = created();
        ledger.apply(1, TaskLedgerOp::Interrupt, 2_000).unwrap();
        assert_eq!(ledger.status, TaskLedgerStatus::Interrupted);
        let next = ledger.next.clone().expect("pre-interrupt next retained");
        assert!(next.provenance.pre_interrupt);
        // A post-recovery next resets the marker.
        ledger
            .apply(
                2,
                TaskLedgerOp::SetNext {
                    statement: "resume from chk-01".to_string(),
                    actor: None,
                },
                3_000,
            )
            .unwrap();
        assert!(!ledger.next.as_ref().unwrap().provenance.pre_interrupt);
    }

    #[test]
    fn invariant_7_core_live_set_capped_at_two_with_swap() {
        let mut ledger = created();
        ledger
            .apply(
                1,
                TaskLedgerOp::AddCore {
                    statement: "preserve API".into(),
                    live: true,
                    actor: None,
                },
                2_000,
            )
            .unwrap();
        ledger
            .apply(
                2,
                TaskLedgerOp::AddCore {
                    statement: "stdlib only".into(),
                    live: true,
                    actor: None,
                },
                3_000,
            )
            .unwrap();
        let err = ledger
            .apply(
                3,
                TaskLedgerOp::AddCore {
                    statement: "third".into(),
                    live: true,
                    actor: None,
                },
                4_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::LiveSetFull);
        // Parked adds are fine.
        ledger
            .apply(
                3,
                TaskLedgerOp::AddCore {
                    statement: "third".into(),
                    live: false,
                    actor: None,
                },
                4_000,
            )
            .unwrap();
        // Swap slot 1 displaces the first live entry; displaced stays parked.
        ledger
            .apply(
                4,
                TaskLedgerOp::SwapCoreLive {
                    slot: 1,
                    statement: "third".into(),
                    actor: Some(TaskLedgerActor::User),
                },
                5_000,
            )
            .unwrap();
        let live: Vec<String> = ledger
            .live_core()
            .into_iter()
            .map(|entry| entry.statement.clone())
            .collect();
        assert_eq!(live, vec!["third".to_string(), "stdlib only".to_string()]);
        assert_eq!(ledger.live_core()[0].set_by, TaskLedgerActor::User);
        assert_eq!(ledger.live_core()[0].set_at, 5_000);
        assert_eq!(ledger.core.len(), 3, "history retained");
        let err = ledger
            .apply(
                5,
                TaskLedgerOp::SwapCoreLive {
                    slot: 3,
                    statement: "x".into(),
                    actor: None,
                },
                6_000,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            TaskLedgerError::LiveSlotOutOfBounds { slot: 3 }
        ));
    }

    #[test]
    fn invariant_8_every_write_is_cas() {
        let mut ledger = created();
        let err = ledger
            .apply(7, checkpoint_op("stale write"), 2_000)
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::RevisionConflict {
                expected: 7,
                actual: 1
            }
        );
        // Failed writes leave the ledger untouched.
        assert_eq!(ledger.revision, 1);
        assert!(ledger.verified.is_empty());
    }

    #[test]
    fn invariant_9_ledgers_are_per_session_values() {
        // The type carries the session id; there is no path-keyed global in
        // this module. Two ledgers with the same content stay independent.
        let mut left = created();
        let right = created();
        assert_eq!(left.session_id, right.session_id);
        left.apply(1, checkpoint_op("only left"), 2_000).unwrap();
        assert!(right.verified.is_empty());
    }

    #[test]
    fn create_requires_goal_and_next() {
        let mut ledger = SessionTaskLedger::empty("ses_x");
        let err = ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: goal("  "),
                    next_statement: "go".to_string(),
                },
                1_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyGoalStatement);
        let err = ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: goal("done means tests pass"),
                    next_statement: " ".to_string(),
                },
                1_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyStatement);
        // Ops before Create are rejected.
        let err = ledger
            .apply(
                0,
                TaskLedgerOp::SetNext {
                    statement: "x".into(),
                    actor: None,
                },
                1_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::NotCreated);
        // Second create rejected.
        ledger
            .apply(
                0,
                TaskLedgerOp::Create {
                    goal: goal("g"),
                    next_statement: "n".to_string(),
                },
                1_000,
            )
            .unwrap();
        let err = ledger
            .apply(
                1,
                TaskLedgerOp::Create {
                    goal: goal("g2"),
                    next_statement: "n2".to_string(),
                },
                2_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::AlreadyCreated);
    }

    #[test]
    fn apply_batch_is_atomic_with_single_revision_bump() {
        let mut ledger = created();
        let ops = vec![
            TaskLedgerOp::OpenQuestion {
                question: "q1".to_string(),
                settled_by: "t1".to_string(),
            },
            TaskLedgerOp::SetNext {
                statement: "resolve q1".to_string(),
                actor: Some(TaskLedgerActor::Model),
            },
            // Invalid op at the END: the whole batch must roll back.
            TaskLedgerOp::AddCheckpoint {
                claim: "bad".to_string(),
                verifier: VerifierRef::DeterministicCheck {
                    description: "x".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: " ".to_string(),
                },
                covered_criteria: vec![],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            },
        ];
        let err = ledger.apply_batch(1, ops, 9_000).unwrap_err();
        assert_eq!(err, TaskLedgerError::EmptyCoverageScope);
        assert_eq!(ledger.revision, 1, "rolled back entirely");
        assert!(ledger.open.is_empty());
        // Valid batch: one revision for all effects, model provenance kept.
        let ops = vec![
            TaskLedgerOp::OpenQuestion {
                question: "q1".to_string(),
                settled_by: "t1".to_string(),
            },
            TaskLedgerOp::SetNext {
                statement: "resolve q1".to_string(),
                actor: Some(TaskLedgerActor::Model),
            },
        ];
        ledger.apply_batch(1, ops, 9_000).unwrap();
        assert_eq!(ledger.revision, 2);
        assert_eq!(ledger.open.len(), 1);
        assert_eq!(
            ledger.next.unwrap().provenance.actor,
            TaskLedgerActor::Model
        );
    }

    #[test]
    fn completion_gate_blocks_open_questions_and_missing_evidence() {
        let mut ledger = created();
        ledger
            .apply(
                1,
                TaskLedgerOp::OpenQuestion {
                    question: "q".to_string(),
                    settled_by: "t".to_string(),
                },
                2_000,
            )
            .unwrap();
        let err = ledger
            .apply(2, TaskLedgerOp::Complete { uncovered: vec![] }, 3_000)
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::CompleteWithOpenQuestions { count: 1 });
        // SetStatus(Completed) hits the same gate — there is no side door.
        let err = ledger
            .apply(
                2,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::Completed,
                    awaiting: None,
                    blocked_reason: None,
                },
                3_000,
            )
            .unwrap_err();
        assert_eq!(err, TaskLedgerError::CompleteWithOpenQuestions { count: 1 });
        // Criteria goal without checkpoints names the uncovered criterion.
        let mut bare = created();
        let err = bare
            .apply(1, TaskLedgerOp::Complete { uncovered: vec![] }, 2_000)
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::CriterionNotCovered {
                criterion: "all tests pass".to_string()
            }
        );
        // An UNRELATED checkpoint covers nothing.
        let mut unrelated = created();
        unrelated
            .apply(1, checkpoint_op("something else verified"), 2_000)
            .unwrap();
        let err = unrelated
            .apply(2, TaskLedgerOp::Complete { uncovered: vec![] }, 3_000)
            .unwrap_err();
        assert_eq!(
            err,
            TaskLedgerError::CriterionNotCovered {
                criterion: "all tests pass".to_string()
            }
        );
        assert!(!completion_ready(&unrelated));
        // A checkpoint whose covered_criteria names the criterion completes.
        let mut done = created();
        done.apply(
            1,
            TaskLedgerOp::AddCheckpoint {
                claim: "all cases pass".to_string(),
                verifier: VerifierRef::DeterministicCheck {
                    description: "python3 -m unittest".to_string(),
                },
                coverage: VerificationCoverage {
                    scope: "3 cases".to_string(),
                },
                covered_criteria: vec!["all tests pass".to_string()],
                evidence_artifact_ids: vec![],
                source_stage_id: None,
                supersedes: None,
            },
            2_000,
        )
        .unwrap();
        done.apply(2, TaskLedgerOp::Complete { uncovered: vec![] }, 3_000)
            .unwrap();
        assert_eq!(done.status, TaskLedgerStatus::Completed);
        assert!(done.next.is_none());
        assert!(done.awaiting_interactions.is_empty());
        assert!(completion_ready(&done));
    }

    #[test]
    fn completion_ignores_superseded_and_prior_goal_evidence() {
        let mut superseded = created();
        superseded
            .apply(
                1,
                TaskLedgerOp::AddCheckpoint {
                    claim: "tests pass".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "cargo test".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "workspace".to_string(),
                    },
                    covered_criteria: vec!["all tests pass".to_string()],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: None,
                },
                2_000,
            )
            .unwrap();
        superseded
            .apply(
                2,
                TaskLedgerOp::AddCheckpoint {
                    claim: "earlier result invalidated".to_string(),
                    verifier: VerifierRef::Evaluator {
                        name: "review".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "test evidence audit".to_string(),
                    },
                    covered_criteria: vec![],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: Some("chk-01".to_string()),
                },
                3_000,
            )
            .unwrap();
        assert!(!completion_ready(&superseded));

        let mut replaced_goal = created();
        replaced_goal
            .apply(
                1,
                TaskLedgerOp::AddCheckpoint {
                    claim: "old goal tests pass".to_string(),
                    verifier: VerifierRef::DeterministicCheck {
                        description: "cargo test".to_string(),
                    },
                    coverage: VerificationCoverage {
                        scope: "old implementation".to_string(),
                    },
                    covered_criteria: vec!["all tests pass".to_string()],
                    evidence_artifact_ids: vec![],
                    source_stage_id: None,
                    supersedes: None,
                },
                2_000,
            )
            .unwrap();
        replaced_goal
            .apply(
                2,
                TaskLedgerOp::SetGoal {
                    goal: goal("new goal"),
                },
                3_000,
            )
            .unwrap();
        assert_eq!(replaced_goal.goal_generation, 2);
        assert!(!completion_ready(&replaced_goal));
    }

    #[test]
    fn completed_snapshot_can_only_change_through_a_valid_reopen_transaction() {
        let mut ledger = created();
        ledger
            .apply(
                1,
                TaskLedgerOp::Complete {
                    uncovered: vec!["all tests pass".to_string()],
                },
                2_000,
            )
            .unwrap();
        let completed = ledger.clone();
        assert!(ledger
            .apply(
                2,
                TaskLedgerOp::OpenQuestion {
                    question: "new doubt".to_string(),
                    settled_by: "new test".to_string(),
                },
                3_000,
            )
            .is_err());
        assert_eq!(ledger, completed, "failed mutation is atomic");
        assert!(ledger
            .apply(
                2,
                TaskLedgerOp::SetGoal {
                    goal: goal("replacement")
                },
                3_000
            )
            .is_err());
        assert_eq!(ledger, completed, "goal replacement also rolls back");

        ledger
            .apply_batch(
                2,
                vec![
                    TaskLedgerOp::SetGoal {
                        goal: goal("replacement"),
                    },
                    TaskLedgerOp::SetNext {
                        statement: "implement replacement".to_string(),
                        actor: Some(TaskLedgerActor::User),
                    },
                    TaskLedgerOp::SetStatus {
                        status: TaskLedgerStatus::Active,
                        awaiting: None,
                        blocked_reason: None,
                    },
                ],
                4_000,
            )
            .unwrap();
        assert_eq!(ledger.status, TaskLedgerStatus::Active);
        assert_eq!(ledger.goal_generation, 2);
        assert!(ledger.uncovered_criteria.is_empty());
    }

    #[test]
    fn pending_interactions_cannot_be_cleared_by_status_or_completion() {
        let mut ledger = created();
        for index in 0..9 {
            ledger
                .apply(
                    ledger.revision,
                    TaskLedgerOp::SetStatus {
                        status: TaskLedgerStatus::AwaitingUser,
                        awaiting: Some(AwaitingInteractionRef {
                            kind: AwaitingInteractionKind::Permission,
                            interaction_id: format!("perm-{index}"),
                        }),
                        blocked_reason: None,
                    },
                    2_000 + index,
                )
                .unwrap();
        }
        assert_eq!(ledger.awaiting_interactions.len(), 9);
        let revision = ledger.revision;
        let active_error = ledger
            .apply(
                revision,
                TaskLedgerOp::SetStatus {
                    status: TaskLedgerStatus::Active,
                    awaiting: None,
                    blocked_reason: None,
                },
                4_000,
            )
            .unwrap_err();
        assert!(matches!(
            active_error,
            TaskLedgerError::StatusConflictsWithAwaitingInteractions { .. }
        ));
        let complete_error = ledger
            .apply(
                revision,
                TaskLedgerOp::Complete {
                    uncovered: vec!["all tests pass".to_string()],
                },
                4_000,
            )
            .unwrap_err();
        assert_eq!(
            complete_error,
            TaskLedgerError::CompleteWithAwaitingInteractions { count: 9 }
        );
        assert_eq!(ledger.revision, revision);
        assert_eq!(ledger.awaiting_interactions.len(), 9);
    }

    #[test]
    fn ledger_roundtrips_through_serde() {
        let mut ledger = created();
        ledger.apply(1, checkpoint_op("c1"), 2_000).unwrap();
        let json = serde_json::to_string(&ledger).unwrap();
        let back: SessionTaskLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ledger);
    }
}
