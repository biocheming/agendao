use crate::blueprint::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateId {
    Direct,
    Plan,
    Coordinate,
    Verify,
    Autoresearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateParameters {
    pub name: BlueprintName,
    pub primary_agent: AgentId,
    pub planning_agent: Option<AgentId>,
    #[serde(default)]
    pub collaborators: Vec<AgentId>,
    #[serde(default)]
    pub agent_skills: BTreeMap<AgentId, BTreeSet<SkillId>>,
    #[serde(default)]
    pub agent_tools: BTreeMap<AgentId, BTreeSet<ToolId>>,
    pub agent_max_steps: BTreeMap<AgentId, u32>,
    pub evaluator: Option<EvaluatorId>,
    pub checkpoint: Option<CapabilityId>,
    pub limits: ExecutionLimits,
    pub output: OutputContract,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("template '{template:?}' requires an evaluator")]
    MissingEvaluator { template: TemplateId },
    #[error("coordinate template requires at least two collaborators")]
    TooFewCollaborators,
}

pub fn build_template(
    template: TemplateId,
    parameters: &TemplateParameters,
) -> Result<SchedulerBlueprint, TemplateError> {
    match template {
        TemplateId::Direct => Ok(direct(parameters)),
        TemplateId::Plan => Ok(plan(parameters)),
        TemplateId::Coordinate => coordinate(parameters),
        TemplateId::Verify => verify(parameters),
        TemplateId::Autoresearch => autoresearch(parameters),
    }
}

fn blueprint(
    parameters: &TemplateParameters,
    entry: &str,
    nodes: BTreeMap<NodeId, NodeSpec>,
) -> SchedulerBlueprint {
    SchedulerBlueprint {
        schema: BlueprintSchemaVersion::V1,
        name: parameters.name.clone(),
        entry: NodeId::from(entry),
        nodes,
        limits: parameters.limits.clone(),
        output: parameters.output.clone(),
    }
}

fn agent(parameters: &TemplateParameters, agent: AgentId, next: &str) -> NodeSpec {
    let skills = parameters
        .agent_skills
        .get(&agent)
        .cloned()
        .unwrap_or_default();
    let tools = parameters
        .agent_tools
        .get(&agent)
        .cloned()
        .unwrap_or_default();
    NodeSpec::Agent(AgentNode {
        max_steps: parameters
            .agent_max_steps
            .get(&agent)
            .copied()
            .unwrap_or(parameters.limits.max_agent_steps)
            .min(parameters.limits.max_agent_steps),
        agent,
        skills,
        tools,
        required_model_capabilities: BTreeSet::new(),
        next: NodeId::from(next),
    })
}

fn end() -> NodeSpec {
    NodeSpec::End(EndNode {
        result: ResultSource::LastNode,
    })
}

fn direct(parameters: &TemplateParameters) -> SchedulerBlueprint {
    blueprint(
        parameters,
        "execute",
        BTreeMap::from([
            (
                NodeId::from("execute"),
                agent(parameters, parameters.primary_agent.clone(), "done"),
            ),
            (NodeId::from("done"), end()),
        ]),
    )
}

fn plan(parameters: &TemplateParameters) -> SchedulerBlueprint {
    let planner = parameters
        .planning_agent
        .clone()
        .unwrap_or_else(|| parameters.primary_agent.clone());
    blueprint(
        parameters,
        "plan",
        BTreeMap::from([
            (NodeId::from("plan"), agent(parameters, planner, "execute")),
            (
                NodeId::from("execute"),
                agent(parameters, parameters.primary_agent.clone(), "done"),
            ),
            (NodeId::from("done"), end()),
        ]),
    )
}

fn coordinate(parameters: &TemplateParameters) -> Result<SchedulerBlueprint, TemplateError> {
    if parameters.collaborators.len() < 2 {
        return Err(TemplateError::TooFewCollaborators);
    }
    let mut nodes = BTreeMap::new();
    let branches: Vec<NodeId> = parameters
        .collaborators
        .iter()
        .enumerate()
        .map(|(index, collaborator)| {
            let id = NodeId::new(format!("branch-{}", index + 1));
            nodes.insert(
                id.clone(),
                agent(parameters, collaborator.clone(), "synthesize"),
            );
            id
        })
        .collect();
    nodes.insert(
        NodeId::from("coordinate"),
        NodeSpec::Parallel(ParallelNode {
            max_parallelism: parameters.limits.max_parallelism.min(branches.len() as u32),
            branches,
            join: NodeId::from("synthesize"),
            failure_mode: ParallelFailureMode::Collect,
        }),
    );
    nodes.insert(
        NodeId::from("synthesize"),
        agent(parameters, parameters.primary_agent.clone(), "done"),
    );
    nodes.insert(NodeId::from("done"), end());
    Ok(blueprint(parameters, "coordinate", nodes))
}

fn verify(parameters: &TemplateParameters) -> Result<SchedulerBlueprint, TemplateError> {
    let evaluator = parameters
        .evaluator
        .clone()
        .ok_or(TemplateError::MissingEvaluator {
            template: TemplateId::Verify,
        })?;
    Ok(blueprint(
        parameters,
        "execute",
        BTreeMap::from([
            (
                NodeId::from("execute"),
                agent(parameters, parameters.primary_agent.clone(), "verify"),
            ),
            (
                NodeId::from("verify"),
                NodeSpec::Gate(GateNode {
                    evaluator,
                    on_pass: NodeId::from("accepted"),
                    on_fail: NodeId::from("rejected"),
                    on_indeterminate: NodeId::from("uncertain"),
                }),
            ),
            (NodeId::from("accepted"), end()),
            (NodeId::from("rejected"), end()),
            (NodeId::from("uncertain"), end()),
        ]),
    ))
}

fn autoresearch(parameters: &TemplateParameters) -> Result<SchedulerBlueprint, TemplateError> {
    let evaluator = parameters
        .evaluator
        .clone()
        .ok_or(TemplateError::MissingEvaluator {
            template: TemplateId::Autoresearch,
        })?;
    let body = SchedulerGraph {
        entry: NodeId::from("experiment"),
        nodes: BTreeMap::from([
            (
                NodeId::from("experiment"),
                agent(
                    parameters,
                    parameters.primary_agent.clone(),
                    "iteration-done",
                ),
            ),
            (NodeId::from("iteration-done"), end()),
        ]),
    };
    Ok(blueprint(
        parameters,
        "research",
        BTreeMap::from([
            (
                NodeId::from("research"),
                NodeSpec::Loop(LoopNode {
                    body: Box::new(body),
                    evaluator,
                    max_iterations: parameters.limits.max_loop_iterations,
                    checkpoint: parameters.checkpoint.clone(),
                    on_satisfied: NodeId::from("accepted"),
                    on_exhausted: NodeId::from("exhausted"),
                }),
            ),
            (NodeId::from("accepted"), end()),
            (NodeId::from("exhausted"), end()),
        ]),
    ))
}
