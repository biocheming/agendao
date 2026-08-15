use crate::blueprint::{
    AgentId, BlueprintValidationError, CapabilityId, EvaluatorId, SchedulerBlueprint, SkillId,
    ToolId, ValidatedBlueprint,
};
use crate::catalog::SchedulerCatalog;
use crate::policy::PolicyEnvelope;
use crate::templates::{build_template, TemplateError, TemplateId, TemplateParameters};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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

#[derive(Debug, Clone, Default)]
pub struct TaskShape {
    pub simple: bool,
    pub requires_verification: bool,
    pub benefits_from_parallelism: bool,
    pub iterative_research: bool,
}

pub struct SelectionRequest {
    pub explicit: Option<ExplicitSelection>,
    pub locked: Option<ValidatedBlueprint>,
    pub task: TaskShape,
    pub default_parameters: TemplateParameters,
    pub goal: String,
    pub workspace_summary: String,
    pub rejected_blueprint_fingerprints: BTreeSet<String>,
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
    SessionLock,
    Heuristic,
    Planner,
}

#[derive(Debug, Clone)]
pub struct SelectionResult {
    pub blueprint: ValidatedBlueprint,
    pub source: SelectionSource,
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
            return self.validate(draft, SelectionSource::User);
        }
        if let Some(locked) = request.locked {
            return self.validate(locked.blueprint().clone(), SelectionSource::SessionLock);
        }

        let heuristic = if request.task.simple {
            Some(TemplateId::Direct)
        } else if request.task.iterative_research {
            Some(TemplateId::Autoresearch)
        } else if request.task.requires_verification {
            Some(TemplateId::Verify)
        } else if request.task.benefits_from_parallelism {
            Some(TemplateId::Coordinate)
        } else {
            None
        };
        if let Some(template) = heuristic {
            return self.validate(
                build_template(template, &request.default_parameters)?,
                SelectionSource::Heuristic,
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
                rejected_blueprint_fingerprints: request.rejected_blueprint_fingerprints.clone(),
            })
            .await
            .map_err(SelectionError::Planner)?;
        let draft = match decision {
            PlannerDecision::UseTemplate {
                template,
                parameters,
            } => build_template(template, &parameters)?,
            PlannerDecision::CreateBlueprint { blueprint } => blueprint,
        };
        let selected = self.validate(draft, SelectionSource::Planner)?;
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
    ) -> Result<SelectionResult, SelectionError> {
        Ok(SelectionResult {
            blueprint: ValidatedBlueprint::new(draft, self.catalog, self.policy)?,
            source,
        })
    }
}
