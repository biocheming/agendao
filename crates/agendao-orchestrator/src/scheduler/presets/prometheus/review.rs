pub struct PrometheusReviewContext<'a> {
    pub route_rationale_summary: Option<&'a str>,
    pub planning_artifact_path: Option<&'a str>,
    pub interviewed: Option<&'a str>,
    pub planned: Option<&'a str>,
    pub draft_snapshot: Option<&'a str>,
    pub advisory_review: Option<&'a str>,
}

pub fn normalize_prometheus_review_output(
    context: PrometheusReviewContext<'_>,
    review_output: &str,
) -> String {
    if prometheus_review_output_has_required_shape(review_output) {
        return review_output.trim().to_string();
    }

    let plan_name = prometheus_plan_name(context.planning_artifact_path);
    let mut key_decisions = Vec::new();
    if let Some(summary) = context
        .route_rationale_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        key_decisions.push(summary.to_string());
    }
    if context.planning_artifact_path.is_some() {
        key_decisions
            .push("Reviewed plan artifact preserved as the single source of truth.".to_string());
    }
    if key_decisions.is_empty() {
        key_decisions.push("Plan structure reviewed before execution handoff.".to_string());
    }

    let scope_in = context
        .interviewed
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|_| "Use the interview brief and reviewed plan as the execution scope.".to_string())
        .unwrap_or_else(|| "Follow the reviewed plan artifact for in-scope work.".to_string());
    let scope_out =
        "Prometheus still does not execute code; implementation remains outside this workflow."
            .to_string();

    let guardrails = extract_named_section_items(review_output, "Guardrails Applied")
        .filter(|items| !items.is_empty())
        .or_else(|| {
            context
                .advisory_review
                .map(|text| extract_markdown_list_items(text, 6))
        })
        .unwrap_or_default();
    let defaults_applied = extract_named_section_items(review_output, "Defaults Applied")
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            vec![
                "Kept the Prometheus planner workflow active instead of rerouting to another preset."
                    .to_string(),
                "Kept review enabled before handoff.".to_string(),
            ]
        });
    let auto_resolved =
        extract_named_section_items(review_output, "Auto-Resolved").unwrap_or_default();
    let decisions_needed = collect_decisions_needed(&context, review_output);
    let handoff_ready = if decisions_needed.is_empty() {
        vec!["Ready for handoff once the reviewed plan is accepted.".to_string()]
    } else {
        vec!["Blocked pending the decisions listed below.".to_string()]
    };
    let review_notes = if review_output.trim().is_empty() {
        "- None.".to_string()
    } else {
        review_output.trim().to_string()
    };

    [
        format!("## Plan Generated: {plan_name}"),
        format!(
            "**Key Decisions Made**\n{}",
            key_decisions
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        format!("**Scope**\n- IN: {scope_in}\n- OUT: {scope_out}"),
        format_optional_list("Guardrails Applied", &guardrails),
        format_optional_list("Auto-Resolved", &auto_resolved),
        format_optional_list("Defaults Applied", &defaults_applied),
        format_optional_list("Decisions Needed", &decisions_needed),
        format_optional_list("Handoff Readiness", &handoff_ready),
        format!("**Review Notes**\n{review_notes}"),
    ]
    .join("\n\n")
}

fn prometheus_plan_name(plan_path: Option<&str>) -> String {
    plan_path
        .and_then(|path| std::path::Path::new(path).file_stem())
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "prometheus-plan".to_string())
}

fn extract_named_section_items(text: &str, title: &str) -> Option<Vec<String>> {
    let heading = format!("**{title}**");
    let start = text.find(&heading)? + heading.len();
    let remainder = text[start..].trim_start();
    let mut items = Vec::new();

    for line in remainder.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("**") || line.starts_with("## ") {
            break;
        }
        if let Some(item) = line.strip_prefix("- ") {
            let item = item.trim();
            if !item.is_empty() && !item.eq_ignore_ascii_case("none.") {
                items.push(item.to_string());
            }
        }
    }

    Some(items)
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
                ch == '-' || ch == '*' || ch.is_ascii_digit() || ch == '.' || ch == ' '
            })
            .trim()
            .to_string()
        })
        .filter(|line| !line.is_empty())
        .take(max)
        .collect()
}

fn collect_decisions_needed(
    context: &PrometheusReviewContext<'_>,
    review_output: &str,
) -> Vec<String> {
    let mut items = Vec::new();
    for source in [context.planned, context.draft_snapshot, Some(review_output)]
        .into_iter()
        .flatten()
    {
        for line in source.lines().map(str::trim) {
            let lower = line.to_ascii_lowercase();
            if lower.contains("[decision needed:") || lower.contains("decision needed") {
                items.push(line.to_string());
            }
        }
    }
    items.sort();
    items.dedup();
    items
}

fn format_optional_list(title: &str, items: &[String]) -> String {
    if items.is_empty() {
        format!("**{title}**\n- None.")
    } else {
        format!(
            "**{title}**\n{}",
            items
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn prometheus_review_output_has_required_shape(review_output: &str) -> bool {
    let trimmed = review_output.trim();
    if trimmed.is_empty() {
        return false;
    }

    [
        "## Plan Generated:",
        "**Key Decisions Made**",
        "**Scope**",
        "**Guardrails Applied**",
        "**Auto-Resolved**",
        "**Defaults Applied**",
        "**Decisions Needed**",
        "**Handoff Readiness**",
        "**Review Notes**",
    ]
    .iter()
    .all(|heading| trimmed.contains(heading))
}
