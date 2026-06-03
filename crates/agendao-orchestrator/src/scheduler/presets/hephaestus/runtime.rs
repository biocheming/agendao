use crate::scheduler::{SchedulerExecutionGateDecision, SchedulerExecutionGateStatus};

pub(super) fn resolve_hephaestus_gate_terminal_content(
    status: SchedulerExecutionGateStatus,
    decision: &SchedulerExecutionGateDecision,
    fallback_content: &str,
) -> Option<String> {
    match status {
        SchedulerExecutionGateStatus::Done => decision
            .final_response
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let trimmed = fallback_content.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }),
        SchedulerExecutionGateStatus::Blocked => {
            let blocked = decision
                .final_response
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| decision.summary.clone());
            (!blocked.trim().is_empty()).then_some(blocked)
        }
        SchedulerExecutionGateStatus::Continue => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hephaestus_gate_terminal_content_prefers_final_response_for_done() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "verified".to_string(),
            next_input: None,
            final_response: Some("## Delivery Summary\nDone.".to_string()),
        };

        assert_eq!(
            resolve_hephaestus_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                "fallback execution output",
            ),
            Some("## Delivery Summary\nDone.".to_string())
        );
    }

    #[test]
    fn hephaestus_gate_terminal_content_falls_back_to_execution_output_for_done() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "verified".to_string(),
            next_input: None,
            final_response: None,
        };

        assert_eq!(
            resolve_hephaestus_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                "fixed the diagnostics path and ran the targeted check",
            ),
            Some("fixed the diagnostics path and ran the targeted check".to_string())
        );
    }

    #[test]
    fn hephaestus_gate_terminal_content_uses_summary_for_blocked_without_final_response() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Blocked,
            summary: "missing external credential".to_string(),
            next_input: None,
            final_response: None,
        };

        assert_eq!(
            resolve_hephaestus_gate_terminal_content(
                SchedulerExecutionGateStatus::Blocked,
                &decision,
                "fallback execution output",
            ),
            Some("missing external credential".to_string())
        );
    }
}
