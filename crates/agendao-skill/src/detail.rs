use crate::{
    write::parse_skill_frontmatter_lines, SkillDetailView,
    SkillFrontmatter as FormalSkillFrontmatter, SkillHermesMetadata, SkillMetadataBlocks,
    SkillPrerequisites, SkillReadinessStatus, SkillRequiredEnvironmentVariable,
    SkillAgendaoMetadata,
};
use serde::de::DeserializeOwned;
use serde_yaml::Value as YamlValue;
use std::env;
use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn read_skill_detail(skill_markdown: &Path) -> Result<SkillDetailView, std::io::Error> {
    let Some(frontmatter) = read_skill_frontmatter(skill_markdown)? else {
        return Ok(SkillDetailView::default());
    };

    let version = project_optional_scalar(&frontmatter, |value| value.version.clone(), "version");
    let author = project_optional_scalar(&frontmatter, |value| value.author.clone(), "author");
    let license = project_optional_scalar(&frontmatter, |value| value.license.clone(), "license");
    let platforms = project_string_list(&frontmatter, |value| value.platforms.clone(), "platforms");
    let tags = project_tags(&frontmatter);
    let related_skills = project_related_skills(&frontmatter);
    let prerequisites = project_prerequisites(&frontmatter);
    let metadata = project_metadata_blocks(&frontmatter, &tags, &related_skills);
    let required_environment_variables = project_required_environment_variables(&frontmatter);
    let required_commands = project_required_commands(&frontmatter);
    let missing_required_environment_variables = required_environment_variables
        .iter()
        .filter(|entry| env::var_os(&entry.name).is_none())
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    let missing_required_commands = required_commands
        .iter()
        .filter(|command| which::which(command).is_err())
        .cloned()
        .collect::<Vec<_>>();
    let setup_needed =
        !missing_required_environment_variables.is_empty() || !missing_required_commands.is_empty();

    Ok(SkillDetailView {
        version,
        author,
        license,
        platforms,
        tags,
        related_skills,
        prerequisites,
        metadata,
        required_environment_variables,
        required_commands,
        missing_required_environment_variables,
        missing_required_commands,
        setup_needed,
        setup_skipped: false,
        readiness_status: if setup_needed {
            SkillReadinessStatus::SetupNeeded
        } else {
            SkillReadinessStatus::Available
        },
    })
}

#[derive(Debug, Clone)]
struct SkillFrontmatterSource {
    raw: String,
    parsed: Option<YamlValue>,
    formal: FormalSkillFrontmatter,
}

fn read_skill_frontmatter(
    skill_markdown: &Path,
) -> Result<Option<SkillFrontmatterSource>, std::io::Error> {
    let Some(raw) = read_frontmatter_block(skill_markdown)? else {
        return Ok(None);
    };
    let lines = raw.lines().map(|line| line.to_string()).collect::<Vec<_>>();
    let parsed = serde_yaml::from_str::<YamlValue>(&raw).ok();
    let formal = parse_skill_frontmatter_lines(&lines)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    Ok(Some(SkillFrontmatterSource {
        raw,
        parsed,
        formal,
    }))
}

fn read_frontmatter_block(skill_markdown: &Path) -> Result<Option<String>, std::io::Error> {
    let raw = fs::read_to_string(skill_markdown)?;
    let normalized = raw.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Ok(None);
    }

    let mut frontmatter_lines = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            return Ok(Some(frontmatter_lines.join("\n")));
        }
        frontmatter_lines.push(line.to_string());
    }

    Ok(None)
}

fn project_optional_scalar(
    frontmatter: &SkillFrontmatterSource,
    formal_value: impl Fn(&FormalSkillFrontmatter) -> Option<String>,
    key: &str,
) -> Option<String> {
    formal_value(&frontmatter.formal)
        .filter(|value| !value.is_empty())
        .or_else(|| parse_optional_scalar(frontmatter, key))
}

fn project_string_list(
    frontmatter: &SkillFrontmatterSource,
    formal_value: impl Fn(&FormalSkillFrontmatter) -> Vec<String>,
    key: &str,
) -> Vec<String> {
    let values = formal_value(&frontmatter.formal);
    if !values.is_empty() {
        return values;
    }
    parse_top_level_list(frontmatter, key)
}

fn project_tags(frontmatter: &SkillFrontmatterSource) -> Vec<String> {
    if let Some(tags) = frontmatter
        .formal
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.tags.clone())
        .filter(|value| !value.is_empty())
    {
        return tags;
    }
    if !frontmatter.formal.tags.is_empty() {
        return frontmatter.formal.tags.clone();
    }
    parse_hermes_list(frontmatter, "tags")
}

fn project_related_skills(frontmatter: &SkillFrontmatterSource) -> Vec<String> {
    if let Some(related_skills) = frontmatter
        .formal
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.hermes.as_ref())
        .map(|metadata| metadata.related_skills.clone())
        .filter(|value| !value.is_empty())
    {
        return related_skills;
    }
    if !frontmatter.formal.related_skills.is_empty() {
        return frontmatter.formal.related_skills.clone();
    }
    parse_hermes_list(frontmatter, "related_skills")
}

fn project_prerequisites(frontmatter: &SkillFrontmatterSource) -> Option<SkillPrerequisites> {
    if let Some(prerequisites) = frontmatter
        .formal
        .prerequisites
        .clone()
        .filter(|value| !value.env_vars.is_empty() || !value.commands.is_empty())
    {
        return Some(prerequisites);
    }
    parse_prerequisites(frontmatter)
}

fn project_metadata_blocks(
    frontmatter: &SkillFrontmatterSource,
    tags: &[String],
    related_skills: &[String],
) -> Option<SkillMetadataBlocks> {
    if let Some(metadata) = frontmatter
        .formal
        .metadata
        .clone()
        .filter(|value| value.hermes.is_some() || value.agendao.is_some())
    {
        return Some(metadata);
    }
    parse_metadata_blocks(frontmatter, tags, related_skills)
}

fn project_required_environment_variables(
    frontmatter: &SkillFrontmatterSource,
) -> Vec<SkillRequiredEnvironmentVariable> {
    if !frontmatter.formal.required_environment_variables.is_empty() {
        return frontmatter.formal.required_environment_variables.clone();
    }
    if let Some(prerequisites) = frontmatter.formal.prerequisites.as_ref() {
        if !prerequisites.env_vars.is_empty() {
            return prerequisites
                .env_vars
                .iter()
                .cloned()
                .map(|name| SkillRequiredEnvironmentVariable {
                    name,
                    description: None,
                    prompt: None,
                    help: None,
                    required_for: None,
                })
                .collect();
        }
    }
    parse_required_environment_variables(frontmatter)
}

fn project_required_commands(frontmatter: &SkillFrontmatterSource) -> Vec<String> {
    if !frontmatter.formal.required_commands.is_empty() {
        return frontmatter.formal.required_commands.clone();
    }
    if let Some(prerequisites) = frontmatter.formal.prerequisites.as_ref() {
        if !prerequisites.commands.is_empty() {
            return prerequisites.commands.clone();
        }
    }
    parse_required_commands(frontmatter)
}

fn parse_hermes_list(frontmatter: &SkillFrontmatterSource, key: &str) -> Vec<String> {
    let nested = parse_yaml_list(frontmatter.parsed.as_ref(), &["metadata", "hermes"], key);
    if !nested.is_empty() {
        return nested;
    }
    let top_level = parse_yaml_list(frontmatter.parsed.as_ref(), &[], key);
    if !top_level.is_empty() {
        return top_level;
    }
    let nested = parse_nested_list(&frontmatter.raw, &["metadata", "hermes"], key);
    if !nested.is_empty() {
        return nested;
    }
    parse_nested_list(&frontmatter.raw, &[], key)
}

fn parse_optional_scalar(frontmatter: &SkillFrontmatterSource, key: &str) -> Option<String> {
    parse_yaml_scalar(frontmatter.parsed.as_ref(), &[], key)
        .or_else(|| parse_top_level_scalar(&frontmatter.raw, key))
        .filter(|value| !value.is_empty())
}

fn parse_top_level_list(frontmatter: &SkillFrontmatterSource, key: &str) -> Vec<String> {
    let values = parse_yaml_list(frontmatter.parsed.as_ref(), &[], key);
    if !values.is_empty() {
        return values;
    }
    parse_nested_list(&frontmatter.raw, &[], key)
}

fn parse_prerequisites(frontmatter: &SkillFrontmatterSource) -> Option<SkillPrerequisites> {
    if let Some(parsed) =
        parse_yaml_typed::<SkillPrerequisites>(frontmatter.parsed.as_ref(), &[], "prerequisites")
    {
        if !parsed.commands.is_empty() || !parsed.env_vars.is_empty() {
            return Some(parsed);
        }
    }

    let env_vars = parse_nested_list(&frontmatter.raw, &["prerequisites"], "env_vars");
    let commands = parse_nested_list(&frontmatter.raw, &["prerequisites"], "commands");
    if env_vars.is_empty() && commands.is_empty() {
        None
    } else {
        Some(SkillPrerequisites { env_vars, commands })
    }
}

fn parse_metadata_blocks(
    frontmatter: &SkillFrontmatterSource,
    tags: &[String],
    related_skills: &[String],
) -> Option<SkillMetadataBlocks> {
    if let Some(parsed) =
        parse_yaml_typed::<SkillMetadataBlocks>(frontmatter.parsed.as_ref(), &[], "metadata")
    {
        if parsed.hermes.is_some() || parsed.agendao.is_some() {
            return Some(parsed);
        }
    }

    let hermes = (!tags.is_empty() || !related_skills.is_empty()).then(|| SkillHermesMetadata {
        tags: tags.to_vec(),
        related_skills: related_skills.to_vec(),
    });

    let agendao = {
        let requires_tools =
            parse_nested_list(&frontmatter.raw, &["metadata", "agendao"], "requires_tools");
        let fallback_for_tools = parse_nested_list(
            &frontmatter.raw,
            &["metadata", "agendao"],
            "fallback_for_tools",
        );
        let requires_toolsets = parse_nested_list(
            &frontmatter.raw,
            &["metadata", "agendao"],
            "requires_toolsets",
        );
        let fallback_for_toolsets = parse_nested_list(
            &frontmatter.raw,
            &["metadata", "agendao"],
            "fallback_for_toolsets",
        );
        let stage_filter =
            parse_nested_list(&frontmatter.raw, &["metadata", "agendao"], "stage_filter");
        if requires_tools.is_empty()
            && fallback_for_tools.is_empty()
            && requires_toolsets.is_empty()
            && fallback_for_toolsets.is_empty()
            && stage_filter.is_empty()
        {
            None
        } else {
            Some(SkillAgendaoMetadata {
                requires_tools,
                fallback_for_tools,
                requires_toolsets,
                fallback_for_toolsets,
                stage_filter,
            })
        }
    };

    if hermes.is_none() && agendao.is_none() {
        None
    } else {
        Some(SkillMetadataBlocks { hermes, agendao })
    }
}

fn parse_required_commands(frontmatter: &SkillFrontmatterSource) -> Vec<String> {
    let commands = parse_yaml_list(frontmatter.parsed.as_ref(), &[], "required_commands");
    if !commands.is_empty() {
        return commands;
    }
    let legacy = parse_yaml_list(frontmatter.parsed.as_ref(), &["prerequisites"], "commands");
    if !legacy.is_empty() {
        return legacy;
    }
    let commands = parse_nested_list(&frontmatter.raw, &[], "required_commands");
    if !commands.is_empty() {
        return commands;
    }
    parse_nested_list(&frontmatter.raw, &["prerequisites"], "commands")
}

fn parse_required_environment_variables(
    frontmatter: &SkillFrontmatterSource,
) -> Vec<SkillRequiredEnvironmentVariable> {
    let env_vars = parse_yaml_named_requirement_list(
        frontmatter.parsed.as_ref(),
        &[],
        "required_environment_variables",
    );
    if !env_vars.is_empty() {
        return env_vars;
    }
    let legacy = parse_yaml_named_requirement_list(
        frontmatter.parsed.as_ref(),
        &["prerequisites"],
        "env_vars",
    );
    if !legacy.is_empty() {
        return legacy;
    }
    let env_vars =
        parse_named_requirement_list(&frontmatter.raw, &[], "required_environment_variables");
    if !env_vars.is_empty() {
        return env_vars;
    }
    parse_named_requirement_list(&frontmatter.raw, &["prerequisites"], "env_vars")
}

fn parse_yaml_typed<T: DeserializeOwned>(
    root: Option<&YamlValue>,
    scope: &[&str],
    key: &str,
) -> Option<T> {
    let root = root?;
    let value = yaml_lookup(root, scope, key)?.clone();
    serde_yaml::from_value(value).ok()
}

fn parse_yaml_scalar(root: Option<&YamlValue>, scope: &[&str], key: &str) -> Option<String> {
    let root = root?;
    let value = yaml_lookup(root, scope, key)?;
    yaml_scalar_to_string(value).filter(|item| !item.is_empty())
}

fn parse_yaml_list(root: Option<&YamlValue>, scope: &[&str], key: &str) -> Vec<String> {
    let Some(root) = root else {
        return Vec::new();
    };
    let Some(value) = yaml_lookup(root, scope, key) else {
        return Vec::new();
    };
    yaml_value_to_string_list(value)
}

fn parse_yaml_named_requirement_list(
    root: Option<&YamlValue>,
    scope: &[&str],
    key: &str,
) -> Vec<SkillRequiredEnvironmentVariable> {
    let Some(root) = root else {
        return Vec::new();
    };
    let Some(value) = yaml_lookup(root, scope, key) else {
        return Vec::new();
    };
    yaml_value_to_named_requirement_list(value)
}

fn yaml_lookup<'a>(mut value: &'a YamlValue, scope: &[&str], key: &str) -> Option<&'a YamlValue> {
    for segment in scope {
        value = yaml_mapping_get(value, segment)?;
    }
    yaml_mapping_get(value, key)
}

fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
    match value {
        YamlValue::Mapping(mapping) => mapping.get(YamlValue::String(key.to_string())),
        _ => None,
    }
}

fn yaml_value_to_string_list(value: &YamlValue) -> Vec<String> {
    match value {
        YamlValue::Sequence(items) => items
            .iter()
            .filter_map(yaml_scalar_to_string)
            .filter(|item| !item.is_empty())
            .collect(),
        _ => yaml_scalar_to_string(value)
            .filter(|item| !item.is_empty())
            .into_iter()
            .collect(),
    }
}

fn yaml_value_to_named_requirement_list(
    value: &YamlValue,
) -> Vec<SkillRequiredEnvironmentVariable> {
    match value {
        YamlValue::Sequence(items) => items
            .iter()
            .filter_map(yaml_requirement_item_to_struct)
            .collect(),
        _ => yaml_requirement_item_to_struct(value).into_iter().collect(),
    }
}

fn yaml_requirement_item_to_struct(value: &YamlValue) -> Option<SkillRequiredEnvironmentVariable> {
    match value {
        YamlValue::String(name) => {
            let name = name.trim();
            if name.is_empty() {
                None
            } else {
                Some(SkillRequiredEnvironmentVariable {
                    name: name.to_string(),
                    description: None,
                    prompt: None,
                    help: None,
                    required_for: None,
                })
            }
        }
        YamlValue::Mapping(mapping) => {
            let name = yaml_mapping_get(value, "name")
                .or_else(|| yaml_mapping_get(value, "env_var"))
                .and_then(yaml_scalar_to_string)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())?;
            let description = mapping
                .get(YamlValue::String("description".to_string()))
                .and_then(yaml_scalar_to_string)
                .filter(|value| !value.is_empty());
            let prompt = mapping
                .get(YamlValue::String("prompt".to_string()))
                .and_then(yaml_scalar_to_string)
                .filter(|value| !value.is_empty());
            let help = mapping
                .get(YamlValue::String("help".to_string()))
                .or_else(|| mapping.get(YamlValue::String("provider_url".to_string())))
                .or_else(|| mapping.get(YamlValue::String("url".to_string())))
                .and_then(yaml_scalar_to_string)
                .filter(|value| !value.is_empty());
            let required_for = mapping
                .get(YamlValue::String("required_for".to_string()))
                .and_then(yaml_scalar_to_string)
                .filter(|value| !value.is_empty());
            Some(SkillRequiredEnvironmentVariable {
                name,
                description,
                prompt,
                help,
                required_for,
            })
        }
        _ => None,
    }
}

fn yaml_scalar_to_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(value) => Some(value.trim().to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        YamlValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_nested_list(frontmatter: &str, scope: &[&str], key: &str) -> Vec<String> {
    let lines = frontmatter.lines().collect::<Vec<_>>();
    let mut scope_stack: Vec<usize> = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        while scope_stack
            .last()
            .copied()
            .is_some_and(|last| indent <= last)
        {
            scope_stack.pop();
        }

        if scope_stack.len() < scope.len() {
            let expected = scope[scope_stack.len()];
            if trimmed == format!("{expected}:") {
                scope_stack.push(indent);
                index += 1;
                continue;
            }
        }

        if scope_stack.len() == scope.len() {
            let prefix = format!("{key}:");
            if let Some(value) = trimmed.strip_prefix(&prefix) {
                let value = value.trim();
                if !value.is_empty() {
                    return parse_inline_yaml_list(value);
                }
                return collect_indented_yaml_list(&lines, index + 1, indent);
            }
        }

        index += 1;
    }

    Vec::new()
}

fn parse_named_requirement_list(
    frontmatter: &str,
    scope: &[&str],
    key: &str,
) -> Vec<SkillRequiredEnvironmentVariable> {
    let lines = frontmatter.lines().collect::<Vec<_>>();
    let mut scope_stack: Vec<usize> = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        while scope_stack
            .last()
            .copied()
            .is_some_and(|last| indent <= last)
        {
            scope_stack.pop();
        }

        if scope_stack.len() < scope.len() {
            let expected = scope[scope_stack.len()];
            if trimmed == format!("{expected}:") {
                scope_stack.push(indent);
                index += 1;
                continue;
            }
        }

        if scope_stack.len() == scope.len() {
            let prefix = format!("{key}:");
            if let Some(value) = trimmed.strip_prefix(&prefix) {
                let value = value.trim();
                if !value.is_empty() {
                    return parse_inline_yaml_list(value)
                        .into_iter()
                        .map(|name| SkillRequiredEnvironmentVariable {
                            name,
                            description: None,
                            prompt: None,
                            help: None,
                            required_for: None,
                        })
                        .collect();
                }
                return collect_named_requirement_items(&lines, index + 1, indent);
            }
        }

        index += 1;
    }

    Vec::new()
}

fn parse_top_level_scalar(frontmatter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in frontmatter.lines() {
        let indent = line.len().saturating_sub(line.trim_start().len());
        let trimmed = line.trim();
        if indent != 0 || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix(&prefix) {
            let normalized = normalize_yaml_scalar(value);
            if !normalized.is_empty() {
                return Some(normalized);
            }
        }
    }
    None
}

fn collect_indented_yaml_list(lines: &[&str], start: usize, parent_indent: usize) -> Vec<String> {
    let mut items = Vec::new();
    let mut cursor = start;
    while cursor < lines.len() {
        let next = lines[cursor];
        let next_trimmed = next.trim();
        let next_indent = next.len().saturating_sub(next.trim_start().len());
        if next_trimmed.is_empty() || next_trimmed.starts_with('#') {
            cursor += 1;
            continue;
        }
        if next_indent <= parent_indent {
            break;
        }
        if let Some(item) = next_trimmed.strip_prefix('-') {
            let item = normalize_yaml_scalar(item.trim());
            if !item.is_empty() {
                items.push(item);
            }
        }
        cursor += 1;
    }
    items
}

fn collect_named_requirement_items(
    lines: &[&str],
    start: usize,
    parent_indent: usize,
) -> Vec<SkillRequiredEnvironmentVariable> {
    let mut items = Vec::new();
    let mut cursor = start;
    while cursor < lines.len() {
        let next = lines[cursor];
        let trimmed = next.trim();
        let indent = next.len().saturating_sub(next.trim_start().len());
        if trimmed.is_empty() || trimmed.starts_with('#') {
            cursor += 1;
            continue;
        }
        if indent <= parent_indent {
            break;
        }

        if let Some(item) = trimmed.strip_prefix('-') {
            let item = item.trim();
            if let Some((nested_key, nested_value)) = item.split_once(':') {
                if nested_key.trim() == "name" {
                    let name = normalize_yaml_scalar(nested_value);
                    let item_indent = indent;
                    let mut description = None;
                    let mut prompt = None;
                    let mut help = None;
                    let mut required_for = None;
                    cursor += 1;
                    while cursor < lines.len() {
                        let nested = lines[cursor];
                        let nested_trimmed = nested.trim();
                        let nested_indent = nested.len().saturating_sub(nested.trim_start().len());
                        if nested_trimmed.is_empty() || nested_trimmed.starts_with('#') {
                            cursor += 1;
                            continue;
                        }
                        if nested_indent <= item_indent {
                            break;
                        }
                        if let Some((field_key, field_value)) = nested_trimmed.split_once(':') {
                            let value = normalize_yaml_scalar(field_value);
                            match field_key.trim() {
                                "description" if !value.is_empty() => description = Some(value),
                                "prompt" if !value.is_empty() => prompt = Some(value),
                                "help" | "provider_url" | "url" if !value.is_empty() => {
                                    if help.is_none() {
                                        help = Some(value);
                                    }
                                }
                                "required_for" if !value.is_empty() => required_for = Some(value),
                                _ => {}
                            }
                        }
                        cursor += 1;
                    }
                    if !name.is_empty() {
                        items.push(SkillRequiredEnvironmentVariable {
                            name,
                            description,
                            prompt,
                            help,
                            required_for,
                        });
                    }
                    continue;
                }
            }

            let scalar = normalize_yaml_scalar(item);
            if !scalar.is_empty() {
                items.push(SkillRequiredEnvironmentVariable {
                    name: scalar,
                    description: None,
                    prompt: None,
                    help: None,
                    required_for: None,
                });
            }
        }
        cursor += 1;
    }
    items
}

fn parse_inline_yaml_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return trimmed[1..trimmed.len() - 1]
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(normalize_yaml_scalar)
            .filter(|item| !item.is_empty())
            .collect();
    }

    let scalar = normalize_yaml_scalar(trimmed);
    if scalar.is_empty() {
        Vec::new()
    } else {
        vec![scalar]
    }
}

fn normalize_yaml_scalar(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_skill_detail_prefers_yaml_parse_but_keeps_fallback() {
        let dir = tempdir().unwrap();
        let skill_markdown = dir.path().join("SKILL.md");
        fs::write(
            &skill_markdown,
            r#"---
name: example
description: Example skill
version: 1.2.3
author: Example Author
license: MIT
platforms: [linux, macos]
prerequisites:
  env_vars: [LEGACY_API_KEY]
  commands: [legacy-cli]
required_environment_variables:
  - name: DEMO_API_KEY
    description: Demo token
required_commands: [demo-cli]
metadata:
  hermes:
    tags: [chemistry, design]
    related_skills: [molecule-report]
  agendao:
    requires_tools: [skill_manage]
    stage_filter: [implementation]
---
# Example
"#,
        )
        .unwrap();

        let detail = read_skill_detail(&skill_markdown).unwrap();
        assert_eq!(detail.version.as_deref(), Some("1.2.3"));
        assert_eq!(detail.author.as_deref(), Some("Example Author"));
        assert_eq!(detail.license.as_deref(), Some("MIT"));
        assert_eq!(detail.platforms, vec!["linux", "macos"]);
        assert_eq!(detail.tags, vec!["chemistry", "design"]);
        assert_eq!(detail.related_skills, vec!["molecule-report"]);
        assert_eq!(
            detail.prerequisites,
            Some(SkillPrerequisites {
                env_vars: vec!["LEGACY_API_KEY".to_string()],
                commands: vec!["legacy-cli".to_string()],
            })
        );
        assert_eq!(
            detail.metadata,
            Some(SkillMetadataBlocks {
                hermes: Some(SkillHermesMetadata {
                    tags: vec!["chemistry".to_string(), "design".to_string()],
                    related_skills: vec!["molecule-report".to_string()],
                }),
                agendao: Some(SkillAgendaoMetadata {
                    requires_tools: vec!["skill_manage".to_string()],
                    fallback_for_tools: Vec::new(),
                    requires_toolsets: Vec::new(),
                    fallback_for_toolsets: Vec::new(),
                    stage_filter: vec!["implementation".to_string()],
                }),
            })
        );
        assert_eq!(detail.required_environment_variables.len(), 1);
        assert_eq!(
            detail.required_environment_variables[0].name,
            "DEMO_API_KEY"
        );
        assert_eq!(
            detail.required_environment_variables[0]
                .description
                .as_deref(),
            Some("Demo token")
        );
        assert_eq!(detail.required_commands, vec!["demo-cli"]);
        assert_eq!(detail.readiness_status, SkillReadinessStatus::SetupNeeded);
    }

    #[test]
    fn read_skill_detail_falls_back_when_yaml_parse_fails() {
        let dir = tempdir().unwrap();
        let skill_markdown = dir.path().join("SKILL.md");
        fs::write(
            &skill_markdown,
            r#"---
name: broken
description: Broken skill
tags: [chemistry, design
related_skills:
  - molecule-report
required_commands:
  - demo-cli
---
# Broken
"#,
        )
        .unwrap();

        let detail = read_skill_detail(&skill_markdown).unwrap();
        assert_eq!(detail.tags, vec!["[chemistry, design"]);
        assert_eq!(detail.related_skills, vec!["molecule-report"]);
        assert_eq!(detail.required_commands, vec!["demo-cli"]);
        assert_eq!(detail.readiness_status, SkillReadinessStatus::SetupNeeded);
    }
}
