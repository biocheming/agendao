use crate::scheduler::{SchedulerExecutionGateDecision, SchedulerExecutionGateStatus};

fn is_structured_sisyphus_delivery(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Execution Outcome**")
        && trimmed.contains("**Verification**")
}

pub(super) fn resolve_sisyphus_gate_terminal_content(
    status: SchedulerExecutionGateStatus,
    decision: &SchedulerExecutionGateDecision,
    fallback_content: &str,
) -> Option<String> {
    match status {
        SchedulerExecutionGateStatus::Done => {
            let fallback = fallback_content.trim();
            let final_response = decision
                .final_response
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());

            match final_response {
                Some(response)
                    if is_structured_sisyphus_delivery(fallback)
                        && !is_structured_sisyphus_delivery(response) =>
                {
                    Some(fallback.to_string())
                }
                Some(response) => Some(response.to_string()),
                None => (!fallback.is_empty()).then(|| fallback.to_string()),
            }
        }
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
    fn sisyphus_gate_terminal_content_prefers_final_response_for_done() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "verified".to_string(),
            next_input: None,
            final_response: Some("## Delivery Summary\nDone.".to_string()),
        };

        assert_eq!(
            resolve_sisyphus_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                "fallback execution output",
            ),
            Some("## Delivery Summary\nDone.".to_string())
        );
    }

    #[test]
    fn sisyphus_gate_terminal_content_preserves_structured_execution_output() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "verified".to_string(),
            next_input: None,
            final_response: Some(
                "所有任务已完成。如需深入了解其中任何一个方向的细节，请告诉我。".to_string(),
            ),
        };
        let fallback = "## Delivery Summary\nResearch complete.\n\n**Execution Outcome**\n# 基于 AlphaFold3 的新方法学研究综述\n\n- Protenix\n- Boltz-2\n\n**Verification**\n- Web search evidence collected.";

        assert_eq!(
            resolve_sisyphus_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                fallback,
            ),
            Some(fallback.to_string())
        );
    }

    #[test]
    fn sisyphus_gate_terminal_content_falls_back_to_execution_output_for_done() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "verified".to_string(),
            next_input: None,
            final_response: None,
        };

        assert_eq!(
            resolve_sisyphus_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                "shipped the change and verified the targeted behavior",
            ),
            Some("shipped the change and verified the targeted behavior".to_string())
        );
    }

    #[test]
    fn sisyphus_gate_terminal_content_uses_summary_for_blocked_without_final_response() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Blocked,
            summary: "blocked by missing external dependency".to_string(),
            next_input: None,
            final_response: None,
        };

        assert_eq!(
            resolve_sisyphus_gate_terminal_content(
                SchedulerExecutionGateStatus::Blocked,
                &decision,
                "fallback execution output",
            ),
            Some("blocked by missing external dependency".to_string())
        );
    }
}
