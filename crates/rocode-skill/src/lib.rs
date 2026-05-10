mod artifact;
mod audit;
mod authority;
mod catalog;
mod detail;
mod discovery;
mod distribution;
mod errors;
mod governance;
mod guard;
mod hub;
mod lifecycle;
mod methodology;
mod runtime;
mod sync;
mod types;
mod workspace_artifact;
mod write;

pub use artifact::SkillArtifactStore;
pub use authority::{infer_toolsets_from_tools, SkillAuthority, SkillFilter};
pub use catalog::{
    SkillCatalogCache, SkillCatalogSnapshot, SkillDirectorySignature, SkillFileSignature,
    SkillRoot, SkillRootSignature,
};
pub use distribution::SkillDistributionResolver;
pub use errors::SkillError;
pub use governance::{SkillGovernanceAuthority, SkillGovernedSyncResult, SkillGovernedWriteResult};
pub use guard::{SkillGuardEngine, SkillGuardMode};
pub use hub::{SkillHubSnapshot, SkillHubStore};
pub use lifecycle::SkillLifecycleCoordinator;
pub use methodology::{
    extract_methodology_template_from_markdown, render_methodology_skill_body,
    SkillMethodologyReference, SkillMethodologyStep, SkillMethodologyTemplate, SkillQualityRubric,
    SkillQualityRule,
};
pub use runtime::{
    infer_runtime_skill_names, RuntimeInstructionSource, RuntimeSkillBootstrapReport,
    RuntimeSkillMaterialization, RuntimeSkillMaterializationAction, RuntimeSkillPromptBodyKind,
    RuntimeSkillPromptPacket, RuntimeSkillSourceKind, SkillRuntimeResolutionDiagnostic,
    SkillRuntimeResolver,
};
pub use sync::SkillSyncPlanner;
pub use types::{
    LoadedSkill, LoadedSkillFile, SkillCategoryView, SkillConditions, SkillDetailView,
    SkillFileRef, SkillFrontmatter, SkillFrontmatterPatch, SkillHermesMetadata, SkillMeta,
    SkillMetaView, SkillMetadataBlocks, SkillPrerequisites, SkillReadinessStatus,
    SkillRequiredEnvironmentVariable, SkillRocodeMetadata, SkillSummary,
};
pub use workspace_artifact::{
    export_workspace_skill_artifact_bundle, import_workspace_skill_artifact_bundle,
    import_workspace_skill_artifact_bundle_with_legacy_adapter,
    WorkspaceSkillArtifactLegacyAdapter,
};
pub use write::{
    CreateSkillRequest, DeleteSkillRequest, EditSkillRequest, PatchSkillRequest,
    RemoveSkillFileRequest, SkillWriteAction, SkillWriteResult, WriteSkillFileRequest,
};

use rocode_config::ConfigStore;
use std::path::PathBuf;
use std::sync::Arc;

pub fn list_available_skill_views() -> Vec<SkillMetaView> {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_store = ConfigStore::from_project_dir(&base).ok().map(Arc::new);
    let authority = SkillAuthority::new(base, config_store);
    authority.list_skill_meta(None).unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
