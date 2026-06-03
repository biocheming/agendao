pub(super) const SHARED_EXECUTION_EVIDENCE_CONTRACT: &str = "## Evidence Contract\n\
- Separate claimed work from proved work.\n\
- Keep concrete verification evidence in the same response as completion claims.\n\
- If something remains uncertain, say so explicitly instead of implying completion.\n\
- Do not upgrade a worker claim into scheduler truth without evidence.";

pub(super) const SHARED_VERIFICATION_EVIDENCE_CONTRACT: &str = "## Evidence Rules\n\
- Verify against the actual request boundary, not just internal progress.\n\
- Prefer observed artifacts, diagnostics, tests, and file state over confidence claims.\n\
- Name missing evidence, conflicting evidence, and blockers explicitly.";

pub(super) const SHARED_GATE_DECISION_CONTRACT: &str = "## Gate Evidence Rules\n\
- `done` requires evidence-backed completion at the active boundary.\n\
- `continue` requires one bounded retry focus with a concrete missing proof, unfinished task, or fix.\n\
- `blocked` requires a real blocker, not just low confidence or unfinished thinking.\n\
- `final_response`, when present, must stay faithful to execution and verification evidence.";

pub(super) const SHARED_RETRY_RECOVERY_CONTRACT: &str = "## Retry Recovery Rules\n\
- Retry only the specific unresolved gap named by the gate.\n\
- Preserve prior constraints, verified progress, and continuation authority.\n\
- Treat a retry as a continuation, not a fresh rediscovery pass.\n\
- State the concrete evidence still needed before the next completion claim.";

pub(super) fn normalize_retry_focus(summary: &str, next_input: Option<&str>) -> String {
    next_input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| summary.trim())
        .to_string()
}
