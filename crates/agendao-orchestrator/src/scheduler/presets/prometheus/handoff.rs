use serde_json::json;

pub fn plan_start_work_command(plan_path: Option<&str>) -> String {
    let Some(plan_path) = plan_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return "/start-work".to_string();
    };
    std::path::Path::new(plan_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|name| format!("/start-work {name}"))
        .unwrap_or_else(|| "/start-work".to_string())
}

pub fn prometheus_workflow_todos_payload() -> serde_json::Value {
    json!({
        "todos": [
            { "id": "plan-1", "content": "Consult Metis for gap analysis (auto-proceed)", "status": "pending", "priority": "high" },
            { "id": "plan-2", "content": "Generate work plan to .sisyphus/plans/{name}.md", "status": "pending", "priority": "high" },
            { "id": "plan-3", "content": "Self-review: classify gaps (critical/minor/ambiguous)", "status": "pending", "priority": "high" },
            { "id": "plan-4", "content": "Present summary with auto-resolved items and decisions needed", "status": "pending", "priority": "high" },
            { "id": "plan-5", "content": "If decisions needed: wait for user, update plan", "status": "pending", "priority": "high" },
            { "id": "plan-6", "content": "Ask user about high accuracy mode (Momus review)", "status": "pending", "priority": "high" },
            { "id": "plan-7", "content": "If high accuracy: submit to Momus and iterate until OKAY", "status": "pending", "priority": "medium" },
            { "id": "plan-8", "content": "Delete draft file and hand the reviewed plan to Atlas via /start-work {name}", "status": "pending", "priority": "medium" }
        ]
    })
}

pub fn prometheus_approval_review_is_accepted(output: &str) -> bool {
    let upper = output.trim().to_ascii_uppercase();
    upper.starts_with("[OKAY]") || upper.starts_with("OKAY")
}

pub fn append_handoff_guidance(
    content: String,
    plan_path: Option<&str>,
    draft_path: Option<&str>,
    draft_deleted: bool,
    recommend_start_work: bool,
    high_accuracy_approved: Option<bool>,
) -> String {
    let suggested_start_work = plan_start_work_command(plan_path);
    let trimmed = content.trim();

    if prometheus_handoff_output_has_required_shape(trimmed) {
        let notes = build_handoff_notes(
            plan_path,
            draft_path,
            draft_deleted,
            recommend_start_work,
            high_accuracy_approved,
            &suggested_start_work,
        );
        if notes.is_empty() {
            return trimmed.to_string();
        }
        return format!(
            "{trimmed}

{}",
            notes.join(
                "
"
            )
        );
    }

    let mut plan_summary = extract_handoff_summary_lines(trimmed);
    if plan_summary.is_empty() {
        plan_summary.push("Reviewed planning handoff prepared for Prometheus.".to_string());
    }

    let recommended_next_step = match high_accuracy_approved {
        Some(false) => vec![
            "High Accuracy Review is still blocked by Momus feedback.".to_string(),
            "Revise the plan and re-run the review loop before execution.".to_string(),
            "Do not run `/start-work` yet.".to_string(),
        ],
        _ if recommend_start_work => vec![
            format!("Run `{suggested_start_work}` to hand the reviewed plan to Atlas."),
            "This switches the execution workflow into Atlas, registers the plan as the active boulder, tracks progress across sessions, and enables continuation if interrupted.".to_string(),
        ],
        _ => vec![
            "Review the plan and choose the next step when ready.".to_string(),
            "If high accuracy is required, keep iterating with Momus until approval.".to_string(),
        ],
    };

    let mut remaining_risks = collect_decision_needed_lines(trimmed);
    if high_accuracy_approved == Some(false) {
        remaining_risks.push("Momus has not yet approved the plan.".to_string());
    }
    dedup_preserve(&mut remaining_risks);
    if remaining_risks.is_empty() {
        remaining_risks.push("None.".to_string());
    }

    let mut execution_status = vec![
        "Code execution has not been performed in this workflow.".to_string(),
        "Prometheus remains planner-only; `/start-work` hands the reviewed plan to Atlas for execution orchestration.".to_string(),
    ];
    if let Some(plan_path) = plan_path.map(str::trim).filter(|value| !value.is_empty()) {
        execution_status.push(format!("Plan saved to: `{plan_path}`"));
    }
    if draft_deleted {
        if let Some(draft_path) = draft_path.map(str::trim).filter(|value| !value.is_empty()) {
            execution_status.push(format!("Draft cleaned up: `{draft_path}`"));
        }
    }
    match high_accuracy_approved {
        Some(true) => execution_status.push("High Accuracy Review: approved by Momus.".to_string()),
        Some(false) => execution_status
            .push("High Accuracy Review: still blocked by Momus feedback.".to_string()),
        None => {}
    }

    [
        format_markdown_list_section("Plan Summary", &plan_summary, true),
        format_markdown_list_section("Recommended Next Step", &recommended_next_step, false),
        format_markdown_list_section("Remaining Decisions or Risks", &remaining_risks, false),
        format_markdown_list_section("Execution Status", &execution_status, false),
    ]
    .join(
        "

",
    )
}

fn build_handoff_notes(
    plan_path: Option<&str>,
    draft_path: Option<&str>,
    draft_deleted: bool,
    recommend_start_work: bool,
    high_accuracy_approved: Option<bool>,
    suggested_start_work: &str,
) -> Vec<String> {
    let mut notes = Vec::new();

    if let Some(plan_path) = plan_path.filter(|value| !value.trim().is_empty()) {
        notes.push(format!("Plan saved to: `{plan_path}`"));
    }

    if draft_deleted {
        if let Some(draft_path) = draft_path.filter(|value| !value.trim().is_empty()) {
            notes.push(format!("Draft cleaned up: `{draft_path}`"));
        }
    }

    match high_accuracy_approved {
        Some(true) => notes.push("High Accuracy Review: approved by Momus.".to_string()),
        Some(false) => notes.push("High Accuracy Review: still blocked; review the Momus feedback before execution. Do not run `/start-work` yet.".to_string()),
        None => {}
    }

    if recommend_start_work {
        notes.push(format!(
            "To hand the reviewed plan to Atlas, run `{suggested_start_work}`."
        ));
        notes.push("This will switch the execution workflow into Atlas, register the plan as the active boulder, track progress across sessions, and enable continuation if work is interrupted.".to_string());
    }

    notes
}

pub(super) fn prometheus_handoff_output_has_required_shape(content: &str) -> bool {
    [
        "## Plan Summary",
        "**Recommended Next Step**",
        "**Remaining Decisions or Risks**",
        "**Execution Status**",
    ]
    .iter()
    .all(|heading| content.contains(heading))
}

fn extract_handoff_summary_lines(content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in content.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("## ") || line.starts_with("**") || line.starts_with('#') {
            continue;
        }
        lines.push(line.trim_start_matches("- ").trim().to_string());
    }
    dedup_preserve(&mut lines);
    lines.into_iter().take(4).collect()
}

fn collect_decision_needed_lines(content: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in content.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        if lower.contains("[decision needed:") || lower.contains("decision needed") {
            items.push(line.to_string());
        }
    }
    items
}

fn format_markdown_list_section(title: &str, items: &[String], top_heading: bool) -> String {
    let heading = if top_heading {
        format!("## {title}")
    } else {
        format!("**{title}**")
    };
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
        "{heading}
{body}"
    )
}

fn dedup_preserve(items: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| seen.insert(item.clone()));
}
