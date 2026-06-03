use std::sync::Arc;

use agendao_skill::{
    extract_methodology_template_from_markdown, infer_runtime_skill_names,
    RuntimeInstructionSource, SkillAuthority,
};
use serde::{Deserialize, Serialize};

use crate::{PartType, Session};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SkillReflectionData {
    pub(super) skills_used: Vec<SkillUsageSummary>,
    pub(super) tool_calls: Vec<ToolCallSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SkillUsageSummary {
    pub(super) name: String,
    pub(super) methodology: Option<agendao_skill::SkillMethodologyTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ToolCallSummary {
    pub(super) tool_name: String,
    pub(super) tool_input_summary: String,
    pub(super) tool_result_summary: String,
}

pub(super) fn extract_tool_call_history(session: &Session) -> Vec<ToolCallSummary> {
    session
        .messages
        .iter()
        .flat_map(|msg| msg.parts.iter())
        .filter_map(|part| match &part.part_type {
            PartType::ToolCall {
                id, name, input, ..
            } => Some(ToolCallSummary {
                tool_name: name.clone(),
                tool_input_summary: summarize_tool_input(input),
                tool_result_summary: find_tool_result(session, id)
                    .unwrap_or_else(|| "(no result)".to_string()),
            }),
            _ => None,
        })
        .collect()
}

pub(super) fn prepare_skill_reflection(
    config_store: Option<Arc<agendao_config::ConfigStore>>,
    session: &Session,
) -> Option<SkillReflectionData> {
    let instructions = session
        .metadata
        .get("runtime_skill_instructions")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<RuntimeInstructionSource>>(value).ok())?;

    let skill_names =
        infer_runtime_skill_names(std::path::Path::new(&session.directory), &instructions);
    if skill_names.is_empty() {
        return None;
    }

    let authority = SkillAuthority::new(std::path::PathBuf::from(&session.directory), config_store);
    let skills_used = skill_names
        .into_iter()
        .map(|name| {
            let methodology = authority
                .load_skill_for_inspection(&name, None)
                .ok()
                .and_then(|loaded| extract_methodology_template_from_markdown(&loaded.content));
            SkillUsageSummary { name, methodology }
        })
        .collect::<Vec<_>>();

    if skills_used.is_empty() {
        return None;
    }

    Some(SkillReflectionData {
        skills_used,
        tool_calls: extract_tool_call_history(session),
    })
}

pub(super) fn update_skill_reflection_metadata(
    config_store: Option<Arc<agendao_config::ConfigStore>>,
    session: &mut Session,
) {
    if let Some(reflection) = prepare_skill_reflection(config_store, session) {
        if let Ok(value) = serde_json::to_value(reflection) {
            session.insert_metadata("skill_reflection", value);
            return;
        }
    }
    session.remove_metadata("skill_reflection");
}

pub(super) fn augment_system_prompt_with_skill_reflection(
    session: &mut Session,
    system_prompt: Option<String>,
) -> Option<String> {
    let Some(reflection) = take_skill_reflection(session) else {
        return system_prompt;
    };
    let reflection_section = build_skill_reflection_section(&reflection);
    match system_prompt {
        Some(base) if !base.trim().is_empty() => Some(format!("{base}\n\n{reflection_section}")),
        _ => Some(reflection_section),
    }
}

fn take_skill_reflection(session: &mut Session) -> Option<SkillReflectionData> {
    session
        .remove_metadata("skill_reflection")
        .and_then(|value| serde_json::from_value::<SkillReflectionData>(value).ok())
}

fn build_skill_reflection_section(reflection: &SkillReflectionData) -> String {
    let mut lines = vec![
        "## Skill Usage Reflection".to_string(),
        String::new(),
        "The previous session used the following skills. Review whether they still accurately reflect what was done.".to_string(),
        String::new(),
    ];

    for skill in &reflection.skills_used {
        lines.push(format!("### Skill: `{}`", skill.name));
        if let Some(methodology) = &skill.methodology {
            for (idx, step) in methodology.core_steps.iter().enumerate() {
                let tools = if step.experienced_tools.is_empty() {
                    "(no experienced_tools recorded)".to_string()
                } else {
                    format!("tools: {}", step.experienced_tools.join(", "))
                };
                lines.push(format!("- Step {}: {} [{}]", idx + 1, step.title, tools));
            }
        }
        lines.push(String::new());
    }

    lines.push("### Actual Tool Calls".to_string());
    for call in &reflection.tool_calls {
        lines.push(format!(
            "- `{}`: {}",
            call.tool_name, call.tool_input_summary
        ));
    }

    lines.push(String::new());
    lines.push("If a skill is clearly outdated or incomplete compared with what was actually done, consider calling `skill_manage(\"patch\", ...)`.".to_string());
    lines.push("Do not patch for minor variations.".to_string());

    lines.join("\n")
}

fn summarize_tool_input(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(map) => {
            if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                return format!("command={}", truncate_string(cmd, 100));
            }
            truncate_string(&serde_json::to_string(input).unwrap_or_default(), 150)
        }
        _ => truncate_string(&input.to_string(), 150),
    }
}

fn find_tool_result(session: &Session, tool_call_id: &str) -> Option<String> {
    session
        .messages
        .iter()
        .flat_map(|msg| msg.parts.iter())
        .find_map(|part| match &part.part_type {
            PartType::ToolResult {
                tool_call_id: id,
                content,
                ..
            } if id == tool_call_id => Some(truncate_string(content, 200)),
            _ => None,
        })
}

fn truncate_string(value: &str, max_len: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_len).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
