use super::super::{
    DynamicAgentTreeDeclaration, SchedulerExecutionGateDecision, SchedulerExecutionGateStatus,
    DYNAMIC_AGENT_TREE_MAX_CHILDREN,
};
use crate::agent_tree::AgentTreeNode;
use crate::types::{AgentDescriptor, ModelRef};
use serde_json::Value;

pub fn parse_execution_gate_decision(output: &str) -> Option<SchedulerExecutionGateDecision> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    for candidate in profile_json_candidates(trimmed) {
        if let Some(decision) = parse_execution_gate_candidate(&candidate) {
            return Some(decision);
        }
    }

    None
}

fn parse_execution_gate_candidate(candidate: &str) -> Option<SchedulerExecutionGateDecision> {
    if let Ok(decision) = serde_json::from_str::<SchedulerExecutionGateDecision>(candidate) {
        return Some(normalize_execution_gate_decision(decision));
    }

    let value = serde_json::from_str::<Value>(candidate).ok()?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| value.get("gate_decision").and_then(Value::as_str))
        .and_then(parse_execution_gate_status_token)?;

    let summary = first_non_empty_string(&[
        value.get("summary").and_then(Value::as_str),
        value.get("reasoning").and_then(Value::as_str),
        value.get("execution_fidelity").and_then(Value::as_str),
    ])
    .unwrap_or_default()
    .to_string();

    let next_input = first_non_empty_string(&[
        value.get("next_input").and_then(Value::as_str),
        joined_string_array(value.get("next_actions")).as_deref(),
    ])
    .map(str::to_string);

    let final_response = first_non_empty_string(&[
        value.get("final_response").and_then(Value::as_str),
        build_legacy_gate_details_markdown(&value).as_deref(),
    ])
    .map(str::to_string);

    Some(normalize_execution_gate_decision(
        SchedulerExecutionGateDecision {
            status,
            summary,
            next_input,
            final_response,
        },
    ))
}

fn parse_execution_gate_status_token(token: &str) -> Option<SchedulerExecutionGateStatus> {
    match token.trim().to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" | "finish" | "finished" => {
            Some(SchedulerExecutionGateStatus::Done)
        }
        "continue" | "retry" | "again" => Some(SchedulerExecutionGateStatus::Continue),
        "blocked" | "block" | "stop" => Some(SchedulerExecutionGateStatus::Blocked),
        _ => None,
    }
}

fn first_non_empty_string<'a>(candidates: &[Option<&'a str>]) -> Option<&'a str> {
    candidates
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

fn joined_string_array(value: Option<&Value>) -> Option<String> {
    let items = value?.as_array()?;
    let lines = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("- {value}"))
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn build_legacy_gate_details_markdown(value: &Value) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(summary) = value
        .get("verification_summary")
        .and_then(Value::as_object)
        .filter(|summary| !summary.is_empty())
    {
        let mut lines = Vec::new();
        for (key, raw) in summary {
            let rendered = match raw {
                Value::String(text) => text.clone(),
                _ => raw.to_string(),
            };
            lines.push(format!("- {}: {}", key.replace('_', " "), rendered));
        }
        if !lines.is_empty() {
            sections.push(format!("### Verification Summary\n{}", lines.join("\n")));
        }
    }

    if let Some(task_status) = value
        .get("task_status")
        .and_then(Value::as_object)
        .filter(|status| !status.is_empty())
    {
        let mut lines = Vec::new();
        for (key, raw) in task_status {
            let rendered = raw.as_str().unwrap_or_default().trim();
            if !rendered.is_empty() {
                lines.push(format!("- {}: {}", key.replace('_', " "), rendered));
            }
        }
        if !lines.is_empty() {
            sections.push(format!("### Task Status\n{}", lines.join("\n")));
        }
    }

    if let Some(execution_fidelity) = value
        .get("execution_fidelity")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("### Execution Fidelity\n{}", execution_fidelity));
    }

    if let Some(minor_issues) = joined_string_array(value.get("minor_issues")) {
        sections.push(format!("### Minor Issues\n{}", minor_issues));
    }

    if let Some(next_actions) = joined_string_array(value.get("next_actions")) {
        sections.push(format!("### Next Actions\n{}", next_actions));
    }

    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

pub fn normalize_execution_gate_decision(
    mut decision: SchedulerExecutionGateDecision,
) -> SchedulerExecutionGateDecision {
    decision.summary = decision.summary.trim().to_string();
    decision.next_input = decision
        .next_input
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    decision.final_response = decision
        .final_response
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if matches!(decision.status, SchedulerExecutionGateStatus::Continue)
        && decision.next_input.is_none()
    {
        let fallback = if decision.summary.is_empty() {
            "continue the bounded retry on the unresolved gap and collect concrete verification evidence"
                .to_string()
        } else {
            decision.summary.clone()
        };
        decision.next_input = Some(fallback);
    }

    decision
}

fn profile_json_candidates(output: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    for marker in ["```json", "```JSON", "```"] {
        let mut remaining = output;
        while let Some(start) = remaining.find(marker) {
            let after = &remaining[start + marker.len()..];
            if let Some(end) = after.find("```") {
                let candidate = after[..end].trim();
                if !candidate.is_empty() {
                    candidates.push(candidate.to_string());
                }
                remaining = &after[end + 3..];
            } else {
                break;
            }
        }
    }

    if let Some((start, end)) = profile_find_balanced_json_object(output) {
        let candidate = output[start..end].trim();
        if !candidate.is_empty() {
            candidates.push(candidate.to_string());
        }
    }

    if candidates.is_empty() {
        candidates.push(output.trim().to_string());
    }

    candidates
}

fn profile_find_balanced_json_object(input: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return start.map(|s| (s, idx + ch.len_utf8()));
                }
            }
            _ => {}
        }
    }

    None
}

/// Attempt to extract a dynamic agent tree declaration from LLM output.
///
/// Looks for a `<parallel_plan>` XML block first, then falls back to
/// scanning for a top-level JSON object with a `children` field.
/// Returns `None` gracefully on parse failure so the scheduler can
/// continue with the standard delegation path.
pub fn parse_dynamic_agent_tree(output: &str) -> Option<AgentTreeNode> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Priority 1: <parallel_plan>...</parallel_plan> XML block
    if let Some(json_str) = extract_xml_block(trimmed, "parallel_plan") {
        if let Some(node) = try_parse_declaration(json_str) {
            return Some(node);
        }
    }

    // Priority 2: scan JSON candidates for a declaration
    for candidate in profile_json_candidates(trimmed) {
        if let Some(node) = try_parse_declaration(&candidate) {
            return Some(node);
        }
    }

    None
}

fn extract_xml_block<'a>(input: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = input.find(&open)?;
    let content_start = start + open.len();
    let end = input[content_start..].find(&close)?;
    Some(&input[content_start..content_start + end])
}

fn try_parse_declaration(json_str: &str) -> Option<AgentTreeNode> {
    let decl: DynamicAgentTreeDeclaration = serde_json::from_str(json_str).ok()?;
    validate_declaration(&decl)?;
    Some(declaration_to_node(&decl))
}

fn validate_declaration(decl: &DynamicAgentTreeDeclaration) -> Option<()> {
    if decl.children.is_empty() {
        tracing::warn!("dynamic agent tree rejected: no children");
        return None;
    }
    if decl.children.len() > DYNAMIC_AGENT_TREE_MAX_CHILDREN {
        tracing::warn!(
            count = decl.children.len(),
            max = DYNAMIC_AGENT_TREE_MAX_CHILDREN,
            "dynamic agent tree rejected: too many children"
        );
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    for child in &decl.children {
        let name = child.name.trim();
        if name.is_empty() {
            tracing::warn!("dynamic agent tree rejected: child with empty name");
            return None;
        }
        if !seen.insert(name.to_string()) {
            tracing::warn!(name, "dynamic agent tree rejected: duplicate child name");
            return None;
        }
    }
    Some(())
}

fn declaration_to_node(decl: &DynamicAgentTreeDeclaration) -> AgentTreeNode {
    let root = AgentTreeNode::new(AgentDescriptor {
        name: "coordinator".to_string(),
        system_prompt: Some(decl.root_task.clone()),
        model: None,
        max_steps: None,
        temperature: None,
        allowed_tools: Vec::new(),
    });

    let children: Vec<AgentTreeNode> = decl
        .children
        .iter()
        .map(|child| {
            AgentTreeNode::new(AgentDescriptor {
                name: child.name.clone(),
                system_prompt: Some(child.task.clone()),
                model: child.model.as_deref().and_then(parse_model_ref),
                max_steps: None,
                temperature: None,
                allowed_tools: child.allowed_tools.clone(),
            })
        })
        .collect();

    root.with_children(children)
}

/// Parse a "provider:model" string into a ModelRef.
fn parse_model_ref(raw: &str) -> Option<ModelRef> {
    let (provider, model) = raw
        .trim()
        .split_once(':')
        .or_else(|| raw.trim().split_once('/'))?;
    let provider = provider.trim();
    let model = model.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelRef {
        provider_id: provider.to_string(),
        model_id: model.to_string(),
    })
}

#[cfg(test)]
mod dynamic_tree_tests {
    use super::*;

    #[test]
    fn parse_dynamic_agent_tree_from_xml_block() {
        let output = r#"Here is the plan analysis.

<parallel_plan>
{
  "root_task": "Coordinate the migration cleanup",
  "children": [
    {
      "name": "worker-alpha",
      "task": "Update schema files",
      "allowed_tools": ["read", "write", "glob"]
    },
    {
      "name": "worker-beta",
      "task": "Verify migration paths",
      "allowed_tools": ["read", "bash"]
    }
  ]
}
</parallel_plan>

Now waiting for verification."#;

        let node = parse_dynamic_agent_tree(output).expect("should parse");
        assert_eq!(node.agent.name, "coordinator");
        assert_eq!(
            node.agent.system_prompt.as_deref(),
            Some("Coordinate the migration cleanup")
        );
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].agent.name, "worker-alpha");
        assert_eq!(node.children[1].agent.name, "worker-beta");
    }

    #[test]
    fn parse_dynamic_agent_tree_from_json_code_block() {
        let output = r#"Analysis done.

```json
{
  "root_task": "Refactor module",
  "children": [
    {
      "name": "refactor-core",
      "task": "Refactor core module"
    },
    {
      "name": "refactor-tests",
      "task": "Update tests"
    }
  ]
}
```

Done."#;

        let node = parse_dynamic_agent_tree(output).expect("should parse");
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn parse_dynamic_agent_tree_rejects_too_many_children() {
        let output = r#"<parallel_plan>
{
  "root_task": "Too many workers",
  "children": [
    {"name": "a", "task": "Task A"},
    {"name": "b", "task": "Task B"},
    {"name": "c", "task": "Task C"},
    {"name": "d", "task": "Task D"},
    {"name": "e", "task": "Task E"},
    {"name": "f", "task": "Task F"}
  ]
}
</parallel_plan>"#;

        assert!(parse_dynamic_agent_tree(output).is_none());
    }

    #[test]
    fn parse_dynamic_agent_tree_rejects_duplicate_names() {
        let output = r#"<parallel_plan>
{
  "root_task": "Duplicate names",
  "children": [
    {"name": "worker", "task": "Task A"},
    {"name": "worker", "task": "Task B"}
  ]
}
</parallel_plan>"#;

        assert!(parse_dynamic_agent_tree(output).is_none());
    }

    #[test]
    fn parse_dynamic_agent_tree_rejects_empty_children() {
        let output = r#"<parallel_plan>
{
  "root_task": "No children",
  "children": []
}
</parallel_plan>"#;

        assert!(parse_dynamic_agent_tree(output).is_none());
    }

    #[test]
    fn parse_dynamic_agent_tree_returns_none_on_no_json() {
        let output = "This is just plain text with no JSON at all.";
        assert!(parse_dynamic_agent_tree(output).is_none());
    }

    #[test]
    fn parse_dynamic_agent_tree_handles_model_override() {
        let output = r#"<parallel_plan>
{
  "root_task": "Mixed work",
  "children": [
    {
      "name": "fast-worker",
      "task": "Quick analysis",
      "model": "openai:gpt-4o-mini"
    }
  ]
}
</parallel_plan>"#;

        let node = parse_dynamic_agent_tree(output).expect("should parse");
        assert_eq!(node.children.len(), 1);
        let model = node.children[0]
            .agent
            .model
            .as_ref()
            .expect("should have model");
        assert_eq!(model.provider_id, "openai");
        assert_eq!(model.model_id, "gpt-4o-mini");
    }

    #[test]
    fn parse_dynamic_agent_tree_preserves_allowed_tools() {
        let output = r#"<parallel_plan>
{
  "root_task": "Tool check",
  "children": [
    {
      "name": "reader",
      "task": "Read-only analysis",
      "allowed_tools": ["read", "glob", "grep"]
    }
  ]
}
</parallel_plan>"#;

        let node = parse_dynamic_agent_tree(output).expect("should parse");
        assert_eq!(
            node.children[0].agent.allowed_tools,
            vec!["read", "glob", "grep"]
        );
    }
}
