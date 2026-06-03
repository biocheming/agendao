use super::{print_cli_list_on_surface, CliExecutionRuntime, CliStyle, OutputBlock};
use agendao_command::output_blocks::tool_cli_activity_label;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(super) struct CliObservedExecutionTopology {
    pub(super) active: bool,
    pub(super) root_id: Option<String>,
    pub(super) scheduler_id: Option<String>,
    pub(super) active_stage_id: Option<String>,
    pub(super) stage_order: Vec<String>,
    pub(super) nodes: HashMap<String, CliObservedExecutionNode>,
}

#[derive(Debug, Clone)]
pub(super) struct CliObservedExecutionNode {
    pub(super) kind: String,
    pub(super) label: String,
    pub(super) status: String,
    pub(super) waiting_on: Option<String>,
    pub(super) recent_event: Option<String>,
    pub(super) children: Vec<String>,
}

impl CliObservedExecutionTopology {
    pub(super) fn reset_for_run(&mut self, agent_name: &str, scheduler_profile: Option<&str>) {
        self.active = true;
        self.root_id = Some("prompt".to_string());
        self.scheduler_id = scheduler_profile.map(|_| "scheduler".to_string());
        self.active_stage_id = None;
        self.stage_order.clear();
        self.nodes.clear();
        self.nodes.insert(
            "prompt".to_string(),
            CliObservedExecutionNode {
                kind: "prompt".to_string(),
                label: format!("Prompt run ({})", agent_name),
                status: "running".to_string(),
                waiting_on: Some("model".to_string()),
                recent_event: Some("Prompt run started".to_string()),
                children: Vec::new(),
            },
        );
        if let Some(profile) = scheduler_profile {
            self.nodes.insert(
                "scheduler".to_string(),
                CliObservedExecutionNode {
                    kind: "scheduler".to_string(),
                    label: format!("Scheduler run ({})", profile),
                    status: "running".to_string(),
                    waiting_on: Some("model".to_string()),
                    recent_event: Some("Scheduler orchestration started".to_string()),
                    children: Vec::new(),
                },
            );
            self.attach_child("prompt", "scheduler");
        }
    }

    pub(super) fn observe_block(&mut self, block: &OutputBlock) {
        match block {
            OutputBlock::SchedulerStage(stage) => self.observe_scheduler_stage(stage),
            OutputBlock::Tool(tool) => self.observe_tool(tool),
            _ => {}
        }
    }

    fn observe_scheduler_stage(
        &mut self,
        stage: &agendao_command::output_blocks::SchedulerStageBlock,
    ) {
        let stage_id = stage.stage_id.clone().unwrap_or_else(|| {
            format!(
                "stage:{}:{}",
                stage
                    .stage_index
                    .unwrap_or((self.stage_order.len() + 1) as u64),
                stage.stage
            )
        });
        let parent_id = self
            .scheduler_id
            .clone()
            .unwrap_or_else(|| self.root_id.clone().unwrap_or_else(|| "prompt".to_string()));
        let status = stage
            .status
            .clone()
            .unwrap_or_else(|| "running".to_string());
        let node = self
            .nodes
            .entry(stage_id.clone())
            .or_insert(CliObservedExecutionNode {
                kind: "stage".to_string(),
                label: stage.title.clone(),
                status: status.clone(),
                waiting_on: stage.waiting_on.clone(),
                recent_event: stage.last_event.clone(),
                children: Vec::new(),
            });
        node.label = stage.title.clone();
        node.status = status.clone();
        node.waiting_on = stage.waiting_on.clone();
        node.recent_event = stage.last_event.clone();
        self.attach_child(&parent_id, &stage_id);
        if !self.stage_order.iter().any(|id| id == &stage_id) {
            self.stage_order.push(stage_id.clone());
        }
        if matches!(
            status.as_str(),
            "running" | "waiting" | "cancelling" | "retry"
        ) {
            self.active_stage_id = Some(stage_id.clone());
        }
        if let Some(scheduler_id) = self.scheduler_id.clone() {
            if let Some(scheduler) = self.nodes.get_mut(&scheduler_id) {
                scheduler.waiting_on = stage.waiting_on.clone();
                scheduler.recent_event = stage.last_event.clone();
                scheduler.status = if status == "waiting" {
                    "waiting".to_string()
                } else {
                    "running".to_string()
                };
            }
        }
    }

    fn observe_tool(&mut self, tool: &agendao_command::output_blocks::ToolBlock) {
        let parent_id = self
            .active_stage_id
            .clone()
            .or_else(|| self.scheduler_id.clone())
            .or_else(|| self.root_id.clone())
            .unwrap_or_else(|| "prompt".to_string());
        let tool_id = format!("tool:{}:{}", parent_id, tool.name);
        let status = match tool.phase {
            agendao_command::output_blocks::ToolPhase::Start
            | agendao_command::output_blocks::ToolPhase::Running => "running",
            agendao_command::output_blocks::ToolPhase::Done => "done",
            agendao_command::output_blocks::ToolPhase::Error => "error",
        }
        .to_string();
        let node = self
            .nodes
            .entry(tool_id.clone())
            .or_insert(CliObservedExecutionNode {
                kind: "tool".to_string(),
                label: tool_cli_activity_label(tool),
                status: status.clone(),
                waiting_on: Some("tool".to_string()),
                recent_event: tool.detail.clone(),
                children: Vec::new(),
            });
        node.label = tool_cli_activity_label(tool);
        node.status = status.clone();
        node.waiting_on = if matches!(tool.phase, agendao_command::output_blocks::ToolPhase::Done) {
            None
        } else {
            Some("tool".to_string())
        };
        node.recent_event = tool.detail.clone();
        self.attach_child(&parent_id, &tool_id);
    }

    pub(super) fn start_question(&mut self, count: usize) {
        let parent_id = self
            .active_stage_id
            .clone()
            .or_else(|| self.scheduler_id.clone())
            .or_else(|| self.root_id.clone())
            .unwrap_or_else(|| "prompt".to_string());
        let question_id = format!("question:{}:{}", parent_id, count);
        self.nodes.insert(
            question_id.clone(),
            CliObservedExecutionNode {
                kind: "question".to_string(),
                label: format!("Question ({})", count),
                status: "waiting".to_string(),
                waiting_on: Some("user".to_string()),
                recent_event: Some("Waiting for user answer".to_string()),
                children: Vec::new(),
            },
        );
        self.attach_child(&parent_id, &question_id);
    }

    pub(super) fn finish_question(&mut self, outcome: &str) {
        for node in self
            .nodes
            .values_mut()
            .filter(|node| node.kind == "question")
        {
            if node.status == "waiting" {
                node.status = outcome.to_string();
                node.waiting_on = None;
                node.recent_event = Some(format!("Question {}", outcome));
            }
        }
    }

    pub(super) fn finish_run(&mut self, outcome: Option<String>) {
        self.active = false;
        if let Some(root_id) = self.root_id.clone() {
            if let Some(root) = self.nodes.get_mut(&root_id) {
                root.status = outcome
                    .clone()
                    .unwrap_or_else(|| "completed".to_string())
                    .to_lowercase();
                root.waiting_on = None;
                root.recent_event = outcome;
            }
        }
    }

    fn attach_child(&mut self, parent_id: &str, attached_id: &str) {
        if let Some(parent) = self.nodes.get_mut(parent_id) {
            if !parent.children.iter().any(|id| id == attached_id) {
                parent.children.push(attached_id.to_string());
            }
        }
    }
}

pub(super) fn cli_print_execution_topology(
    observed_topology: &Arc<Mutex<CliObservedExecutionTopology>>,
    runtime: Option<&CliExecutionRuntime>,
    style: &CliStyle,
) {
    let Ok(topology) = observed_topology.lock() else {
        let _ = print_cli_list_on_surface(
            runtime,
            "Execution Topology",
            None,
            &[style.dim("unavailable")],
            style,
        );
        return;
    };
    if topology.nodes.is_empty() {
        let _ = print_cli_list_on_surface(
            runtime,
            "Execution Topology",
            None,
            &[style.dim("no observed execution topology")],
            style,
        );
        return;
    }
    let mut lines = Vec::new();
    if topology.active {
        lines.push(style.bold_green("active"));
    } else {
        lines.push(style.dim("idle · last observed topology"));
    }
    if let Some(root_id) = topology.root_id.as_deref() {
        cli_collect_execution_node(&topology, root_id, "", true, &mut lines);
    }
    let _ = print_cli_list_on_surface(runtime, "Execution Topology", None, &lines, style);
}

fn cli_collect_execution_node(
    topology: &CliObservedExecutionTopology,
    node_id: &str,
    prefix: &str,
    is_last: bool,
    lines: &mut Vec<String>,
) {
    let Some(node) = topology.nodes.get(node_id) else {
        return;
    };
    let branch = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };
    let mut line = format!("{}{}{} · {}", prefix, branch, node.label, node.status);
    if let Some(waiting_on) = node.waiting_on.as_deref() {
        line.push_str(&format!(" · waiting {}", waiting_on));
    }
    if let Some(recent_event) = node.recent_event.as_deref() {
        line.push_str(&format!(" · {}", recent_event));
    }
    lines.push(line);
    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    };
    for (index, attached_id) in node.children.iter().enumerate() {
        cli_collect_execution_node(
            topology,
            attached_id,
            &child_prefix,
            index + 1 == node.children.len(),
            lines,
        );
    }
}
