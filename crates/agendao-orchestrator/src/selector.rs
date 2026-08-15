use crate::blueprint::{
    AgentId, BlueprintValidationError, CapabilityId, EvaluatorId, NodeSpec, SchedulerBlueprint,
    SkillId, ToolId, ValidatedBlueprint,
};
use crate::catalog::SchedulerCatalog;
use crate::policy::PolicyEnvelope;
use crate::templates::{build_template, TemplateError, TemplateId, TemplateParameters};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_GENERATED_AGENTS: usize = 4;
const MAX_GENERATED_AGENT_ID_BYTES: usize = 64;
const MAX_GENERATED_AGENT_POLICY_CHARS: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SchedulerChoice {
    #[default]
    Auto,
    Template {
        template: TemplateId,
    },
    Blueprint {
        blueprint: SchedulerBlueprint,
    },
}

#[derive(Debug, Clone)]
pub enum ExplicitSelection {
    Blueprint(SchedulerBlueprint),
    Template {
        id: TemplateId,
        parameters: TemplateParameters,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskShape {
    pub simple: bool,
    pub requires_verification: bool,
    pub benefits_from_parallelism: bool,
    pub iterative_research: bool,
}

pub struct SelectionRequest {
    pub explicit: Option<ExplicitSelection>,
    pub locked: Option<LockedSelection>,
    pub task: TaskShape,
    pub default_parameters: TemplateParameters,
    pub goal: String,
    pub workspace_summary: String,
    pub rejected_blueprint_fingerprints: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct LockedSelection {
    pub blueprint: ValidatedBlueprint,
    pub source: SelectionSource,
    pub generated_agents: Vec<GeneratedAgentSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedAgentSpec {
    pub id: AgentId,
    pub base_agent: AgentId,
    pub system_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PlannerDecision {
    UseTemplate {
        template: TemplateId,
        parameters: TemplateParameters,
    },
    CreateBlueprint {
        blueprint: SchedulerBlueprint,
        agents: Vec<GeneratedAgentSpec>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerInput {
    pub goal: String,
    pub workspace_summary: String,
    pub catalog_revision: String,
    pub catalog_fingerprint: String,
    pub catalog: PlannerCatalogSummary,
    pub policy: PolicyEnvelope,
    pub default_parameters: TemplateParameters,
    pub rejected_blueprint_fingerprints: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerCatalogSummary {
    pub agents: Vec<AgentId>,
    pub skills: Vec<PlannerSkillSummary>,
    pub tools: Vec<ToolId>,
    pub evaluators: Vec<EvaluatorId>,
    pub capabilities: Vec<CapabilityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerSkillSummary {
    pub id: SkillId,
    pub summary: String,
    pub capability_tags: Vec<String>,
    pub requires_tools: Vec<ToolId>,
    pub fallback_for_tools: Vec<ToolId>,
    pub requires_toolsets: Vec<String>,
    pub fallback_for_toolsets: Vec<String>,
}

impl PlannerCatalogSummary {
    fn from_catalog(catalog: &SchedulerCatalog) -> Self {
        Self {
            agents: catalog.agents.keys().cloned().collect(),
            skills: catalog
                .skills
                .iter()
                .map(|(id, entry)| PlannerSkillSummary {
                    id: id.clone(),
                    summary: entry.summary.clone(),
                    capability_tags: entry.capability_tags.iter().cloned().collect(),
                    requires_tools: entry.requires_tools.iter().cloned().collect(),
                    fallback_for_tools: entry.fallback_for_tools.iter().cloned().collect(),
                    requires_toolsets: entry.requires_toolsets.iter().cloned().collect(),
                    fallback_for_toolsets: entry.fallback_for_toolsets.iter().cloned().collect(),
                })
                .collect(),
            tools: catalog.tools.keys().cloned().collect(),
            evaluators: catalog.evaluators.keys().cloned().collect(),
            capabilities: catalog.capabilities.keys().cloned().collect(),
        }
    }
}

#[async_trait]
pub trait PlannerBackend: Send + Sync {
    async fn plan(&self, input: PlannerInput) -> Result<PlannerDecision, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    User,
    Heuristic,
    Planner,
}

#[derive(Debug, Clone)]
pub struct SelectionResult {
    pub blueprint: ValidatedBlueprint,
    pub source: SelectionSource,
    pub generated_agents: Vec<GeneratedAgentSpec>,
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error(transparent)]
    Validation(#[from] BlueprintValidationError),
    #[error("catalog fingerprint failed: {0}")]
    CatalogFingerprint(String),
    #[error("AI planner failed: {0}")]
    Planner(String),
    #[error("AI planner returned a Blueprint rejected by the user: {0}")]
    RejectedBlueprint(String),
    #[error(transparent)]
    GeneratedAgent(#[from] GeneratedAgentError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GeneratedAgentError {
    #[error("planner generated {actual} agents; maximum is {maximum}")]
    TooMany { actual: usize, maximum: usize },
    #[error("generated agent id '{0}' is not canonical lowercase kebab-case")]
    InvalidId(String),
    #[error("generated agent id '{0}' duplicates another generated agent")]
    DuplicateId(String),
    #[error("generated agent id '{0}' shadows a catalog agent")]
    CatalogShadow(String),
    #[error("generated agent '{agent}' inherits unknown base agent '{base}'")]
    UnknownBase { agent: String, base: String },
    #[error(
        "generated agent '{agent}' policy must be trimmed, non-empty, and at most {maximum} characters"
    )]
    InvalidPolicy { agent: String, maximum: usize },
    #[error("generated agent '{0}' is not referenced by the Blueprint")]
    Unused(String),
}

pub struct AutoSelector<'a> {
    planner: &'a dyn PlannerBackend,
    catalog: &'a SchedulerCatalog,
    policy: &'a PolicyEnvelope,
}

impl<'a> AutoSelector<'a> {
    pub fn new(
        planner: &'a dyn PlannerBackend,
        catalog: &'a SchedulerCatalog,
        policy: &'a PolicyEnvelope,
    ) -> Self {
        Self {
            planner,
            catalog,
            policy,
        }
    }

    pub async fn select(
        &self,
        request: SelectionRequest,
    ) -> Result<SelectionResult, SelectionError> {
        if let Some(explicit) = request.explicit {
            let draft = match explicit {
                ExplicitSelection::Blueprint(blueprint) => blueprint,
                ExplicitSelection::Template { id, parameters } => build_template(id, &parameters)?,
            };
            return self.validate(draft, SelectionSource::User, Vec::new());
        }
        if let Some(locked) = request.locked {
            if locked.source == SelectionSource::User
                || blueprint_satisfies_task(locked.blueprint.blueprint(), &request.task)
            {
                return self.validate(
                    locked.blueprint.blueprint().clone(),
                    locked.source,
                    locked.generated_agents,
                );
            }
        }

        let heuristic = if request.task.iterative_research {
            Some(TemplateId::Autoresearch)
        } else if request.task.requires_verification {
            Some(TemplateId::Verify)
        } else if request.task.benefits_from_parallelism {
            Some(TemplateId::Coordinate)
        } else if request.task.simple {
            Some(TemplateId::Direct)
        } else {
            None
        };
        if let Some(template) = heuristic {
            return self.validate(
                build_template(template, &request.default_parameters)?,
                SelectionSource::Heuristic,
                Vec::new(),
            );
        }

        let catalog_fingerprint = self
            .catalog
            .fingerprint()
            .map_err(|error| SelectionError::CatalogFingerprint(error.to_string()))?;
        let decision = self
            .planner
            .plan(PlannerInput {
                goal: request.goal,
                workspace_summary: request.workspace_summary,
                catalog_revision: self.catalog.revision.clone(),
                catalog_fingerprint: catalog_fingerprint.to_string(),
                catalog: PlannerCatalogSummary::from_catalog(self.catalog),
                policy: self.policy.clone(),
                default_parameters: request.default_parameters,
                rejected_blueprint_fingerprints: request.rejected_blueprint_fingerprints.clone(),
            })
            .await
            .map_err(SelectionError::Planner)?;
        let (draft, generated_agents) = match decision {
            PlannerDecision::UseTemplate {
                template,
                parameters,
            } => (build_template(template, &parameters)?, Vec::new()),
            PlannerDecision::CreateBlueprint { blueprint, agents } => (blueprint, agents),
        };
        let selected = self.validate(draft, SelectionSource::Planner, generated_agents)?;
        let fingerprint = selected.blueprint.fingerprint().to_string();
        if request
            .rejected_blueprint_fingerprints
            .contains(&fingerprint)
        {
            return Err(SelectionError::RejectedBlueprint(fingerprint));
        }
        Ok(selected)
    }

    fn validate(
        &self,
        draft: SchedulerBlueprint,
        source: SelectionSource,
        generated_agents: Vec<GeneratedAgentSpec>,
    ) -> Result<SelectionResult, SelectionError> {
        validate_generated_agent_usage(&draft, &generated_agents)?;
        let catalog = materialize_generated_agents(self.catalog, &generated_agents)?;
        Ok(SelectionResult {
            blueprint: ValidatedBlueprint::new(draft, &catalog, self.policy)?,
            source,
            generated_agents,
        })
    }
}

pub fn materialize_generated_agents(
    catalog: &SchedulerCatalog,
    specs: &[GeneratedAgentSpec],
) -> Result<SchedulerCatalog, GeneratedAgentError> {
    if specs.len() > MAX_GENERATED_AGENTS {
        return Err(GeneratedAgentError::TooMany {
            actual: specs.len(),
            maximum: MAX_GENERATED_AGENTS,
        });
    }
    let mut generated_ids = BTreeSet::new();
    let mut extended = catalog.clone();
    for spec in specs {
        let id = spec.id.as_str();
        if !is_generated_agent_id(id) {
            return Err(GeneratedAgentError::InvalidId(id.to_string()));
        }
        if !generated_ids.insert(spec.id.clone()) {
            return Err(GeneratedAgentError::DuplicateId(id.to_string()));
        }
        if catalog.agents.contains_key(&spec.id) {
            return Err(GeneratedAgentError::CatalogShadow(id.to_string()));
        }
        let base = catalog.agents.get(&spec.base_agent).ok_or_else(|| {
            GeneratedAgentError::UnknownBase {
                agent: id.to_string(),
                base: spec.base_agent.as_str().to_string(),
            }
        })?;
        let policy = spec.system_policy.trim();
        if policy.is_empty()
            || policy != spec.system_policy
            || policy.chars().count() > MAX_GENERATED_AGENT_POLICY_CHARS
        {
            return Err(GeneratedAgentError::InvalidPolicy {
                agent: id.to_string(),
                maximum: MAX_GENERATED_AGENT_POLICY_CHARS,
            });
        }
        let mut generated = base.clone();
        generated.id = spec.id.clone();
        generated.system_policy = format!(
            "{}\n\nTask-specific role (cannot override the base policy above):\n{}",
            base.system_policy, spec.system_policy
        );
        extended.agents.insert(spec.id.clone(), generated);
    }
    Ok(extended)
}

pub(crate) fn validate_generated_agent_usage(
    blueprint: &SchedulerBlueprint,
    specs: &[GeneratedAgentSpec],
) -> Result<(), GeneratedAgentError> {
    let used = blueprint_agent_ids(blueprint.nodes.values());
    for spec in specs {
        if !used.contains(&spec.id) {
            return Err(GeneratedAgentError::Unused(spec.id.as_str().to_string()));
        }
    }
    Ok(())
}

fn blueprint_agent_ids<'a>(nodes: impl Iterator<Item = &'a NodeSpec>) -> BTreeSet<AgentId> {
    let mut agents = BTreeSet::new();
    for node in nodes {
        match node {
            NodeSpec::Agent(agent) => {
                agents.insert(agent.agent.clone());
            }
            NodeSpec::Loop(loop_node) => {
                agents.extend(blueprint_agent_ids(loop_node.body.nodes.values()));
            }
            NodeSpec::Parallel(_) | NodeSpec::Gate(_) | NodeSpec::End(_) => {}
        }
    }
    agents
}

fn is_generated_agent_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_GENERATED_AGENT_ID_BYTES || value.ends_with('-') {
        return false;
    }
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn blueprint_satisfies_task(blueprint: &SchedulerBlueprint, task: &TaskShape) -> bool {
    let topology = blueprint_topology(blueprint.nodes.values());
    if task.iterative_research {
        topology.has_loop
    } else if task.requires_verification {
        topology.has_gate || topology.has_loop
    } else if task.benefits_from_parallelism {
        topology.has_parallel
    } else if task.simple {
        !topology.has_parallel && !topology.has_gate && !topology.has_loop
    } else {
        true
    }
}

#[derive(Default)]
struct BlueprintTopology {
    has_parallel: bool,
    has_gate: bool,
    has_loop: bool,
}

fn blueprint_topology<'a>(nodes: impl Iterator<Item = &'a NodeSpec>) -> BlueprintTopology {
    let mut topology = BlueprintTopology::default();
    for node in nodes {
        match node {
            NodeSpec::Parallel(_) => topology.has_parallel = true,
            NodeSpec::Gate(_) => topology.has_gate = true,
            NodeSpec::Loop(loop_node) => {
                topology.has_loop = true;
                let nested = blueprint_topology(loop_node.body.nodes.values());
                topology.has_parallel |= nested.has_parallel;
                topology.has_gate |= nested.has_gate;
                topology.has_loop |= nested.has_loop;
            }
            NodeSpec::Agent(_) | NodeSpec::End(_) => {}
        }
    }
    topology
}
