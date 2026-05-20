use crate::runtime::events::{
    RequestViewMutationKind, StepCheckpointDirective, StepCheckpointSnapshot, StepUsage,
};

// ---------------------------------------------------------------------------
// LoopPolicy – controls retry, dedup, and error handling behavior.
// Passed to run_loop to configure the execution semantics.
// ---------------------------------------------------------------------------

/// Policy configuration for a single `run_loop` invocation.
#[derive(Debug, Clone)]
pub struct LoopPolicy {
    /// Maximum number of agentic steps (model calls) before stopping.
    /// `None` means no scheduler-imposed step cap.
    pub max_steps: Option<u32>,

    /// Scope for tool_call_id deduplication.
    pub tool_dedup: ToolDedupScope,

    /// How to handle tool execution errors.
    pub on_tool_error: ToolErrorStrategy,

    /// Default in-flight request-view checkpoint governance owned by the
    /// runtime loop. Outer hooks may observe or override it, but they are not
    /// the primary decision owner.
    pub checkpoint_governance: CheckpointGovernancePolicy,

    /// P3-F: Maximum time to wait for the next stream event before treating
    /// the stream as hung. `None` disables the watchdog (legacy behavior).
    /// Typical value: 30_000ms (30 seconds).
    pub stream_event_timeout_ms: Option<u64>,
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            max_steps: Some(100),
            tool_dedup: ToolDedupScope::Global,
            on_tool_error: ToolErrorStrategy::ReportAndContinue,
            checkpoint_governance: CheckpointGovernancePolicy::default(),
            stream_event_timeout_ms: Some(60_000),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelContextLimits {
    pub context_window_tokens: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

impl ModelContextLimits {
    pub fn from_model_info(info: &rocode_provider::ModelInfo) -> Self {
        Self {
            context_window_tokens: Some(info.context_window),
            max_input_tokens: info.max_input_tokens,
            max_output_tokens: Some(info.max_output_tokens),
        }
    }

    pub fn heuristic_for_model_id(model_id: &str) -> Self {
        let (default_max_output_tokens, _) = rocode_provider::models::default_model_limits();
        Self {
            context_window_tokens: Some(rocode_provider::models::get_model_context_limit(model_id)),
            max_input_tokens: None,
            max_output_tokens: Some(default_max_output_tokens),
        }
    }

    pub fn request_limit_tokens(self) -> Option<u64> {
        self.max_input_tokens.or_else(|| {
            self.context_window_tokens
                .map(|window| window.saturating_sub(self.max_output_tokens.unwrap_or_default()))
        })
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointGovernancePolicy {
    pub enabled: bool,
    pub threshold_percent: u64,
    pub critical_percent: u64,
    pub max_assessments: usize,
    pub min_compactable_messages: usize,
}

impl Default for CheckpointGovernancePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_percent: rocode_types::CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT,
            critical_percent: rocode_types::CONTEXT_PRESSURE_CRITICAL_PERCENT,
            max_assessments: 2,
            min_compactable_messages: 10,
        }
    }
}

impl CheckpointGovernancePolicy {
    pub fn default_directive(
        &self,
        model_limits: Option<ModelContextLimits>,
        usage: Option<&StepUsage>,
        checkpoint: &StepCheckpointSnapshot,
    ) -> StepCheckpointDirective {
        if !self.enabled {
            return StepCheckpointDirective::Continue;
        }

        let request_limit_tokens = model_limits.and_then(ModelContextLimits::request_limit_tokens);
        let request_context_tokens = checkpoint
            .current_view
            .estimated_context_tokens
            .or_else(|| usage.and_then(default_usage_request_tokens));

        let request_pressure_percent = request_context_tokens
            .zip(request_limit_tokens)
            .and_then(|(used, limit)| context_usage_percent(used, limit));
        let overflow = request_context_tokens
            .zip(request_limit_tokens)
            .is_some_and(|(used, limit)| used > limit);
        let critical = overflow
            || request_pressure_percent
                .map(|percent| percent >= self.critical_percent)
                .unwrap_or(false);
        let over_threshold = critical
            || request_pressure_percent
                .map(|percent| percent >= self.threshold_percent)
                .unwrap_or(false);

        let compactable_history =
            checkpoint.current_view.compactable_messages >= self.min_compactable_messages;
        let can_attempt_rewrite = checkpoint.remaining_assessments() > 0 && compactable_history;

        if !checkpoint.rewrite_attempted() {
            if over_threshold && can_attempt_rewrite {
                return StepCheckpointDirective::CompactRequestView {
                    focus: None,
                    reason: Some(if critical {
                        "request_view_overflow".to_string()
                    } else {
                        "request_view_threshold".to_string()
                    }),
                };
            }
            if critical {
                return StepCheckpointDirective::Block {
                    reason: checkpoint_block_reason("request_view_overflow"),
                };
            }
            return StepCheckpointDirective::Continue;
        }

        if critical {
            let reason = checkpoint
                .prior_mutations
                .iter()
                .rev()
                .find_map(|mutation| {
                    matches!(mutation.kind, RequestViewMutationKind::Compacted)
                        .then_some("request_view_overflow_after_compaction")
                })
                .unwrap_or("request_view_overflow");
            return StepCheckpointDirective::Block {
                reason: checkpoint_block_reason(reason),
            };
        }

        StepCheckpointDirective::Continue
    }
}

fn context_usage_percent(used: u64, limit: u64) -> Option<u64> {
    (limit > 0).then_some(used.saturating_mul(100) / limit)
}

fn default_usage_request_tokens(usage: &StepUsage) -> Option<u64> {
    Some(usage.context_tokens.max(usage.prompt_tokens)).filter(|tokens| *tokens > 0)
}

fn checkpoint_block_reason(reason: &str) -> String {
    format!("runtime checkpoint blocked the next model call ({reason})")
}

/// Scope of tool_call_id deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDedupScope {
    /// Dedup across the entire `run_loop` invocation (default).
    /// A tool_call_id seen in any step will not be dispatched again.
    Global,

    /// Only dedup within a single step.
    /// The same tool_call_id in different steps will be dispatched.
    PerStep,

    /// No dedup at all.
    None,
}

/// Strategy for handling tool execution errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorStrategy {
    /// Fail the entire loop on the first tool error.
    Fail,

    /// Skip the failed tool call and continue. A synthetic error result is
    /// added to the conversation so the model sees a complete tool response.
    Skip,

    /// Report the error as a tool result and continue (default).
    /// The model sees the error message and can decide how to proceed.
    ReportAndContinue,
}
