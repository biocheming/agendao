use super::{
    BlueprintFingerprint, CanonicalizationError, ExecutionLimits, NodeId, NodeSpec, ResultSource,
    SchedulerBlueprint, SchedulerGraph,
};
use crate::catalog::{CapabilityKind, SchedulerCatalog};
use crate::policy::PolicyEnvelope;
use std::collections::{BTreeMap, BTreeSet};

const MAX_IDENTIFIER_LEN: usize = 128;
const MAX_SKILL_SUMMARY_CHARS: usize = 1024;
const MAX_FINGERPRINT_CHARS: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum BlueprintValidationError {
    #[error("{kind} identifier is empty or not canonical: '{value}'")]
    InvalidIdentifier { kind: &'static str, value: String },
    #[error("catalog revision must be non-empty")]
    EmptyCatalogRevision,
    #[error("catalog {kind} key '{key}' does not match entry id '{entry_id}'")]
    CatalogIdMismatch {
        kind: &'static str,
        key: String,
        entry_id: String,
    },
    #[error(
        "catalog skill '{skill}' {field} must be trimmed, non-empty, and at most {maximum} characters"
    )]
    InvalidSkillMetadata {
        skill: String,
        field: &'static str,
        maximum: usize,
    },
    #[error("catalog agent '{agent}' references unknown {kind} '{reference}'")]
    InvalidCatalogReference {
        agent: String,
        kind: &'static str,
        reference: String,
    },
    #[error("policy references unknown {kind} '{reference}'")]
    InvalidPolicyReference {
        kind: &'static str,
        reference: String,
    },
    #[error("execution limit '{field}' must be greater than zero")]
    ZeroLimit { field: &'static str },
    #[error("execution limit '{field}' ({requested}) exceeds policy maximum ({maximum})")]
    LimitExceeded {
        field: &'static str,
        requested: u64,
        maximum: u64,
    },
    #[error("{scope} graph is empty")]
    EmptyGraph { scope: String },
    #[error("{scope} entry node '{node}' does not exist")]
    MissingEntry { scope: String, node: String },
    #[error("node '{node}' in {scope} targets missing node '{target}'")]
    MissingTarget {
        scope: String,
        node: String,
        target: String,
    },
    #[error("node '{node}' in {scope} is unreachable")]
    UnreachableNode { scope: String, node: String },
    #[error("ordinary graph cycle detected at node '{node}' in {scope}; use a Loop node")]
    OrdinaryCycle { scope: String, node: String },
    #[error("{scope} has no End node")]
    MissingEnd { scope: String },
    #[error("node '{node}' references unknown agent '{agent}'")]
    UnknownAgent { node: String, agent: String },
    #[error("node '{node}' references unknown skill '{skill}'")]
    UnknownSkill { node: String, skill: String },
    #[error("agent '{agent}' does not expose skill '{skill}' at node '{node}'")]
    SkillUnavailable {
        node: String,
        agent: String,
        skill: String,
    },
    #[error("node '{node}' references unknown tool '{tool}'")]
    UnknownTool { node: String, tool: String },
    #[error("agent '{agent}' does not expose tool '{tool}' at node '{node}'")]
    ToolUnavailable {
        node: String,
        agent: String,
        tool: String,
    },
    #[error("policy denies tool '{tool}' at node '{node}'")]
    ToolDenied { node: String, tool: String },
    #[error("policy denies effect '{effect}' required by tool '{tool}' at node '{node}'")]
    EffectDenied {
        node: String,
        tool: String,
        effect: String,
    },
    #[error("agent '{agent}' cannot provide model capability '{capability}' at node '{node}'")]
    ModelCapabilityUnavailable {
        node: String,
        agent: String,
        capability: String,
    },
    #[error("agent node '{node}' has invalid max_steps {steps}; blueprint maximum is {maximum}")]
    InvalidAgentSteps {
        node: String,
        steps: u32,
        maximum: u32,
    },
    #[error("parallel node '{node}' must have at least two unique branches")]
    InvalidParallelBranches { node: String },
    #[error("parallel node '{node}' has invalid max_parallelism {value}; maximum is {maximum}")]
    InvalidParallelism {
        node: String,
        value: u32,
        maximum: u32,
    },
    #[error("parallel branch '{branch}' at node '{node}' cannot reach join '{join}'")]
    ParallelJoinUnreachable {
        node: String,
        branch: String,
        join: String,
    },
    #[error("node '{node}' references unknown evaluator '{evaluator}'")]
    UnknownEvaluator { node: String, evaluator: String },
    #[error("loop node '{node}' has invalid max_iterations {value}; maximum is {maximum}")]
    InvalidLoopIterations {
        node: String,
        value: u32,
        maximum: u32,
    },
    #[error("node '{node}' references unknown capability '{capability}'")]
    UnknownCapability { node: String, capability: String },
    #[error("policy denies capability '{capability}' at node '{node}'")]
    CapabilityDenied { node: String, capability: String },
    #[error(
        "policy denies effect '{effect}' required by capability '{capability}' at node '{node}'"
    )]
    CapabilityEffectDenied {
        node: String,
        capability: String,
        effect: String,
    },
    #[error("loop node '{node}' requires a workspace-checkpoint capability, got '{capability}'")]
    InvalidCheckpointCapability { node: String, capability: String },
    #[error("end node '{node}' requires an artifact-store capability, got '{capability}'")]
    InvalidArtifactCapability { node: String, capability: String },
    #[error("end node '{node}' has an invalid result source: {reason}")]
    InvalidResultSource { node: String, reason: String },
    #[error("blueprint contains {actual} graph nodes; maximum is {maximum}")]
    TooManyNodes { actual: u32, maximum: u32 },
    #[error("blueprint graph depth is {actual}; maximum is {maximum}")]
    GraphTooDeep { actual: u32, maximum: u32 },
    #[error(transparent)]
    Canonicalization(#[from] CanonicalizationError),
}

#[derive(Debug, Clone)]
pub struct ValidatedBlueprint {
    blueprint: SchedulerBlueprint,
    fingerprint: BlueprintFingerprint,
}

impl ValidatedBlueprint {
    pub fn new(
        blueprint: SchedulerBlueprint,
        catalog: &SchedulerCatalog,
        policy: &PolicyEnvelope,
    ) -> Result<Self, BlueprintValidationError> {
        Validator::new(&blueprint, catalog, policy).validate()?;
        let fingerprint = BlueprintFingerprint::from_blueprint(&blueprint)?;
        Ok(Self {
            blueprint,
            fingerprint,
        })
    }

    pub fn blueprint(&self) -> &SchedulerBlueprint {
        &self.blueprint
    }

    pub fn fingerprint(&self) -> BlueprintFingerprint {
        self.fingerprint
    }
}

struct Validator<'a> {
    blueprint: &'a SchedulerBlueprint,
    catalog: &'a SchedulerCatalog,
    policy: &'a PolicyEnvelope,
}

impl<'a> Validator<'a> {
    fn new(
        blueprint: &'a SchedulerBlueprint,
        catalog: &'a SchedulerCatalog,
        policy: &'a PolicyEnvelope,
    ) -> Self {
        Self {
            blueprint,
            catalog,
            policy,
        }
    }

    fn validate(&self) -> Result<(), BlueprintValidationError> {
        validate_identifier("blueprint", self.blueprint.name.as_str())?;
        self.validate_catalog()?;
        self.validate_policy_references()?;
        validate_limits(&self.policy.hard_limits)?;
        validate_limits(&self.blueprint.limits)?;
        compare_limits(&self.blueprint.limits, &self.policy.hard_limits)?;

        let root = SchedulerGraph {
            entry: self.blueprint.entry.clone(),
            nodes: self.blueprint.nodes.clone(),
        };
        self.validate_graph(&root, "root")?;

        let node_count = recursive_node_count(&root);
        if node_count > self.blueprint.limits.max_graph_nodes {
            return Err(BlueprintValidationError::TooManyNodes {
                actual: node_count,
                maximum: self.blueprint.limits.max_graph_nodes,
            });
        }

        let depth = graph_depth(&root);
        if depth > self.blueprint.limits.max_graph_depth {
            return Err(BlueprintValidationError::GraphTooDeep {
                actual: depth,
                maximum: self.blueprint.limits.max_graph_depth,
            });
        }
        Ok(())
    }

    fn validate_catalog(&self) -> Result<(), BlueprintValidationError> {
        validate_identifier("catalog revision", &self.catalog.revision)
            .map_err(|_| BlueprintValidationError::EmptyCatalogRevision)?;

        for (key, entry) in &self.catalog.agents {
            validate_catalog_key("agent", key.as_str(), entry.id.as_str())?;
            for skill in &entry.available_skills {
                if !self.catalog.skills.contains_key(skill) {
                    return Err(BlueprintValidationError::InvalidCatalogReference {
                        agent: key.as_str().to_string(),
                        kind: "skill",
                        reference: skill.as_str().to_string(),
                    });
                }
            }
            for tool in &entry.available_tools {
                if !self.catalog.tools.contains_key(tool) {
                    return Err(BlueprintValidationError::InvalidCatalogReference {
                        agent: key.as_str().to_string(),
                        kind: "tool",
                        reference: tool.as_str().to_string(),
                    });
                }
            }
        }
        for (key, entry) in &self.catalog.skills {
            validate_catalog_key("skill", key.as_str(), entry.id.as_str())?;
            validate_skill_metadata(
                key.as_str(),
                "summary",
                &entry.summary,
                MAX_SKILL_SUMMARY_CHARS,
            )?;
            validate_skill_metadata(
                key.as_str(),
                "fingerprint",
                &entry.content_fingerprint,
                MAX_FINGERPRINT_CHARS,
            )?;
            for tag in &entry.capability_tags {
                validate_identifier("skill capability tag", tag)?;
            }
        }
        for (key, entry) in &self.catalog.tools {
            validate_catalog_key("tool", key.as_str(), entry.id.as_str())?;
        }
        for (key, entry) in &self.catalog.evaluators {
            validate_catalog_key("evaluator", key.as_str(), entry.id.as_str())?;
        }
        for (key, entry) in &self.catalog.capabilities {
            validate_catalog_key("capability", key.as_str(), entry.id.as_str())?;
        }
        Ok(())
    }

    fn validate_policy_references(&self) -> Result<(), BlueprintValidationError> {
        for tool in &self.policy.allowed_tools {
            if !self.catalog.tools.contains_key(tool) {
                return Err(BlueprintValidationError::InvalidPolicyReference {
                    kind: "tool",
                    reference: tool.as_str().to_string(),
                });
            }
        }
        for capability in &self.policy.allowed_capabilities {
            if !self.catalog.capabilities.contains_key(capability) {
                return Err(BlueprintValidationError::InvalidPolicyReference {
                    kind: "capability",
                    reference: capability.as_str().to_string(),
                });
            }
        }
        Ok(())
    }

    fn validate_graph(
        &self,
        graph: &SchedulerGraph,
        scope: &str,
    ) -> Result<(), BlueprintValidationError> {
        if graph.nodes.is_empty() {
            return Err(BlueprintValidationError::EmptyGraph {
                scope: scope.to_string(),
            });
        }
        validate_identifier("node", graph.entry.as_str())?;
        if !graph.nodes.contains_key(&graph.entry) {
            return Err(BlueprintValidationError::MissingEntry {
                scope: scope.to_string(),
                node: graph.entry.as_str().to_string(),
            });
        }

        for (node_id, node) in &graph.nodes {
            validate_identifier("node", node_id.as_str())?;
            self.validate_node(graph, scope, node_id, node)?;
            for target in outgoing(node) {
                if !graph.nodes.contains_key(target) {
                    return Err(BlueprintValidationError::MissingTarget {
                        scope: scope.to_string(),
                        node: node_id.as_str().to_string(),
                        target: target.as_str().to_string(),
                    });
                }
            }
        }

        reject_cycles(graph, scope)?;
        let reachable = reachable_from(graph, &graph.entry);
        for node_id in graph.nodes.keys() {
            if !reachable.contains(node_id) {
                return Err(BlueprintValidationError::UnreachableNode {
                    scope: scope.to_string(),
                    node: node_id.as_str().to_string(),
                });
            }
        }
        if !graph
            .nodes
            .values()
            .any(|node| matches!(node, NodeSpec::End(_)))
        {
            return Err(BlueprintValidationError::MissingEnd {
                scope: scope.to_string(),
            });
        }
        Ok(())
    }

    fn validate_node(
        &self,
        graph: &SchedulerGraph,
        scope: &str,
        node_id: &NodeId,
        node: &NodeSpec,
    ) -> Result<(), BlueprintValidationError> {
        let node_path = format!("{scope}/{}", node_id.as_str());
        match node {
            NodeSpec::Agent(agent_node) => {
                let agent = self.catalog.agents.get(&agent_node.agent).ok_or_else(|| {
                    BlueprintValidationError::UnknownAgent {
                        node: node_path.clone(),
                        agent: agent_node.agent.as_str().to_string(),
                    }
                })?;
                if agent_node.max_steps == 0
                    || agent_node.max_steps > self.blueprint.limits.max_agent_steps
                {
                    return Err(BlueprintValidationError::InvalidAgentSteps {
                        node: node_path.clone(),
                        steps: agent_node.max_steps,
                        maximum: self.blueprint.limits.max_agent_steps,
                    });
                }
                for skill in &agent_node.skills {
                    if !self.catalog.skills.contains_key(skill) {
                        return Err(BlueprintValidationError::UnknownSkill {
                            node: node_path.clone(),
                            skill: skill.as_str().to_string(),
                        });
                    }
                    if !agent.available_skills.contains(skill) {
                        return Err(BlueprintValidationError::SkillUnavailable {
                            node: node_path.clone(),
                            agent: agent.id.as_str().to_string(),
                            skill: skill.as_str().to_string(),
                        });
                    }
                }
                for tool_id in &agent_node.tools {
                    let tool = self.catalog.tools.get(tool_id).ok_or_else(|| {
                        BlueprintValidationError::UnknownTool {
                            node: node_path.clone(),
                            tool: tool_id.as_str().to_string(),
                        }
                    })?;
                    if !agent.available_tools.contains(tool_id) {
                        return Err(BlueprintValidationError::ToolUnavailable {
                            node: node_path.clone(),
                            agent: agent.id.as_str().to_string(),
                            tool: tool_id.as_str().to_string(),
                        });
                    }
                    if !self.policy.allowed_tools.contains(tool_id) {
                        return Err(BlueprintValidationError::ToolDenied {
                            node: node_path.clone(),
                            tool: tool_id.as_str().to_string(),
                        });
                    }
                    if !self.policy.allowed_effects.contains(&tool.effect) {
                        return Err(BlueprintValidationError::EffectDenied {
                            node: node_path.clone(),
                            tool: tool_id.as_str().to_string(),
                            effect: format!("{:?}", tool.effect),
                        });
                    }
                }
                for capability in &agent_node.required_model_capabilities {
                    if !agent.model_capabilities.contains(capability) {
                        return Err(BlueprintValidationError::ModelCapabilityUnavailable {
                            node: node_path.clone(),
                            agent: agent.id.as_str().to_string(),
                            capability: format!("{capability:?}"),
                        });
                    }
                }
            }
            NodeSpec::Parallel(parallel) => {
                let unique: BTreeSet<&NodeId> = parallel.branches.iter().collect();
                if unique.len() < 2 || unique.len() != parallel.branches.len() {
                    return Err(BlueprintValidationError::InvalidParallelBranches {
                        node: node_path.clone(),
                    });
                }
                let branch_count = parallel.branches.len() as u32;
                let maximum = self.blueprint.limits.max_parallelism.min(branch_count);
                if parallel.max_parallelism == 0 || parallel.max_parallelism > maximum {
                    return Err(BlueprintValidationError::InvalidParallelism {
                        node: node_path.clone(),
                        value: parallel.max_parallelism,
                        maximum,
                    });
                }
                for branch in &parallel.branches {
                    if graph.nodes.contains_key(branch)
                        && graph.nodes.contains_key(&parallel.join)
                        && !path_exists(graph, branch, &parallel.join)
                    {
                        return Err(BlueprintValidationError::ParallelJoinUnreachable {
                            node: node_path.clone(),
                            branch: branch.as_str().to_string(),
                            join: parallel.join.as_str().to_string(),
                        });
                    }
                }
            }
            NodeSpec::Gate(gate) => {
                self.require_evaluator(&node_path, &gate.evaluator)?;
            }
            NodeSpec::Loop(loop_node) => {
                self.require_evaluator(&node_path, &loop_node.evaluator)?;
                if loop_node.max_iterations == 0
                    || loop_node.max_iterations > self.blueprint.limits.max_loop_iterations
                {
                    return Err(BlueprintValidationError::InvalidLoopIterations {
                        node: node_path.clone(),
                        value: loop_node.max_iterations,
                        maximum: self.blueprint.limits.max_loop_iterations,
                    });
                }
                if let Some(capability_id) = &loop_node.checkpoint {
                    let capability =
                        self.catalog
                            .capabilities
                            .get(capability_id)
                            .ok_or_else(|| BlueprintValidationError::UnknownCapability {
                                node: node_path.clone(),
                                capability: capability_id.as_str().to_string(),
                            })?;
                    if !self.policy.allowed_capabilities.contains(capability_id) {
                        return Err(BlueprintValidationError::CapabilityDenied {
                            node: node_path.clone(),
                            capability: capability_id.as_str().to_string(),
                        });
                    }
                    if !self.policy.allowed_effects.contains(&capability.effect) {
                        return Err(BlueprintValidationError::CapabilityEffectDenied {
                            node: node_path.clone(),
                            capability: capability_id.as_str().to_string(),
                            effect: format!("{:?}", capability.effect),
                        });
                    }
                    if capability.kind != CapabilityKind::WorkspaceCheckpoint {
                        return Err(BlueprintValidationError::InvalidCheckpointCapability {
                            node: node_path.clone(),
                            capability: capability_id.as_str().to_string(),
                        });
                    }
                }
                self.validate_graph(&loop_node.body, &format!("{node_path}/body"))?;
            }
            NodeSpec::End(end) => match &end.result {
                ResultSource::LastNode => {}
                ResultSource::Named(source) if graph.nodes.contains_key(source) => {}
                ResultSource::Named(source) => {
                    return Err(BlueprintValidationError::InvalidResultSource {
                        node: node_path,
                        reason: format!("named node '{}' does not exist", source.as_str()),
                    });
                }
                ResultSource::Artifact { name, .. } if name.trim().is_empty() => {
                    return Err(BlueprintValidationError::InvalidResultSource {
                        node: node_path,
                        reason: "artifact name is empty".to_string(),
                    });
                }
                ResultSource::Artifact { capability, .. } => {
                    let resolved = self.require_capability(&node_path, capability)?;
                    if resolved.kind != CapabilityKind::ArtifactStore {
                        return Err(BlueprintValidationError::InvalidArtifactCapability {
                            node: node_path,
                            capability: capability.as_str().to_string(),
                        });
                    }
                }
            },
        }
        Ok(())
    }

    fn require_evaluator(
        &self,
        node: &str,
        evaluator: &super::EvaluatorId,
    ) -> Result<(), BlueprintValidationError> {
        if !self.catalog.evaluators.contains_key(evaluator) {
            return Err(BlueprintValidationError::UnknownEvaluator {
                node: node.to_string(),
                evaluator: evaluator.as_str().to_string(),
            });
        }
        Ok(())
    }

    fn require_capability(
        &self,
        node: &str,
        capability_id: &super::CapabilityId,
    ) -> Result<&crate::catalog::CapabilityCatalogEntry, BlueprintValidationError> {
        let capability = self
            .catalog
            .capabilities
            .get(capability_id)
            .ok_or_else(|| BlueprintValidationError::UnknownCapability {
                node: node.to_string(),
                capability: capability_id.as_str().to_string(),
            })?;
        if !self.policy.allowed_capabilities.contains(capability_id) {
            return Err(BlueprintValidationError::CapabilityDenied {
                node: node.to_string(),
                capability: capability_id.as_str().to_string(),
            });
        }
        if !self.policy.allowed_effects.contains(&capability.effect) {
            return Err(BlueprintValidationError::CapabilityEffectDenied {
                node: node.to_string(),
                capability: capability_id.as_str().to_string(),
                effect: format!("{:?}", capability.effect),
            });
        }
        Ok(capability)
    }
}

fn validate_identifier(kind: &'static str, value: &str) -> Result<(), BlueprintValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed != value || value.len() > MAX_IDENTIFIER_LEN {
        return Err(BlueprintValidationError::InvalidIdentifier {
            kind,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_catalog_key(
    kind: &'static str,
    key: &str,
    entry_id: &str,
) -> Result<(), BlueprintValidationError> {
    validate_identifier(kind, key)?;
    validate_identifier(kind, entry_id)?;
    if key != entry_id {
        return Err(BlueprintValidationError::CatalogIdMismatch {
            kind,
            key: key.to_string(),
            entry_id: entry_id.to_string(),
        });
    }
    Ok(())
}

fn validate_skill_metadata(
    skill: &str,
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), BlueprintValidationError> {
    if value.is_empty() || value.trim() != value || value.chars().count() > maximum {
        return Err(BlueprintValidationError::InvalidSkillMetadata {
            skill: skill.to_string(),
            field,
            maximum,
        });
    }
    Ok(())
}

fn validate_limits(limits: &ExecutionLimits) -> Result<(), BlueprintValidationError> {
    for (field, value) in limit_values(limits) {
        if value == 0 {
            return Err(BlueprintValidationError::ZeroLimit { field });
        }
    }
    Ok(())
}

fn compare_limits(
    requested: &ExecutionLimits,
    maximum: &ExecutionLimits,
) -> Result<(), BlueprintValidationError> {
    let requested_values = limit_values(requested);
    let maximum_values = limit_values(maximum);
    for ((field, requested), (_, maximum)) in requested_values.into_iter().zip(maximum_values) {
        if requested > maximum {
            return Err(BlueprintValidationError::LimitExceeded {
                field,
                requested,
                maximum,
            });
        }
    }
    Ok(())
}

fn limit_values(limits: &ExecutionLimits) -> [(&'static str, u64); 9] {
    [
        ("max_model_calls", u64::from(limits.max_model_calls)),
        ("max_tool_calls", u64::from(limits.max_tool_calls)),
        ("max_total_tokens", limits.max_total_tokens),
        ("max_wall_time_ms", limits.max_wall_time_ms),
        ("max_parallelism", u64::from(limits.max_parallelism)),
        ("max_graph_nodes", u64::from(limits.max_graph_nodes)),
        ("max_graph_depth", u64::from(limits.max_graph_depth)),
        ("max_loop_iterations", u64::from(limits.max_loop_iterations)),
        ("max_agent_steps", u64::from(limits.max_agent_steps)),
    ]
}

fn outgoing(node: &NodeSpec) -> Vec<&NodeId> {
    match node {
        NodeSpec::Agent(agent) => vec![&agent.next],
        NodeSpec::Parallel(parallel) => parallel
            .branches
            .iter()
            .chain(std::iter::once(&parallel.join))
            .collect(),
        NodeSpec::Gate(gate) => vec![&gate.on_pass, &gate.on_fail, &gate.on_indeterminate],
        NodeSpec::Loop(loop_node) => vec![&loop_node.on_satisfied, &loop_node.on_exhausted],
        NodeSpec::End(_) => Vec::new(),
    }
}

fn reachable_from(graph: &SchedulerGraph, start: &NodeId) -> BTreeSet<NodeId> {
    let mut visited = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(node_id) = stack.pop() {
        if !visited.insert(node_id.clone()) {
            continue;
        }
        if let Some(node) = graph.nodes.get(node_id) {
            stack.extend(outgoing(node));
        }
    }
    visited
}

fn reject_cycles(graph: &SchedulerGraph, scope: &str) -> Result<(), BlueprintValidationError> {
    fn visit(
        graph: &SchedulerGraph,
        node_id: &NodeId,
        visiting: &mut BTreeSet<NodeId>,
        visited: &mut BTreeSet<NodeId>,
        scope: &str,
    ) -> Result<(), BlueprintValidationError> {
        if visited.contains(node_id) {
            return Ok(());
        }
        if !visiting.insert(node_id.clone()) {
            return Err(BlueprintValidationError::OrdinaryCycle {
                scope: scope.to_string(),
                node: node_id.as_str().to_string(),
            });
        }
        if let Some(node) = graph.nodes.get(node_id) {
            for target in outgoing(node) {
                if graph.nodes.contains_key(target) {
                    visit(graph, target, visiting, visited, scope)?;
                }
            }
        }
        visiting.remove(node_id);
        visited.insert(node_id.clone());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for node_id in graph.nodes.keys() {
        visit(graph, node_id, &mut visiting, &mut visited, scope)?;
    }
    Ok(())
}

fn path_exists(graph: &SchedulerGraph, start: &NodeId, target: &NodeId) -> bool {
    if start == target {
        return true;
    }
    let mut visited = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(node_id) = stack.pop() {
        if !visited.insert(node_id.clone()) {
            continue;
        }
        let Some(node) = graph.nodes.get(node_id) else {
            continue;
        };
        for next in outgoing(node) {
            if next == target {
                return true;
            }
            stack.push(next);
        }
    }
    false
}

fn recursive_node_count(graph: &SchedulerGraph) -> u32 {
    graph
        .nodes
        .values()
        .fold(graph.nodes.len() as u32, |count, node| {
            count
                + match node {
                    NodeSpec::Loop(loop_node) => recursive_node_count(&loop_node.body),
                    _ => 0,
                }
        })
}

fn graph_depth(graph: &SchedulerGraph) -> u32 {
    fn visit(graph: &SchedulerGraph, node_id: &NodeId, memo: &mut BTreeMap<NodeId, u32>) -> u32 {
        if let Some(depth) = memo.get(node_id) {
            return *depth;
        }
        let Some(node) = graph.nodes.get(node_id) else {
            return 0;
        };
        let nested = match node {
            NodeSpec::Loop(loop_node) => graph_depth(&loop_node.body),
            _ => 0,
        };
        let successor = outgoing(node)
            .into_iter()
            .map(|next| visit(graph, next, memo))
            .max()
            .unwrap_or(0);
        let depth = 1 + nested + successor;
        memo.insert(node_id.clone(), depth);
        depth
    }

    visit(graph, &graph.entry, &mut BTreeMap::new())
}
