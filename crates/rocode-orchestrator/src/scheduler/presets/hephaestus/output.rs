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

pub fn normalize_hephaestus_final_output(output: &str) -> String {
    let normalized_input = normalize_embedded_delivery_summary(output);
    let trimmed = normalized_input.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Completion Status**")
        && trimmed.contains("**What Changed**")
        && trimmed.contains("**Verification**")
    {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**What Changed**")
        && trimmed.contains("**Verification**")
    {
        let summary = first_body_line_after_delivery_summary(trimmed);
        return [
            format!("## Delivery Summary\n{summary}"),
            structured_section(
                "Completion Status",
                "- Preserve the Hephaestus finish-gate result only: done when completion is proved, otherwise state the concrete blocker or retry gap explicitly.",
            ),
            strip_delivery_summary_heading(trimmed).to_string(),
        ]
        .join("\n\n");
    }

    let summary = first_meaningful_line(trimmed);
    [
        format!("## Delivery Summary\n{summary}"),
        structured_section(
            "Completion Status",
            "- Default to incomplete unless the autonomous loop produced concrete proof that the request is done.",
        ),
        structured_section("What Changed", trimmed),
        structured_section(
            "Verification",
            "- Preserve only verification-backed completion claims from the Hephaestus execution loop.",
        ),
        structured_section("Risks or Follow-ups", "- None noted in the final Hephaestus output."),
    ]
    .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hephaestus_final_output_normalization_wraps_unstructured_delivery() {
        let output = normalize_hephaestus_final_output(
            "Fixed the diagnostics path and ran the targeted check.",
        );
        assert!(output.contains("## Delivery Summary"));
        assert!(output.contains("**Completion Status**"));
        assert!(output.contains("**What Changed**"));
        assert!(output.contains("**Verification**"));
        assert!(output.contains("Fixed the diagnostics path"));
    }

    #[test]
    fn hephaestus_final_output_normalization_preserves_structured_delivery() {
        let structured =
            "## Delivery Summary\nDone.\n\n**Completion Status**\n- Done.\n\n**What Changed**\n- A\n\n**Verification**\n- B";
        assert_eq!(normalize_hephaestus_final_output(structured), structured);
    }

    #[test]
    fn hephaestus_final_output_normalization_upgrades_legacy_structured_delivery() {
        let legacy = "## Delivery Summary\nDone.\n\n**What Changed**\n- A\n\n**Verification**\n- B";
        let normalized = normalize_hephaestus_final_output(legacy);
        assert!(normalized.contains("**Completion Status**"));
        assert!(normalized.contains("**What Changed**"));
        assert!(normalized.contains("**Verification**"));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
    }

    #[test]
    fn hephaestus_final_output_normalization_strips_preface_before_embedded_delivery() {
        let prefaced = "I have the final verified state.\n\n## `## Delivery Summary`\n\nDone.\n\n**Completion Status**\n- Done.\n\n**What Changed**\n- A\n\n**Verification**\n- B";
        let normalized = normalize_hephaestus_final_output(prefaced);
        assert!(normalized.starts_with("## Delivery Summary"));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
        assert!(!normalized.contains("I have the final verified state."));
    }
}
