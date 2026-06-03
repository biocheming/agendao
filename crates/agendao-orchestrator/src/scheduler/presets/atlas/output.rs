use super::super::normalize_embedded_delivery_summary;

fn structured_section(title: &str, body: &str) -> String {
    format!("**{title}**\n{}", body.trim())
}

fn first_meaningful_line(content: &str) -> &str {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("No summary provided.")
}

fn first_body_line_after_delivery_summary(content: &str) -> &str {
    let mut seen_delivery_summary = false;
    for line in content.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line == "## Delivery Summary" {
            seen_delivery_summary = true;
            continue;
        }
        if seen_delivery_summary {
            return line;
        }
    }
    first_meaningful_line(content)
}

fn strip_delivery_summary_heading(content: &str) -> &str {
    content
        .trim()
        .strip_prefix("## Delivery Summary")
        .map(str::trim_start)
        .unwrap_or_else(|| content.trim())
}

pub fn normalize_atlas_final_output(output: &str) -> String {
    let normalized_input = normalize_embedded_delivery_summary(output);
    let trimmed = normalized_input.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Task Status**")
        && trimmed.contains("**Verification**")
        && trimmed.contains("**Gate Decision**")
    {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Task Status**")
        && trimmed.contains("**Verification**")
    {
        let summary = first_body_line_after_delivery_summary(trimmed);
        return [
            format!("## Delivery Summary\n{summary}"),
            strip_delivery_summary_heading(trimmed).to_string(),
            structured_section(
                "Gate Decision",
                "- Atlas gate result: preserve the verified decision only. Ship only when every required task is complete with evidence; otherwise continue or block explicitly.",
            ),
        ]
        .join("\n\n");
    }

    let summary = first_meaningful_line(trimmed);
    [
        format!("## Delivery Summary\n{summary}"),
        structured_section("Task Status", trimmed),
        structured_section(
            "Verification",
            "- Preserve only evidence-backed completion claims from Atlas coordination and verification stages.",
        ),
        structured_section(
            "Gate Decision",
            "- Default to `continue` unless Atlas has explicit evidence that every required task boundary is complete.",
        ),
        structured_section("Blockers or Risks", "- None noted in the final Atlas output."),
        structured_section("Next Actions", "- None."),
    ]
    .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_final_output_normalization_wraps_unstructured_delivery() {
        let output = normalize_atlas_final_output("Task A done. Task B verified.");
        assert!(output.contains("## Delivery Summary"));
        assert!(output.contains("**Task Status**"));
        assert!(output.contains("**Verification**"));
        assert!(output.contains("**Gate Decision**"));
        assert!(output.contains("Task A done. Task B verified."));
    }

    #[test]
    fn atlas_final_output_normalization_preserves_structured_delivery() {
        let structured =
            "## Delivery Summary\nDone.\n\n**Task Status**\n- A\n\n**Verification**\n- B\n\n**Gate Decision**\n- Ship.";
        assert_eq!(normalize_atlas_final_output(structured), structured);
    }

    #[test]
    fn atlas_final_output_normalization_upgrades_legacy_structured_delivery() {
        let legacy = "## Delivery Summary\nDone.\n\n**Task Status**\n- A\n\n**Verification**\n- B";
        let normalized = normalize_atlas_final_output(legacy);
        assert!(normalized.contains("**Gate Decision**"));
        assert!(normalized.contains("**Task Status**"));
        assert!(normalized.contains("**Verification**"));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
    }

    #[test]
    fn atlas_final_output_normalization_strips_preface_before_embedded_delivery() {
        let prefaced = "Working through the verified result now.\n\n## `## Delivery Summary`\n\nReady.\n\n**Task Status**\n- A\n\n**Verification**\n- B\n\n**Gate Decision**\n- Ship.";
        let normalized = normalize_atlas_final_output(prefaced);
        assert!(normalized.starts_with("## Delivery Summary"));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
        assert!(!normalized.contains("Working through the verified result now."));
    }
}
