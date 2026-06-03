use super::*;

pub fn sisyphus_junior() -> AgentInfo {
    let tools: Vec<&str> = PRIMARY_BUILTIN_TOOLS
        .iter()
        .filter(|t| !matches!(**t, "task" | "task_flow"))
        .copied()
        .collect();
    let mut agent = base_allowlist_agent("sisyphus-junior", AgentMode::Subagent, &tools);
    agent.description = Some("Category-dispatched executor for domain-specific tasks.".to_string());
    agent.max_steps = Some(50);
    agent
}
