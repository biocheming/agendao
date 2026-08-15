mod audit;
mod composition;
mod distribution;
mod evolution;
mod guard;
mod index;
mod relationships;
mod semantic;
mod store;
mod sync;
mod write;

use crate::{
    SkillArtifactStore, SkillAuthority, SkillDistributionResolver, SkillError, SkillGuardEngine,
    SkillHubStore, SkillLifecycleCoordinator, SkillSyncPlanner, SkillWriteResult,
};
use agendao_config::ConfigStore;
use agendao_types::{
    SkillCapabilityMemberRole, SkillGovernanceDiagnosticSeverity, SkillGuardReport, SkillSyncPlan,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedWriteResult {
    #[serde(flatten)]
    pub result: SkillWriteResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_report: Option<SkillGuardReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillGovernedSyncResult {
    pub plan: SkillSyncPlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guard_reports: Vec<SkillGuardReport>,
}

#[derive(Clone)]
pub struct SkillGovernanceAuthority {
    skill_authority: SkillAuthority,
    hub_store: Arc<SkillHubStore>,
    sync_planner: Arc<SkillSyncPlanner>,
    guard_engine: Arc<SkillGuardEngine>,
    distribution_resolver: Arc<SkillDistributionResolver>,
    artifact_store: Arc<SkillArtifactStore>,
    lifecycle: Arc<SkillLifecycleCoordinator>,
    config_store: Option<Arc<ConfigStore>>,
}

#[derive(Debug, Clone, Default)]
struct SkillCompositionConsumptionContext {
    canonical_skill_name: Option<String>,
    canonical_family_id: Option<String>,
    family_member_role: Option<SkillCapabilityMemberRole>,
    complementary_group_ids: Vec<String>,
    complementary_peer_skill_names: Vec<String>,
}

impl SkillCompositionConsumptionContext {
    fn complementary_protected(&self) -> bool {
        !self.complementary_group_ids.is_empty() || !self.complementary_peer_skill_names.is_empty()
    }

    fn related_skill_name_for_review(&self, skill_name: &str) -> Option<String> {
        let canonical = self.canonical_skill_name.as_deref()?;
        (!canonical.eq_ignore_ascii_case(skill_name)).then(|| canonical.to_string())
    }
}

impl SkillGovernanceAuthority {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        let base_dir = base_dir.into();
        Self {
            skill_authority: SkillAuthority::new(base_dir.clone(), config_store.clone()),
            hub_store: Arc::new(SkillHubStore::new(base_dir.clone())),
            sync_planner: Arc::new(SkillSyncPlanner::new()),
            guard_engine: Arc::new(SkillGuardEngine::new()),
            distribution_resolver: Arc::new(SkillDistributionResolver::new()),
            artifact_store: Arc::new(SkillArtifactStore::new(base_dir, config_store.clone())),
            lifecycle: Arc::new(SkillLifecycleCoordinator::new()),
            config_store,
        }
    }

    pub fn skill_authority(&self) -> &SkillAuthority {
        &self.skill_authority
    }

    pub fn hub_store(&self) -> Arc<SkillHubStore> {
        Arc::clone(&self.hub_store)
    }

    pub fn sync_planner(&self) -> Arc<SkillSyncPlanner> {
        Arc::clone(&self.sync_planner)
    }

    pub fn guard_engine(&self) -> Arc<SkillGuardEngine> {
        Arc::clone(&self.guard_engine)
    }

    pub fn distribution_resolver(&self) -> Arc<SkillDistributionResolver> {
        Arc::clone(&self.distribution_resolver)
    }

    pub fn artifact_store(&self) -> Arc<SkillArtifactStore> {
        Arc::clone(&self.artifact_store)
    }

    pub fn lifecycle(&self) -> Arc<SkillLifecycleCoordinator> {
        Arc::clone(&self.lifecycle)
    }
}

fn normalize_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn set_intersection_count(left: &BTreeSet<String>, right: &BTreeSet<String>) -> usize {
    left.intersection(right).count()
}

fn skill_diagnostic_sort_key(severity: SkillGovernanceDiagnosticSeverity) -> u8 {
    match severity {
        SkillGovernanceDiagnosticSeverity::Warn => 0,
        SkillGovernanceDiagnosticSeverity::Info => 1,
    }
}

fn required_nonempty_text(value: &str, field_name: &str) -> Result<String, SkillError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SkillError::InvalidSkillContent {
            message: format!("{field_name} cannot be empty"),
        });
    }
    Ok(trimmed.to_string())
}

fn dedupe_string_reasons(reasons: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for reason in reasons {
        let normalized = normalize_name(&reason);
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        deduped.push(reason);
    }
    deduped
}
