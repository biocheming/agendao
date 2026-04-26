fn structured_section(title: &str, body: &str) -> String {
    format!(
        "**{title}**
{}",
        body.trim()
    )
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

pub fn normalize_sisyphus_final_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Delegation Path**")
        && trimmed.contains("**Execution Outcome**")
        && trimmed.contains("**Verification**")
    {
        return trimmed.to_string();
    }
    if trimmed.contains("## Delivery Summary")
        && trimmed.contains("**Execution Outcome**")
        && trimmed.contains("**Verification**")
    {
        let summary = first_body_line_after_delivery_summary(trimmed);
        return [
            format!("## Delivery Summary\n{summary}"),
            structured_section(
                "Delegation Path",
                "- Preserve the actual Sisyphus routing decision: delegate by default, direct execution only when the task was genuinely trivial.",
            ),
            strip_delivery_summary_heading(trimmed).to_string(),
        ]
        .join("\n\n");
    }

    let summary = first_meaningful_line(trimmed);
    [
        format!("## Delivery Summary
{summary}"),
        structured_section(
            "Delegation Path",
            "- Default bias: delegate to the best available specialist or skill-backed worker.",
        ),
        structured_section("Execution Outcome", trimmed),
        structured_section(
            "Verification",
            "- Preserve only evidence-backed completion claims from the Sisyphus single-pass execution.",
        ),
        structured_section("Remaining Risks", "- None noted in the final Sisyphus output."),
    ]
    .join("

")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sisyphus_final_output_normalization_wraps_unstructured_delivery() {
        let output = normalize_sisyphus_final_output(
            "Shipped the change and verified the targeted behavior.",
        );
        assert!(output.contains("## Delivery Summary"));
        assert!(output.contains("**Delegation Path**"));
        assert!(output.contains("**Execution Outcome**"));
        assert!(output.contains("**Verification**"));
        assert!(output.contains("Shipped the change"));
    }

    #[test]
    fn sisyphus_final_output_normalization_preserves_structured_delivery() {
        let structured = "## Delivery Summary
Done.

**Delegation Path**
- Delegated frontend checks to the best specialist.

**Execution Outcome**
- A

**Verification**
- B";
        assert_eq!(normalize_sisyphus_final_output(structured), structured);
    }

    #[test]
    fn sisyphus_final_output_normalization_upgrades_legacy_structured_delivery() {
        let legacy = "## Delivery Summary
Done.

**Execution Outcome**
- A

**Verification**
- B";
        let normalized = normalize_sisyphus_final_output(legacy);
        assert!(normalized.contains("**Delegation Path**"));
        assert!(normalized.contains("**Execution Outcome**"));
        assert!(normalized.contains("**Verification**"));
        assert_eq!(normalized.matches("## Delivery Summary").count(), 1);
    }
}
