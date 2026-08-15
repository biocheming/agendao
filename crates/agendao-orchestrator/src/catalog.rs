use super::blueprint::{AgentId, CapabilityId, EvaluatorId, ModelCapability, SkillId, ToolId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerCatalog {
    pub revision: String,
    pub agents: BTreeMap<AgentId, AgentCatalogEntry>,
    pub skills: BTreeMap<SkillId, SkillCatalogEntry>,
    pub tools: BTreeMap<ToolId, ToolCatalogEntry>,
    pub evaluators: BTreeMap<EvaluatorId, EvaluatorCatalogEntry>,
    pub capabilities: BTreeMap<CapabilityId, CapabilityCatalogEntry>,
}

impl SchedulerCatalog {
    pub fn fingerprint(&self) -> Result<CatalogFingerprint, serde_json::Error> {
        let bytes = serde_json::to_vec(self)?;
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        Ok(CatalogFingerprint(digest))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogEntry {
    pub id: AgentId,
    pub system_policy: String,
    pub available_skills: BTreeSet<SkillId>,
    pub available_tools: BTreeSet<ToolId>,
    pub model_capabilities: BTreeSet<ModelCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogEntry {
    pub id: SkillId,
    pub summary: String,
    pub content_fingerprint: String,
    pub capability_tags: BTreeSet<String>,
    /// Populated only after selection. It is intentionally omitted from the
    /// serialized catalog; `content_fingerprint` carries its cache identity.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub hydrated_prompt: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCatalogEntry {
    pub id: ToolId,
    pub effect: EffectClass,
    pub permission: PermissionClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectClass {
    ReadOnly,
    WorkspaceMutation,
    ProcessExecution,
    Network,
    ExternalMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionClass {
    Automatic,
    Ask,
    DenyByDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorCatalogEntry {
    pub id: EvaluatorId,
    pub kind: EvaluatorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluatorKind {
    Deterministic,
    ModelJudge,
    Metric,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogEntry {
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    pub effect: EffectClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    WorkspaceCheckpoint,
    ArtifactStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogFingerprint([u8; 32]);

impl fmt::Display for CatalogFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
