use crate::iterative_workflow::{ObjectiveDirection, VerifierTraceFormat};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VerifierTraceSnapshot {
    pub candidate_id: String,
    pub source_iteration: u32,
    pub trajectory_fingerprint: String,
    pub final_response: String,
    pub execution_steps: u32,
    pub execution_tool_calls: u32,
    pub execution_metadata_summary: Vec<String>,
    pub metric_value: Option<f64>,
    pub objective_direction: ObjectiveDirection,
    pub verify: VerifierCommandTrace,
    pub guard: Option<VerifierCommandTrace>,
    pub change_summary: Vec<String>,
    pub artifact_summary: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifierCommandTrace {
    pub name: &'static str,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub timed_out: bool,
    pub runtime_error: Option<String>,
    pub output: String,
}

impl VerifierTraceSnapshot {
    pub(crate) fn render(&self, trace_format: Option<VerifierTraceFormat>) -> String {
        render_verifier_trace(self, trace_format)
    }

    pub(crate) fn stable_fingerprint(&self) -> &str {
        &self.trajectory_fingerprint
    }
}

pub(crate) fn render_verifier_trace(
    snapshot: &VerifierTraceSnapshot,
    trace_format: Option<VerifierTraceFormat>,
) -> String {
    let format = trace_format.unwrap_or(VerifierTraceFormat::Compact);
    let output_limit = match format {
        VerifierTraceFormat::Compact => 2_400,
        VerifierTraceFormat::Full => 7_000,
    };
    let command_limit = match format {
        VerifierTraceFormat::Compact => 1_200,
        VerifierTraceFormat::Full => 4_000,
    };

    let mut sections = Vec::new();
    sections.push(format!(
        "format: {}\ncandidate_id: {}\nsource_iteration: {}\ntrajectory_fingerprint: {}\nmetric: {}\nobjective_direction: {}",
        trace_format_label(format),
        snapshot.candidate_id,
        snapshot.source_iteration,
        snapshot.trajectory_fingerprint,
        display_metric(snapshot.metric_value),
        objective_direction_label(snapshot.objective_direction)
    ));
    sections.push(format!(
        "execution_summary:\nsteps: {}\ntool_calls: {}",
        snapshot.execution_steps, snapshot.execution_tool_calls
    ));
    if !snapshot.execution_metadata_summary.is_empty() {
        sections.push(format!(
            "execution_metadata:\n{}",
            render_list(&snapshot.execution_metadata_summary, 24)
        ));
    }
    sections.push(render_command_trace(&snapshot.verify, command_limit));
    if let Some(guard) = snapshot.guard.as_ref() {
        sections.push(render_command_trace(guard, command_limit));
    } else {
        sections.push("guard: not configured".to_string());
    }
    if !snapshot.change_summary.is_empty() {
        sections.push(format!(
            "workspace_changes:\n{}",
            render_list(&snapshot.change_summary, 24)
        ));
    }
    if !snapshot.artifact_summary.is_empty() {
        sections.push(format!(
            "structured_artifacts:\n{}",
            render_list(&snapshot.artifact_summary, 24)
        ));
    }
    sections.push(format!(
        "final_response_chars: {}\nfinal_response:\n{}",
        snapshot.final_response.trim().chars().count(),
        trim_head_tail(&snapshot.final_response, output_limit)
    ));
    sections.join("\n\n")
}

pub(crate) fn trajectory_fingerprint(
    candidate_id: &str,
    source_iteration: u32,
    final_response: &str,
    execution_steps: u32,
    execution_tool_calls: u32,
    execution_metadata_summary: &[String],
    metric_value: Option<f64>,
    objective_direction: ObjectiveDirection,
    verify: &VerifierCommandTrace,
    guard: Option<&VerifierCommandTrace>,
    change_summary: &[String],
    artifact_summary: &[String],
) -> String {
    let mut hasher = DefaultHasher::new();
    candidate_id.hash(&mut hasher);
    source_iteration.hash(&mut hasher);
    final_response.hash(&mut hasher);
    execution_steps.hash(&mut hasher);
    execution_tool_calls.hash(&mut hasher);
    execution_metadata_summary.hash(&mut hasher);
    metric_value.map(f64::to_bits).hash(&mut hasher);
    objective_direction_label(objective_direction).hash(&mut hasher);
    hash_command_trace(verify, &mut hasher);
    if let Some(guard) = guard {
        hash_command_trace(guard, &mut hasher);
    }
    change_summary.hash(&mut hasher);
    artifact_summary.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn hash_command_trace(command: &VerifierCommandTrace, hasher: &mut DefaultHasher) {
    command.name.hash(hasher);
    command.exit_code.hash(hasher);
    command.passed.hash(hasher);
    command.timed_out.hash(hasher);
    command.runtime_error.hash(hasher);
    command.output.hash(hasher);
}

fn render_list(items: &[String], max_items: usize) -> String {
    let mut rendered = items
        .iter()
        .take(max_items)
        .map(|item| format!("- {}", trim_head_tail(item, 320)))
        .collect::<Vec<_>>();
    if items.len() > max_items {
        rendered.push(format!("- ...[{} more items]", items.len() - max_items));
    }
    rendered.join("\n")
}

fn render_command_trace(command: &VerifierCommandTrace, max_chars: usize) -> String {
    format!(
        "{} command:\npassed: {}\nexit_code: {}\ntimed_out: {}\nruntime_error: {}\noutput_chars: {}\noutput:\n{}",
        command.name,
        command.passed,
        command
            .exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
        command.timed_out,
        command
            .runtime_error
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("none"),
        command.output.trim().chars().count(),
        trim_head_tail(&command.output, max_chars)
    )
}

fn trace_format_label(format: VerifierTraceFormat) -> &'static str {
    match format {
        VerifierTraceFormat::Compact => "compact",
        VerifierTraceFormat::Full => "full",
    }
}

fn objective_direction_label(direction: ObjectiveDirection) -> &'static str {
    match direction {
        ObjectiveDirection::HigherIsBetter => "higher-is-better",
        ObjectiveDirection::LowerIsBetter => "lower-is-better",
    }
}

fn display_metric(metric_value: Option<f64>) -> String {
    metric_value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn trim_head_tail(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return trimmed.to_string();
    }

    let head_chars = (max_chars / 3).max(1);
    let tail_chars = max_chars.saturating_sub(head_chars).max(1);
    let head = trimmed.chars().take(head_chars).collect::<String>();
    let tail = trimmed
        .chars()
        .skip(char_count.saturating_sub(tail_chars))
        .collect::<String>();
    format!("{head}\n...[truncated middle]...\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(final_response: &str, verify_output: &str) -> VerifierTraceSnapshot {
        VerifierTraceSnapshot {
            candidate_id: "cand-001".to_string(),
            source_iteration: 1,
            trajectory_fingerprint: "trace-001".to_string(),
            final_response: final_response.to_string(),
            execution_steps: 3,
            execution_tool_calls: 1,
            execution_metadata_summary: vec!["finish_reason=end-turn".to_string()],
            metric_value: Some(12.0),
            objective_direction: ObjectiveDirection::HigherIsBetter,
            verify: VerifierCommandTrace {
                name: "verify",
                exit_code: Some(0),
                passed: true,
                timed_out: false,
                runtime_error: None,
                output: verify_output.to_string(),
            },
            guard: None,
            change_summary: Vec::new(),
            artifact_summary: Vec::new(),
        }
    }

    #[test]
    fn compact_trace_prioritizes_verify_evidence_before_final_response() {
        let rendered = render_verifier_trace(
            &trace("I claim success.", "cargo test passed with score=12"),
            Some(VerifierTraceFormat::Compact),
        );

        let verify_index = rendered.find("verify command").unwrap();
        let final_index = rendered.find("final_response").unwrap();
        assert!(verify_index < final_index);
        assert!(rendered.contains("cargo test passed with score=12"));
        assert!(rendered.contains("guard: not configured"));
        assert!(rendered.contains("execution_summary:"));
        assert!(rendered.contains("execution_metadata:"));
    }

    #[test]
    fn full_trace_keeps_more_output_than_compact_trace() {
        let long_output = "abcdef".repeat(1_000);
        let snapshot = trace(&long_output, &long_output);

        let compact = render_verifier_trace(&snapshot, Some(VerifierTraceFormat::Compact));
        let full = render_verifier_trace(&snapshot, Some(VerifierTraceFormat::Full));

        assert!(compact.contains("format: compact"));
        assert!(full.contains("format: full"));
        assert!(compact.contains("...[truncated middle]..."));
        assert!(full.len() > compact.len());
    }

    #[test]
    fn trace_renders_workspace_changes_and_structured_artifacts() {
        let mut snapshot = trace("candidate output", "cargo test passed");
        snapshot.change_summary = vec!["modified src/lib.rs (9 -> 14 bytes)".to_string()];
        snapshot.artifact_summary = vec![
            "candidate-registry:cand-001 status=kept title=Candidate detail=ready evidence=src/lib.rs"
                .to_string(),
        ];

        let rendered = render_verifier_trace(&snapshot, Some(VerifierTraceFormat::Compact));

        assert!(rendered.contains("workspace_changes:"));
        assert!(rendered.contains("modified src/lib.rs"));
        assert!(rendered.contains("structured_artifacts:"));
        assert!(rendered.contains("candidate-registry:cand-001"));
    }

    #[test]
    fn trajectory_fingerprint_changes_with_observed_command_output() {
        let left = trace("same response", "score=1");
        let right = trace("same response", "score=2");

        let left_fingerprint = trajectory_fingerprint(
            &left.candidate_id,
            left.source_iteration,
            &left.final_response,
            left.execution_steps,
            left.execution_tool_calls,
            &left.execution_metadata_summary,
            left.metric_value,
            left.objective_direction,
            &left.verify,
            left.guard.as_ref(),
            &left.change_summary,
            &left.artifact_summary,
        );
        let right_fingerprint = trajectory_fingerprint(
            &right.candidate_id,
            right.source_iteration,
            &right.final_response,
            right.execution_steps,
            right.execution_tool_calls,
            &right.execution_metadata_summary,
            right.metric_value,
            right.objective_direction,
            &right.verify,
            right.guard.as_ref(),
            &right.change_summary,
            &right.artifact_summary,
        );

        assert_ne!(left_fingerprint, right_fingerprint);
    }
}
