use crate::scheduler::{
    SchedulerDraftArtifactInput, SchedulerPlanningArtifactInput, StageToolConstraint,
    StageToolPolicy,
};
use crate::{ExecutionContext, ToolExecError};

use super::super::runtime_enforcement::{
    validate_runtime_artifact_path, validate_runtime_orchestration_tool, RuntimeArtifactPolicy,
};

pub const PROMETHEUS_PLANNING_STAGE_TOOLS: &[&str] = &[
    "read",
    "glob",
    "grep",
    "ls",
    "ast_grep_search",
    "question",
    "write",
    "edit",
];
pub const PROMETHEUS_RUNTIME_ORCHESTRATION_TOOLS: &[&str] = &["question", "todowrite"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrometheusArtifactKind {
    Planning,
    Draft,
}

pub fn prometheus_planning_stage_tool_policy() -> StageToolPolicy {
    StageToolPolicy::Restricted(StageToolConstraint::new(
        PROMETHEUS_PLANNING_STAGE_TOOLS,
        Some("prometheus-planning-artifacts"),
        Some(validate_prometheus_planning_tool_call),
    ))
}

pub struct PrometheusDraftContext<'a> {
    pub original_input: &'a str,
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub current_plan: Option<&'a str>,
    pub approval_review: Option<&'a str>,
    pub user_choice: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub draft_artifact_path: Option<&'a str>,
}

pub struct PrometheusPlanningArtifactContext<'a> {
    pub request_brief: &'a str,
    pub route_summary: Option<&'a str>,
    pub interview_output: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
    pub planning_output: &'a str,
    pub planning_artifact_path: Option<&'a str>,
}

pub fn build_prometheus_artifact_relative_path(
    kind: PrometheusArtifactKind,
    session_id: &str,
) -> String {
    let (directory, prefix) = match kind {
        PrometheusArtifactKind::Planning => (".sisyphus/plans", "plan"),
        PrometheusArtifactKind::Draft => (".sisyphus/drafts", "draft"),
    };
    let session_slug = slugify_artifact_component(session_id, 32);
    format!("{directory}/{prefix}-{session_slug}.md")
}

pub fn validate_prometheus_planning_tool_call(
    tool_name: &str,
    arguments: &serde_json::Value,
    exec_ctx: &ExecutionContext,
) -> Result<(), ToolExecError> {
    match tool_name.to_ascii_lowercase().as_str() {
        "write" | "edit" => {
            let raw_path = extract_tool_file_path(arguments).ok_or_else(|| {
                ToolExecError::InvalidArguments(format!(
                    "tool `{tool_name}` requires a file_path when used in Prometheus planning stages"
                ))
            })?;
            validate_prometheus_artifact_path(raw_path, exec_ctx)
        }
        _ => Ok(()),
    }
}

pub fn validate_prometheus_runtime_orchestration_tool(tool_name: &str) -> Result<(), String> {
    validate_runtime_orchestration_tool(
        "Prometheus planner",
        tool_name,
        PROMETHEUS_RUNTIME_ORCHESTRATION_TOOLS,
    )
}

pub fn validate_prometheus_runtime_artifact_path(
    raw_path: &str,
    exec_ctx: &ExecutionContext,
) -> Result<(), String> {
    validate_runtime_artifact_path(
        "Prometheus planner",
        raw_path,
        exec_ctx,
        RuntimeArtifactPolicy::MarkdownUnder(".sisyphus"),
    )
}

#[cfg(test)]
pub fn build_planning_artifact_relative_path(session_id: &str) -> String {
    build_prometheus_artifact_relative_path(PrometheusArtifactKind::Planning, session_id)
}

#[cfg(test)]
pub fn build_draft_artifact_relative_path(session_id: &str) -> String {
    build_prometheus_artifact_relative_path(PrometheusArtifactKind::Draft, session_id)
}

pub fn compose_prometheus_planning_artifact(input: SchedulerPlanningArtifactInput<'_>) -> String {
    render_prometheus_plan_artifact(PrometheusPlanningArtifactContext {
        request_brief: input.request_brief,
        route_summary: input.route_summary,
        interview_output: input.interview_output,
        advisory_review: input.advisory_review,
        planning_output: input.planning_output,
        planning_artifact_path: input.planning_artifact_path,
    })
}

fn extract_tool_file_path(arguments: &serde_json::Value) -> Option<&str> {
    arguments
        .get("file_path")
        .or_else(|| arguments.get("filePath"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn validate_prometheus_artifact_path(
    raw_path: &str,
    exec_ctx: &ExecutionContext,
) -> Result<(), ToolExecError> {
    validate_prometheus_runtime_artifact_path(raw_path, exec_ctx)
        .map_err(ToolExecError::PermissionDenied)
}

pub fn compose_prometheus_draft_artifact(input: SchedulerDraftArtifactInput<'_>) -> String {
    render_prometheus_draft(PrometheusDraftContext {
        original_input: input.original_request,
        request_brief: input.request_brief,
        route_summary: input.route_summary,
        interview_output: input.interview_output,
        advisory_review: input.advisory_review,
        current_plan: input.current_plan,
        approval_review: input.approval_review,
        user_choice: input.user_choice,
        planning_artifact_path: input.planning_artifact_path,
        draft_artifact_path: input.draft_artifact_path,
    })
}

pub fn render_prometheus_plan_artifact(context: PrometheusPlanningArtifactContext<'_>) -> String {
    let planning_output = context.planning_output.trim();
    if planning_output.is_empty() {
        return String::new();
    }
    if prometheus_plan_artifact_has_omo_shape(planning_output) {
        return planning_output.to_string();
    }

    let title = artifact_title(
        context.planning_artifact_path,
        planning_output,
        "prometheus-plan",
    );
    let deliverables = extract_markdown_list_items(planning_output, 3);
    let deliverables = if deliverables.is_empty() {
        vec![
            "Produce a single reviewed execution plan artifact.".to_string(),
            "Preserve planner-only handoff to Atlas via `/start-work`.".to_string(),
        ]
    } else {
        deliverables
    };
    let interview_summary = context
        .interview_output
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(context.request_brief);
    let metis_items = context
        .advisory_review
        .map(|text| extract_markdown_list_items(text, 6))
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            vec!["No explicit Metis guardrails were captured in this pass.".to_string()]
        });
    let task_titles = extract_plan_task_titles(planning_output, &deliverables);
    let parallel_waves = render_prometheus_parallel_waves(&task_titles);
    let dependency_matrix = render_prometheus_dependency_matrix(&task_titles);
    let agent_dispatch_summary = render_prometheus_agent_dispatch_summary(&task_titles);
    let todo_templates = render_prometheus_todo_templates(&task_titles);

    let todos_body = if looks_like_task_breakdown(planning_output) {
        format!("{}\n\n{}", planning_output, todo_templates)
    } else {
        format!(
            "- [ ] Refine the generated planning body into concrete execution tasks.

### Generated Plan Body
{}

{}",
            planning_output, todo_templates
        )
    };

    [
        format!("# {title}"),
        "## TL;DR

> **Quick Summary**: Prometheus generated a planner-only work plan for the request below.
>
> **Deliverables**:".to_string(),
        deliverables
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("
"),
        ">
> **Estimated Effort**: TBD
> **Parallel Execution**: TBD
> **Critical Path**: To be finalized by the task breakdown.".to_string(),
        "---".to_string(),
        format!("## Context

### Request Brief
{}

### Interview Summary
{}

### Metis Review
{}",
            context.request_brief,
            interview_summary,
            metis_items.iter().map(|item| format!("- {item}")).collect::<Vec<_>>().join("
")
        ),
        format!("## Work Objectives

### Core Objective
{}

### Concrete Deliverables
{}

### Definition of Done
- [ ] Plan saved as markdown under `.sisyphus/plans/*.md`
- [ ] Tasks include concrete acceptance criteria
- [ ] Tasks include Agent-Executed QA Scenarios

### Must Have
- Preserve Prometheus as planner-only
- Keep one consolidated work plan

### Must NOT Have (Guardrails)
- No claims that implementation is already complete
- No non-markdown file edits in this workflow
{}",
            context.request_brief,
            deliverables.iter().map(|item| format!("- {item}")).collect::<Vec<_>>().join("
"),
            context.route_summary.map(|summary| format!("- {summary}")).unwrap_or_default(),
        ),
        "## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.
> Acceptance criteria requiring manual user confirmation are forbidden.

### Test Decision
- **Infrastructure exists**: Determine from repo exploration
- **Automated tests**: TDD / tests-after / none, but still include agent QA
- **Framework**: Use the repo's actual test harness

### QA Policy
Every task must include agent-executed QA scenarios and evidence paths under `.sisyphus/evidence/`.".to_string(),
        format!("## Execution Strategy

### Parallel Execution Waves
{parallel_waves}

### Dependency Matrix
{dependency_matrix}

### Agent Dispatch Summary
{agent_dispatch_summary}"),
        format!("## TODOs

{todos_body}"),
        "## Final Verification Wave
- Plan compliance audit
- Scope fidelity check
- Execution readiness review".to_string(),
        "## Commit Strategy
- Decide during execution; do not fabricate commit details during planning.".to_string(),
        "## Success Criteria
- The plan is concrete, bounded, and execution-ready
- Remaining decisions are explicit
- `/start-work` can hand this artifact to Atlas without re-interviewing the user".to_string(),
    ]
    .join("

")
}

pub fn render_prometheus_draft(context: PrometheusDraftContext<'_>) -> String {
    let title = artifact_title(
        context
            .planning_artifact_path
            .or(context.draft_artifact_path),
        context.original_input,
        "prometheus-session",
    );

    let mut requirements = vec![format!(
        "Original request: {}",
        context.original_input.trim()
    )];
    if !context.request_brief.trim().is_empty() {
        requirements.push(format!("Request brief: {}", context.request_brief.trim()));
    }
    requirements.extend(extract_markdown_list_items(
        context.interview_output.unwrap_or_default(),
        4,
    ));
    dedup_preserve(&mut requirements);

    let mut technical_decisions = Vec::new();
    if let Some(route_summary) = context
        .route_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        technical_decisions.push(route_summary.to_string());
    }
    if context
        .current_plan
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        technical_decisions.push("A planning snapshot exists and should remain the single working plan candidate until handoff.".to_string());
    }
    if let Some(choice) = context
        .user_choice
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        technical_decisions.push(format!("Current handoff preference: {choice}"));
    }
    if technical_decisions.is_empty() {
        technical_decisions.push("Prometheus remains in planner-only mode.".to_string());
    }

    let mut research_findings = context
        .advisory_review
        .map(|text| extract_markdown_list_items(text, 6))
        .unwrap_or_default();
    if research_findings.is_empty() {
        research_findings.push("No external research findings recorded yet.".to_string());
    }

    let mut open_questions = collect_decision_needed_lines(&[
        context.interview_output,
        context.current_plan,
        context.approval_review,
    ]);
    if open_questions.is_empty() {
        open_questions.push("None recorded yet.".to_string());
    }

    let include_scope = context.request_brief.trim();
    let mut sections = Vec::new();
    sections.push(format!("# Draft: {title}"));
    sections.push(format_markdown_list_section(
        "Requirements (confirmed)",
        &requirements,
    ));
    sections.push(format_markdown_list_section(
        "Technical Decisions",
        &technical_decisions,
    ));
    sections.push(format_markdown_list_section(
        "Research Findings",
        &research_findings,
    ));
    sections.push(format_markdown_list_section(
        "Open Questions",
        &open_questions,
    ));
    sections.push(format!(
        "## Scope Boundaries
- INCLUDE: {}
- EXCLUDE: Code execution or implementation claims inside the Prometheus workflow.",
        if include_scope.is_empty() {
            "Scope still being clarified."
        } else {
            include_scope
        }
    ));
    if let Some(path) = context
        .planning_artifact_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "## Planning Artifact
- Target plan path: `{path}`"
        ));
    }
    sections.join(
        "

",
    )
}

pub fn append_artifact_note(content: String, artifact_path: Option<&str>) -> String {
    let Some(artifact_path) = artifact_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return content;
    };

    if content.contains(artifact_path) {
        return content;
    }

    let trimmed = content.trim_end();
    if trimmed.is_empty() {
        format!("Plan saved to: `{artifact_path}`")
    } else {
        format!(
            "{trimmed}

Plan saved to: `{artifact_path}`"
        )
    }
}

pub fn slugify_artifact_component(input: &str, max_len: usize) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            last_was_dash = false;
        } else if !slug.is_empty() && !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }

        if slug.len() >= max_len {
            break;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "plan".to_string()
    } else {
        slug
    }
}

fn artifact_title(path_hint: Option<&str>, fallback_source: &str, default_title: &str) -> String {
    path_hint
        .and_then(|path| std::path::Path::new(path).file_stem())
        .and_then(|value| value.to_str())
        .or_else(|| first_markdown_heading(fallback_source))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .trim_start_matches("plan-")
                .trim_start_matches("draft-")
                .to_string()
        })
        .unwrap_or_else(|| default_title.to_string())
}

fn first_markdown_heading(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("# "))
        .map(|line| line.trim_start_matches("# ").trim())
}

fn prometheus_plan_artifact_has_omo_shape(text: &str) -> bool {
    [
        "## TL;DR",
        "## Context",
        "## Work Objectives",
        "## Verification Strategy",
        "## Execution Strategy",
        "## TODOs",
        "## Success Criteria",
    ]
    .iter()
    .all(|heading| text.contains(heading))
}

fn extract_plan_task_titles(planning_output: &str, deliverables: &[String]) -> Vec<String> {
    let mut tasks = extract_markdown_list_items(planning_output, 8);
    if tasks.is_empty() {
        tasks = deliverables.to_vec();
    }
    dedup_preserve(&mut tasks);
    if tasks.is_empty() {
        vec!["Refine the Prometheus execution plan into bounded tasks.".to_string()]
    } else {
        tasks
    }
}

fn render_prometheus_parallel_waves(task_titles: &[String]) -> String {
    if task_titles.is_empty() {
        return "- Wave 1: Refine the task graph before execution.".to_string();
    }

    task_titles
        .chunks(3)
        .enumerate()
        .map(|(index, chunk)| {
            let wave = index + 1;
            let label = if wave == 1 {
                format!("Wave {wave} (Start Immediately)")
            } else {
                format!("Wave {wave} (After Wave {})", wave - 1)
            };
            let items = chunk
                .iter()
                .enumerate()
                .map(|(offset, title)| {
                    format!(
                        "├── Task {}: {} [assign category + dependencies from repo evidence]",
                        index * 3 + offset + 1,
                        title
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("{label}:\n{items}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_prometheus_dependency_matrix(task_titles: &[String]) -> String {
    task_titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            format!(
                "- **Task {}** `{}` — Blocked By: determine from repo evidence; Blocks: determine after task graph review; Wave: assign during final plan drafting",
                index + 1,
                title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_prometheus_agent_dispatch_summary(task_titles: &[String]) -> String {
    task_titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            format!(
                "- **Task {}** `{}` — Category: choose from available execution resources; Skills: justify explicitly; Verification owner: assign agent-executed QA evidence path",
                index + 1,
                title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_prometheus_todo_templates(task_titles: &[String]) -> String {
    task_titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            format!(
                "- [ ] {}. {}\n\n  **What to do**:\n  - Implement the bounded scope for this task.\n  - Add or update verification that proves the behavior.\n\n  **Must NOT do**:\n  - Do not expand beyond the stated task boundary.\n  - Do not skip agent-executed QA scenarios.\n\n  **Recommended Agent Profile**:\n  - **Category**: `[choose category from available execution resources]`\n    - Reason: explain why this category fits the task domain.\n  - **Skills**: `[skill-1, skill-2]`\n    - `skill-1`: explain the domain overlap.\n    - `skill-2`: explain the domain overlap.\n  - **Skills Evaluated but Omitted**:\n    - `omitted-skill`: explain why it was rejected.\n\n  **Parallelization**:\n  - **Can Run In Parallel**: YES | NO\n  - **Parallel Group**: Wave N | Sequential\n  - **Blocks**: list downstream tasks explicitly\n  - **Blocked By**: list upstream tasks explicitly or `None`\n\n  **References** (CRITICAL - Be Exhaustive):\n  - **Pattern References**: cite exact files / symbols to follow and explain why.\n  - **API/Type References**: cite contracts to implement against and explain why.\n  - **Test References**: cite existing test patterns and explain why.\n  - **External References**: cite docs only when they materially affect implementation.\n  - **WHY Each Reference Matters**: do not just list files; explain the extracted pattern.\n\n  **Acceptance Criteria**:\n  - [ ] A concrete command, tool run, or artifact can prove completion.\n  - [ ] No criterion requires human intervention.\n\n  **QA Scenarios (MANDATORY)**:\n  ```\n  Scenario: Happy path — {}\n    Tool: [Playwright / interactive_bash / Bash (curl) / unit test command]\n    Preconditions: [Exact setup state]\n    Steps:\n      1. [Exact action]\n      2. [Exact assertion]\n    Expected Result: [Binary pass/fail result]\n    Failure Indicators: [Concrete failure signal]\n    Evidence: .sisyphus/evidence/task-{}-happy.ext\n\n  Scenario: Failure/edge case — {}\n    Tool: [same format]\n    Preconditions: [Invalid input / error state]\n    Steps:\n      1. [Trigger failure condition]\n      2. [Assert graceful handling]\n    Expected Result: [Correct error or safe behavior]\n    Evidence: .sisyphus/evidence/task-{}-error.ext\n  ```",
                index + 1,
                title,
                title,
                index + 1,
                title,
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn looks_like_task_breakdown(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("- [ ]")
            || line.starts_with("- ")
            || line
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
    })
}

fn extract_markdown_list_items(text: &str, max: usize) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("- ")
                || line.starts_with("* ")
                || line
                    .chars()
                    .next()
                    .map(|ch| ch.is_ascii_digit())
                    .unwrap_or(false)
        })
        .map(|line| {
            line.trim_start_matches(|ch: char| {
                ch == '-'
                    || ch == '*'
                    || ch.is_ascii_digit()
                    || ch == '.'
                    || ch == ' '
                    || ch == '['
                    || ch == ']'
            })
            .trim()
            .to_string()
        })
        .filter(|line| !line.is_empty())
        .take(max)
        .collect()
}

fn collect_decision_needed_lines(sources: &[Option<&str>]) -> Vec<String> {
    let mut items = Vec::new();
    for source in sources.iter().flatten() {
        for line in source.lines().map(str::trim) {
            let lower = line.to_ascii_lowercase();
            if lower.contains("[decision needed:") || lower.contains("decision needed") {
                items.push(line.to_string());
            }
        }
    }
    dedup_preserve(&mut items);
    items
}

fn format_markdown_list_section(title: &str, items: &[String]) -> String {
    let body = if items.is_empty() {
        "- None.".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    };
    format!(
        "## {title}
{body}"
    )
}

fn dedup_preserve(items: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| seen.insert(item.clone()));
}
