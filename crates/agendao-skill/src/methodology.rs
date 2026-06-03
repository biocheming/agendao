use crate::{write::parse_skill_document, SkillError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_MAX_SKILL_LINES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillQualityRule {
    pub rule_id: String,
    pub heading_hints: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillQualityRubric {
    pub recommended_max_lines: usize,
    pub required_rules: Vec<SkillQualityRule>,
}

impl Default for SkillQualityRubric {
    fn default() -> Self {
        Self {
            recommended_max_lines: DEFAULT_MAX_SKILL_LINES,
            required_rules: vec![
                SkillQualityRule {
                    rule_id: "quality.trigger_section".to_string(),
                    heading_hints: vec![
                        "when to use".to_string(),
                        "quick start".to_string(),
                        "how to use".to_string(),
                        "何时使用".to_string(),
                        "快速开始".to_string(),
                    ],
                    message: "skill should state when it should be used, so the runtime can load it for the right task shape.".to_string(),
                },
                SkillQualityRule {
                    rule_id: "quality.steps_section".to_string(),
                    heading_hints: vec![
                        "core steps".to_string(),
                        "steps".to_string(),
                        "workflow".to_string(),
                        "核心步骤".to_string(),
                        "执行步骤".to_string(),
                    ],
                    message: "skill should include an explicit step-by-step workflow instead of only principles or prose.".to_string(),
                },
                SkillQualityRule {
                    rule_id: "quality.success_criteria".to_string(),
                    heading_hints: vec![
                        "success criteria".to_string(),
                        "success standard".to_string(),
                        "成功标准".to_string(),
                    ],
                    message: "skill should define success criteria so the agent can verify whether the workflow actually completed.".to_string(),
                },
                SkillQualityRule {
                    rule_id: "quality.validation_section".to_string(),
                    heading_hints: vec![
                        "validation".to_string(),
                        "checklist".to_string(),
                        "验证".to_string(),
                        "检查清单".to_string(),
                    ],
                    message: "skill should include a validation or checklist section so execution can be audited instead of assumed.".to_string(),
                },
                SkillQualityRule {
                    rule_id: "quality.boundaries_section".to_string(),
                    heading_hints: vec![
                        "when not to use".to_string(),
                        "boundaries".to_string(),
                        "pitfalls".to_string(),
                        "不适用".to_string(),
                        "边界".to_string(),
                        "注意事项".to_string(),
                    ],
                    message: "skill should document boundaries or pitfalls so one-off workarounds do not get mistaken for general methodology.".to_string(),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillMethodologyTemplate {
    #[serde(default)]
    pub when_to_use: Vec<String>,
    #[serde(default)]
    pub when_not_to_use: Vec<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub core_steps: Vec<SkillMethodologyStep>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub validation: Vec<String>,
    #[serde(default)]
    pub pitfalls: Vec<String>,
    #[serde(default)]
    pub references: Vec<SkillMethodologyReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMethodologyStep {
    pub title: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub experienced_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMethodologyReference {
    pub label: String,
    pub path: String,
}

pub fn render_methodology_skill_body(
    skill_name: &str,
    template: &SkillMethodologyTemplate,
) -> Result<String, SkillError> {
    validate_methodology_template(template)?;

    let mut lines = Vec::new();
    lines.push(format!("# {}", humanize_skill_name(skill_name)));
    lines.push(String::new());
    lines.push("## When To Use".to_string());
    for item in normalized_non_empty(&template.when_to_use) {
        lines.push(format!("- {}", item));
    }

    if !template.when_not_to_use.is_empty() {
        lines.push(String::new());
        lines.push("## When Not To Use".to_string());
        for item in normalized_non_empty(&template.when_not_to_use) {
            lines.push(format!("- {}", item));
        }
    }

    if !template.prerequisites.is_empty() {
        lines.push(String::new());
        lines.push("## Prerequisites".to_string());
        for item in normalized_non_empty(&template.prerequisites) {
            lines.push(format!("- {}", item));
        }
    }

    lines.push(String::new());
    lines.push("## Core Steps".to_string());
    for (index, step) in template.core_steps.iter().enumerate() {
        let title = step.title.trim();
        let action = step.action.trim();
        let experienced_suffix = if step.experienced_tools.is_empty() {
            String::new()
        } else {
            format!(" _Experienced: {}_", step.experienced_tools.join(", "))
        };
        let outcome_suffix = step
            .outcome
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!(" Outcome: {}", value))
            .unwrap_or_default();
        lines.push(format!(
            "{}. **{}**: {}{}{}",
            index + 1,
            title,
            action,
            experienced_suffix,
            outcome_suffix
        ));
    }

    lines.push(String::new());
    lines.push("## Success Criteria".to_string());
    for item in normalized_non_empty(&template.success_criteria) {
        lines.push(format!("- [ ] {}", item));
    }

    lines.push(String::new());
    lines.push("## Validation".to_string());
    for item in normalized_non_empty(&template.validation) {
        lines.push(format!("- [ ] {}", item));
    }

    lines.push(String::new());
    lines.push("## Boundaries and Pitfalls".to_string());
    let boundaries = normalized_non_empty(&template.when_not_to_use);
    let pitfalls = normalized_non_empty(&template.pitfalls);
    for item in boundaries.iter().chain(pitfalls.iter()) {
        lines.push(format!("- {}", item));
    }

    if !template.references.is_empty() {
        lines.push(String::new());
        lines.push("## References".to_string());
        for reference in &template.references {
            let label = reference.label.trim();
            let path = reference.path.trim();
            if !label.is_empty() && !path.is_empty() {
                lines.push(format!("- `{}` - {}", path, label));
            }
        }
    }

    Ok(lines.join("\n").trim().to_string())
}

pub fn extract_methodology_template_from_markdown(
    content: &str,
) -> Option<SkillMethodologyTemplate> {
    let body = extract_skill_body(content)?;
    let sections = parse_methodology_sections(&body);

    let when_to_use = parse_markdown_bullets(sections.get("when to use")?)?;
    let when_not_to_use = sections
        .get("when not to use")
        .and_then(|lines| parse_markdown_bullets(lines))
        .unwrap_or_default();
    let prerequisites = sections
        .get("prerequisites")
        .and_then(|lines| parse_markdown_bullets(lines))
        .unwrap_or_default();
    let core_steps = parse_methodology_steps(sections.get("core steps")?)?;
    let success_criteria = parse_markdown_checklist(sections.get("success criteria")?)?;
    let validation = parse_markdown_checklist(sections.get("validation")?)?;
    let boundaries = parse_markdown_bullets(sections.get("boundaries and pitfalls")?)?;
    let pitfalls = boundaries
        .into_iter()
        .filter(|item| {
            !when_not_to_use
                .iter()
                .any(|boundary| boundary.eq_ignore_ascii_case(item))
        })
        .collect::<Vec<_>>();
    let references = sections
        .get("references")
        .and_then(|lines| parse_methodology_references(lines))
        .unwrap_or_default();

    let template = SkillMethodologyTemplate {
        when_to_use,
        when_not_to_use,
        prerequisites,
        core_steps,
        success_criteria,
        validation,
        pitfalls,
        references,
    };
    validate_methodology_template(&template).ok()?;
    Some(template)
}

fn validate_methodology_template(template: &SkillMethodologyTemplate) -> Result<(), SkillError> {
    if normalized_non_empty(&template.when_to_use).is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: "methodology template requires at least one `when_to_use` item".to_string(),
        });
    }
    if template.core_steps.is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: "methodology template requires at least one `core_steps` entry".to_string(),
        });
    }
    for step in &template.core_steps {
        if step.title.trim().is_empty() || step.action.trim().is_empty() {
            return Err(SkillError::InvalidSkillContent {
                message: "each methodology step must include both `title` and `action`".to_string(),
            });
        }
        for tool_id in &step.experienced_tools {
            if tool_id.trim().is_empty() || tool_id.contains(char::is_whitespace) {
                return Err(SkillError::InvalidSkillContent {
                    message: format!(
                        "experienced_tools entry `{}` is invalid: must be non-empty and contain no whitespace",
                        tool_id
                    ),
                });
            }
        }
    }
    if normalized_non_empty(&template.success_criteria).is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: "methodology template requires at least one `success_criteria` item"
                .to_string(),
        });
    }
    if normalized_non_empty(&template.validation).is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: "methodology template requires at least one `validation` item".to_string(),
        });
    }
    if normalized_non_empty(&template.when_not_to_use).is_empty()
        && normalized_non_empty(&template.pitfalls).is_empty()
    {
        return Err(SkillError::InvalidSkillContent {
            message: "methodology template requires at least one boundary or pitfall item"
                .to_string(),
        });
    }
    Ok(())
}

fn extract_skill_body(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("---") {
        return parse_skill_document(trimmed)
            .ok()
            .map(|document| document.body);
    }
    Some(trimmed.to_string())
}

fn normalized_non_empty(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|item| item.trim().replace("\r\n", "\n"))
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_methodology_sections(body: &str) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current_heading: Option<String> = None;

    for raw_line in body.replace("\r\n", "\n").lines() {
        let trimmed = raw_line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            current_heading = Some(heading.trim().to_ascii_lowercase());
            sections
                .entry(heading.trim().to_ascii_lowercase())
                .or_default();
            continue;
        }
        if current_heading.is_none() || trimmed.starts_with("# ") {
            continue;
        }
        if let Some(lines) = current_heading
            .as_ref()
            .and_then(|heading| sections.get_mut(heading))
        {
            lines.push(trimmed.to_string());
        }
    }

    sections
}

fn parse_markdown_bullets(lines: &[String]) -> Option<Vec<String>> {
    let items = lines
        .iter()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("- ")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

fn parse_markdown_checklist(lines: &[String]) -> Option<Vec<String>> {
    let items = lines
        .iter()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("- [ ] ")
                .or_else(|| trimmed.strip_prefix("- [] "))
                .or_else(|| trimmed.strip_prefix("- "))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

fn parse_methodology_steps(lines: &[String]) -> Option<Vec<SkillMethodologyStep>> {
    let mut steps = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let numbered = strip_numbered_prefix(trimmed)?;
        let (title, remainder) = if let Some(stripped) = numbered.strip_prefix("**") {
            let (title, remainder) = stripped.split_once("**:")?;
            (title.trim().to_string(), remainder.trim().to_string())
        } else {
            let (title, remainder) = numbered.split_once(':')?;
            (title.trim().to_string(), remainder.trim().to_string())
        };
        if title.is_empty() || remainder.is_empty() {
            return None;
        }
        let (remainder, experienced_tools) = if let Some(idx) = remainder.find("_Experienced: ") {
            let before = remainder[..idx].trim();
            let after = &remainder[idx + "_Experienced: ".len()..];
            let parsed = if let Some(end_idx) = after.find("_ Outcome: ") {
                Some((
                    after[..end_idx].to_string(),
                    after[end_idx + 1..].trim().to_string(),
                ))
            } else {
                after
                    .strip_suffix('_')
                    .map(|tools| (tools.to_string(), String::new()))
            };
            if let Some((tools_str, rest)) = parsed {
                let tools = tools_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>();
                let combined = if before.is_empty() {
                    rest
                } else if rest.is_empty() {
                    before.to_string()
                } else {
                    format!("{before} {rest}")
                };
                (combined, tools)
            } else {
                (remainder.to_string(), Vec::new())
            }
        } else {
            (remainder.to_string(), Vec::new())
        };
        let (action, outcome) = if let Some((action, outcome)) = remainder.split_once(" Outcome: ")
        {
            (
                action.trim().to_string(),
                Some(outcome.trim().to_string()).filter(|value| !value.is_empty()),
            )
        } else {
            (remainder.trim().to_string(), None)
        };
        if action.is_empty() {
            return None;
        }
        steps.push(SkillMethodologyStep {
            title,
            action,
            outcome,
            experienced_tools,
        });
    }
    (!steps.is_empty()).then_some(steps)
}

fn parse_methodology_references(lines: &[String]) -> Option<Vec<SkillMethodologyReference>> {
    let mut references = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(reference) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let Some(path_start) = reference.find('`') else {
            continue;
        };
        let remainder = &reference[path_start + 1..];
        let Some(path_end) = remainder.find('`') else {
            continue;
        };
        let path = remainder[..path_end].trim();
        let label = remainder[path_end + 1..]
            .trim()
            .trim_start_matches('-')
            .trim();
        if !path.is_empty() && !label.is_empty() {
            references.push(SkillMethodologyReference {
                label: label.to_string(),
                path: path.to_string(),
            });
        }
    }
    (!references.is_empty()).then_some(references)
}

fn strip_numbered_prefix(line: &str) -> Option<&str> {
    let digit_count = line.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let remainder = line.get(digit_count..)?.trim_start();
    remainder.strip_prefix('.').map(str::trim_start)
}

fn humanize_skill_name(skill_name: &str) -> String {
    skill_name
        .split(['-', '_'])
        .filter(|segment| !segment.trim().is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.push_str(chars.as_str());
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methodology_template_renders_expected_sections() {
        let markdown = render_methodology_skill_body(
            "provider-refresh",
            &SkillMethodologyTemplate {
                when_to_use: vec!["Use when provider metadata is stale.".to_string()],
                when_not_to_use: vec!["Do not use for one-off API key edits.".to_string()],
                prerequisites: vec!["Provider auth must already be configured.".to_string()],
                core_steps: vec![SkillMethodologyStep {
                    title: "Refresh catalog".to_string(),
                    action: "Run the provider refresh entrypoint.".to_string(),
                    outcome: Some("Local provider list matches the latest source.".to_string()),
                    experienced_tools: vec![],
                }],
                success_criteria: vec![
                    "Target provider exposes the expected model ids.".to_string()
                ],
                validation: vec![
                    "Re-open the provider catalog and confirm the new ids appear.".to_string(),
                ],
                pitfalls: vec!["Do not overwrite workspace-local overrides.".to_string()],
                references: vec![SkillMethodologyReference {
                    label: "Source index contract".to_string(),
                    path: "references/provider-index.md".to_string(),
                }],
            },
        )
        .expect("template should render");

        assert!(markdown.contains("## When To Use"));
        assert!(markdown.contains("## Core Steps"));
        assert!(markdown.contains("## Success Criteria"));
        assert!(markdown.contains("## Validation"));
        assert!(markdown.contains("## Boundaries and Pitfalls"));
        assert!(markdown.contains("references/provider-index.md"));
    }

    #[test]
    fn methodology_template_requires_quality_sections() {
        let err = render_methodology_skill_body(
            "broken",
            &SkillMethodologyTemplate {
                when_to_use: vec![],
                when_not_to_use: vec![],
                prerequisites: vec![],
                core_steps: vec![],
                success_criteria: vec![],
                validation: vec![],
                pitfalls: vec![],
                references: vec![],
            },
        )
        .expect_err("missing template sections should fail");

        assert!(matches!(err, SkillError::InvalidSkillContent { .. }));
    }

    #[test]
    fn methodology_round_trip_extracts_template_from_rendered_markdown() {
        let template = SkillMethodologyTemplate {
            when_to_use: vec!["Use when provider metadata is stale.".to_string()],
            when_not_to_use: vec!["Do not use for API key rotation.".to_string()],
            prerequisites: vec!["Auth must already exist.".to_string()],
            core_steps: vec![SkillMethodologyStep {
                title: "Refresh".to_string(),
                action: "Run the refresh workflow.".to_string(),
                outcome: Some("The latest models are visible.".to_string()),
                experienced_tools: vec!["provider_refresh".to_string(), "catalog_diff".to_string()],
            }],
            success_criteria: vec!["Expected model ids appear.".to_string()],
            validation: vec!["Reload the catalog and compare entries.".to_string()],
            pitfalls: vec!["Do not overwrite workspace overrides.".to_string()],
            references: vec![SkillMethodologyReference {
                label: "Design note".to_string(),
                path: "docs/provider.md".to_string(),
            }],
        };
        let body = render_methodology_skill_body("provider-refresh", &template)
            .expect("render should work");
        let source = format!(
            "---\nname: provider-refresh\ndescription: refresh providers\n---\n\n{}\n",
            body
        );

        let parsed = extract_methodology_template_from_markdown(&source)
            .expect("rendered methodology should parse back");

        assert_eq!(parsed, template);
    }

    #[test]
    fn methodology_extract_rejects_non_methodology_markdown() {
        let source = r#"---
name: ad-hoc
description: ad-hoc
---

# Ad Hoc

Just some prose.
"#;

        assert!(extract_methodology_template_from_markdown(source).is_none());
    }

    #[test]
    fn experienced_tools_render_parse_round_trip() {
        let template = SkillMethodologyTemplate {
            when_to_use: vec!["Use when a repeated container health check is needed.".to_string()],
            when_not_to_use: vec!["Do not use for one-off shell experiments.".to_string()],
            prerequisites: vec![],
            core_steps: vec![SkillMethodologyStep {
                title: "Check health".to_string(),
                action: "Inspect the running service and verify it responds.".to_string(),
                outcome: Some("The current health status is known.".to_string()),
                experienced_tools: vec!["docker".to_string(), "curl".to_string()],
            }],
            success_criteria: vec!["The service health is confirmed.".to_string()],
            validation: vec!["Re-run the health check after changes.".to_string()],
            pitfalls: vec!["Do not restart the service before capturing logs.".to_string()],
            references: vec![],
        };

        let body = render_methodology_skill_body("container-health", &template)
            .expect("render should work");
        assert!(body.contains("_Experienced: docker, curl_"));

        let source = format!(
            "---\nname: container-health\ndescription: check health\n---\n\n{}\n",
            body
        );
        let parsed = extract_methodology_template_from_markdown(&source)
            .expect("rendered methodology should parse");
        assert_eq!(parsed, template);
    }

    #[test]
    fn experienced_tools_empty_backward_compat() {
        let template = SkillMethodologyTemplate {
            when_to_use: vec!["Use when a repeated provider refresh is needed.".to_string()],
            when_not_to_use: vec!["Do not use for ad-hoc scratch notes.".to_string()],
            prerequisites: vec![],
            core_steps: vec![SkillMethodologyStep {
                title: "Refresh".to_string(),
                action: "Run the refresh workflow.".to_string(),
                outcome: None,
                experienced_tools: vec![],
            }],
            success_criteria: vec!["The latest provider list is visible.".to_string()],
            validation: vec!["Reload the provider list.".to_string()],
            pitfalls: vec!["Do not overwrite local overrides.".to_string()],
            references: vec![],
        };

        let body = render_methodology_skill_body("provider-refresh", &template)
            .expect("render should work");
        assert!(!body.contains("_Experienced:"));

        let source = format!(
            "---\nname: provider-refresh\ndescription: refresh providers\n---\n\n{}\n",
            body
        );
        let parsed = extract_methodology_template_from_markdown(&source)
            .expect("rendered methodology should parse");
        assert_eq!(parsed.core_steps[0].experienced_tools, Vec::<String>::new());
    }

    #[test]
    fn experienced_tools_format_validation() {
        let template = SkillMethodologyTemplate {
            when_to_use: vec!["Use when repeated container checks are needed.".to_string()],
            when_not_to_use: vec!["Do not use for one-off notes.".to_string()],
            prerequisites: vec![],
            core_steps: vec![SkillMethodologyStep {
                title: "Check".to_string(),
                action: "Run the container check.".to_string(),
                outcome: None,
                experienced_tools: vec!["docker compose".to_string()],
            }],
            success_criteria: vec!["The container state is known.".to_string()],
            validation: vec!["Confirm the reported state.".to_string()],
            pitfalls: vec!["Do not restart containers during inspection.".to_string()],
            references: vec![],
        };

        let err = validate_methodology_template(&template).expect_err("invalid tools should fail");
        assert!(matches!(err, SkillError::InvalidSkillContent { .. }));
    }
}
