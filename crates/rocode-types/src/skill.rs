use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Bundled,
    LocalPath,
    Git,
    Archive,
    Registry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSourceRef {
    pub source_id: String,
    pub source_kind: SkillSourceKind,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSourceIndexEntry {
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSourceIndexSnapshot {
    pub source: SkillSourceRef,
    pub updated_at: i64,
    #[serde(default)]
    pub entries: Vec<SkillSourceIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillDistributionResolverKind {
    Bundled,
    LocalPath,
    RegistryIndex,
    RegistryManifest,
    ArchiveManifest,
    GitCheckout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillArtifactKind {
    RegistryPackage,
    GitCheckout,
    Archive,
    LocalSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillArtifactRef {
    pub artifact_id: String,
    pub kind: SkillArtifactKind,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDistributionRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDistributionResolution {
    pub resolved_at: i64,
    pub resolver_kind: SkillDistributionResolverKind,
    pub artifact: SkillArtifactRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInstalledDistribution {
    pub installed_at: i64,
    pub workspace_skill_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillManagedLifecycleState {
    Indexed,
    Resolved,
    Fetched,
    PlannedInstall,
    Installed,
    UpdateAvailable,
    Diverged,
    Detached,
    RemovePending,
    Removed,
    ResolutionFailed,
    FetchFailed,
    ApplyFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDistributionRecord {
    pub distribution_id: String,
    pub source: SkillSourceRef,
    pub skill_name: String,
    pub release: SkillDistributionRelease,
    pub resolution: SkillDistributionResolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<SkillInstalledDistribution>,
    pub lifecycle: SkillManagedLifecycleState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillArtifactCacheStatus {
    Fetched,
    Extracted,
    Failed,
    Evicted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillArtifactCacheEntry {
    pub artifact: SkillArtifactRef,
    pub cached_at: i64,
    pub local_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_path: Option<String>,
    pub status: SkillArtifactCacheStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillManagedLifecycleRecord {
    pub distribution_id: String,
    pub source_id: String,
    pub skill_name: String,
    pub state: SkillManagedLifecycleState,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundledSkillManifestEntry {
    pub skill_name: String,
    pub relative_path: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundledSkillManifest {
    pub bundle_id: String,
    #[serde(default)]
    pub entries: Vec<BundledSkillManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedSkillRecord {
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillSourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    #[serde(default)]
    pub locally_modified: bool,
    #[serde(default)]
    pub deleted_locally: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillOperationalSourceScope {
    WorkspaceLocal,
    Managed,
    DiscoveredReadOnly,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillWriteLedgerAction {
    Create,
    Patch,
    Edit,
    WriteFile,
    RemoveFile,
    Install,
    Update,
    Detach,
    Remove,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillUsageLedgerEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub runtime_use_count: u64,
    #[serde(default)]
    pub runtime_success_count: u64,
    #[serde(default)]
    pub runtime_error_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillWriteLedgerEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_written_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_write_at: Option<i64>,
    #[serde(default)]
    pub create_count: u64,
    #[serde(default)]
    pub patch_count: u64,
    #[serde(default)]
    pub edit_count: u64,
    #[serde(default)]
    pub supporting_file_write_count: u64,
    #[serde(default)]
    pub supporting_file_remove_count: u64,
    #[serde(default)]
    pub install_count: u64,
    #[serde(default)]
    pub update_count: u64,
    #[serde(default)]
    pub detach_count: u64,
    #[serde(default)]
    pub remove_count: u64,
    #[serde(default)]
    pub delete_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action: Option<SkillWriteLedgerAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_supporting_file: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillVitalityState {
    #[default]
    Active,
    ReviewCandidate,
    Retired,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillRetirementReasonKind {
    NegativeEntropy,
    SemanticConflict,
    ManualOverride,
    Restored,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRetirementReason {
    pub kind: SkillRetirementReasonKind,
    pub summary: String,
    pub noted_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillVitalityRecord {
    pub state: SkillVitalityState,
    pub updated_at: i64,
    pub reason: SkillRetirementReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillEvolutionEvidenceSummary {
    #[serde(default)]
    pub memory_promotion_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_memory_promotion_at: Option<i64>,
    #[serde(default)]
    pub proposal_signal_count: u64,
    #[serde(default)]
    pub last_observed_draft_proposal_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_proposal_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_positive_signal_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillOperationalSnapshot {
    pub skill_name: String,
    #[serde(default)]
    pub source_scope: SkillOperationalSourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SkillUsageLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writes: Option<SkillWriteLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evolution: Option<SkillEvolutionEvidenceSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vitality: Option<SkillVitalityRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillRelationshipKind {
    #[default]
    RedundantOverlap,
    SpecializationVariant,
    ComplementaryComponent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillRelationshipState {
    #[default]
    Observed,
    Accepted,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillRelationshipEdge {
    pub left_skill_name: String,
    pub right_skill_name: String,
    pub relation_kind: SkillRelationshipKind,
    #[serde(default)]
    pub state: SkillRelationshipState,
    #[serde(default)]
    pub score: u16,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillCapabilityGroupKind {
    #[default]
    CanonicalFamily,
    ComplementaryBundle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillCapabilityMemberRole {
    #[default]
    Canonical,
    Specialization,
    Complementary,
    MergeCandidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillCapabilityMember {
    pub skill_name: String,
    #[serde(default)]
    pub role: SkillCapabilityMemberRole,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillCapabilityGroupState {
    #[default]
    Candidate,
    Active,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillCapabilityGroup {
    pub capability_id: String,
    #[serde(default)]
    pub group_kind: SkillCapabilityGroupKind,
    #[serde(default)]
    pub state: SkillCapabilityGroupState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_skill_name: Option<String>,
    #[serde(default)]
    pub members: Vec<SkillCapabilityMember>,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillRuntimeCompositionHintKind {
    #[default]
    PreferCanonicalSkill,
    ComplementaryBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillRuntimeCompositionHint {
    #[serde(default)]
    pub kind: SkillRuntimeCompositionHintKind,
    #[serde(default)]
    pub skill_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSyncAction {
    Install,
    Update,
    SkipLocalModification,
    SkipDeletedLocally,
    RemoveManaged,
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSyncEntry {
    pub skill_name: String,
    pub action: SkillSyncAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSyncPlan {
    pub source_id: String,
    #[serde(default)]
    pub entries: Vec<SkillSyncEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubManagedResponse {
    #[serde(default)]
    pub managed_skills: Vec<ManagedSkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubUsageLedgerResponse {
    #[serde(default)]
    pub entries: Vec<SkillOperationalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionRelationshipsResponse {
    pub generated_at: i64,
    #[serde(default)]
    pub relationships: Vec<SkillRelationshipEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionGroupsResponse {
    pub generated_at: i64,
    #[serde(default)]
    pub groups: Vec<SkillCapabilityGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionRelationshipAcceptRequest {
    pub session_id: String,
    pub left_skill_name: String,
    pub right_skill_name: String,
    pub relation_kind: SkillRelationshipKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionRelationshipDismissRequest {
    pub session_id: String,
    pub left_skill_name: String,
    pub right_skill_name: String,
    pub relation_kind: SkillRelationshipKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionRelationshipWriteResponse {
    pub relationship: SkillRelationshipEdge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionGroupCreateRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    pub group_kind: SkillCapabilityGroupKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_skill_name: Option<String>,
    #[serde(default)]
    pub members: Vec<SkillCapabilityMember>,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionGroupMemberRoleRequest {
    pub session_id: String,
    pub capability_id: String,
    pub skill_name: String,
    pub role: SkillCapabilityMemberRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionGroupMemberRemoveRequest {
    pub session_id: String,
    pub capability_id: String,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubCompositionGroupWriteResponse {
    pub group: SkillCapabilityGroup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillGovernanceDiagnosticSeverity {
    Info,
    Warn,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillNegativeEntropySignal {
    NeverReused,
    StaleUnused,
    WriteHeavyLowReuse,
    DormantManaged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillNegativeEntropyDiagnostic {
    pub skill_name: String,
    #[serde(default)]
    pub source_scope: SkillOperationalSourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default)]
    pub signals: Vec<SkillNegativeEntropySignal>,
    pub severity: SkillGovernanceDiagnosticSeverity,
    #[serde(default)]
    pub runtime_use_count: u64,
    #[serde(default)]
    pub runtime_error_count: u64,
    #[serde(default)]
    pub write_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_write_at: Option<i64>,
    #[serde(default)]
    pub semantic_overlap_count: u64,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubNegativeEntropyResponse {
    pub generated_at: i64,
    #[serde(default)]
    pub candidates: Vec<SkillNegativeEntropyDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubReviewCandidatesSyncRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubReviewCandidatesSyncResponse {
    #[serde(default)]
    pub updated: Vec<SkillOperationalSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSemanticConflictKind {
    NearDuplicate,
    TriggerOverlap,
    ReplacementHint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSemanticConflictDiagnostic {
    pub left_skill_name: String,
    pub right_skill_name: String,
    pub kind: SkillSemanticConflictKind,
    pub severity: SkillGovernanceDiagnosticSeverity,
    #[serde(default)]
    pub score: u16,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_skill_name: Option<String>,
    #[serde(default)]
    pub left_runtime_use_count: u64,
    #[serde(default)]
    pub right_runtime_use_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_last_used_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSemanticConflictResponse {
    pub generated_at: i64,
    #[serde(default)]
    pub conflicts: Vec<SkillSemanticConflictDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubVitalityUpdateRequest {
    pub session_id: String,
    pub skill_name: String,
    pub state: SkillVitalityState,
    pub reason_kind: SkillRetirementReasonKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubVitalityUpdateResponse {
    pub snapshot: SkillOperationalSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubDistributionResponse {
    #[serde(default)]
    pub distributions: Vec<SkillDistributionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubArtifactCacheResponse {
    #[serde(default)]
    pub artifact_cache: Vec<SkillArtifactCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubPolicy {
    pub artifact_cache_retention_seconds: u64,
    pub fetch_timeout_ms: u64,
    pub max_download_bytes: u64,
    pub max_extract_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubPolicyResponse {
    pub policy: SkillHubPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubLifecycleResponse {
    #[serde(default)]
    pub lifecycle: Vec<SkillManagedLifecycleRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillRemoteInstallAction {
    Install,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRemoteInstallEntry {
    pub distribution_id: String,
    pub source_id: String,
    pub skill_name: String,
    pub action: SkillRemoteInstallAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRemoteInstallPlan {
    pub source_id: String,
    pub distribution: SkillDistributionRecord,
    pub entry: SkillRemoteInstallEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRemoteInstallResponse {
    pub plan: SkillRemoteInstallPlan,
    pub artifact_cache: SkillArtifactCacheEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_report: Option<SkillGuardReport>,
    pub result: SkillGovernanceWriteResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubRemoteInstallPlanRequest {
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubRemoteInstallApplyRequest {
    pub session_id: String,
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubRemoteUpdatePlanRequest {
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubRemoteUpdateApplyRequest {
    pub session_id: String,
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGovernanceWriteResult {
    pub action: String,
    pub skill_name: String,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supporting_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubIndexResponse {
    #[serde(default)]
    pub source_indices: Vec<SkillSourceIndexSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillHubSearchRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<SkillSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSearchMatch {
    pub source: SkillSourceRef,
    pub entry: SkillSourceIndexEntry,
    pub source_updated_at: i64,
    pub score: i64,
    #[serde(default)]
    pub match_reasons: Vec<String>,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub locally_modified: bool,
    #[serde(default)]
    pub deleted_locally: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSearchResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default)]
    pub matches: Vec<SkillHubSearchMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubIndexRefreshRequest {
    pub source: SkillSourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubIndexRefreshResponse {
    pub snapshot: SkillSourceIndexSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubManagedDetachRequest {
    pub session_id: String,
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubManagedDetachResponse {
    pub lifecycle: SkillManagedLifecycleRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubManagedRemoveRequest {
    pub session_id: String,
    pub source: SkillSourceRef,
    pub skill_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubManagedRemoveResponse {
    pub lifecycle: SkillManagedLifecycleRecord,
    #[serde(default)]
    pub deleted_from_workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SkillGovernanceWriteResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubAuditResponse {
    #[serde(default)]
    pub audit_events: Vec<SkillAuditEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillHubTimelineQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubTimelineResponse {
    #[serde(default)]
    pub entries: Vec<SkillGovernanceTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSyncPlanRequest {
    pub source: SkillSourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSyncApplyRequest {
    pub session_id: String,
    pub source: SkillSourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubSyncPlanResponse {
    pub plan: SkillSyncPlan,
    #[serde(default)]
    pub guard_reports: Vec<SkillGuardReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubGuardRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillSourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillHubGuardRunResponse {
    #[serde(default)]
    pub reports: Vec<SkillGuardReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillGovernanceTimelineKind {
    ManagedSnapshot,
    CompositionRelationshipAccepted,
    CompositionRelationshipDismissed,
    CapabilityGroupActivated,
    CapabilityGroupMemberRoleUpdated,
    CapabilityGroupMemberRemoved,
    VitalityTransitioned,
    SourceIndexRefreshed,
    SourceResolved,
    ArtifactFetched,
    ArtifactEvicted,
    ArtifactFetchFailed,
    RemoteInstallPlanned,
    RemoteUpdatePlanned,
    LifecycleTransitioned,
    Create,
    Patch,
    Edit,
    Delete,
    WriteFile,
    RemoveFile,
    HubInstall,
    HubUpdate,
    HubDetach,
    HubRemove,
    SyncPlanCreated,
    SyncApplyCompleted,
    GuardBlocked,
    GuardWarned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillGovernanceTimelineStatus {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGovernanceTimelineEntry {
    pub entry_id: String,
    pub kind: SkillGovernanceTimelineKind,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub title: String,
    pub summary: String,
    pub status: SkillGovernanceTimelineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_record: Option<ManagedSkillRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_report: Option<SkillGuardReport>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillGuardSeverity {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillGuardStatus {
    Passed,
    Warn,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGuardViolation {
    pub rule_id: String,
    pub severity: SkillGuardSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillGuardReport {
    pub skill_name: String,
    pub status: SkillGuardStatus,
    #[serde(default)]
    pub violations: Vec<SkillGuardViolation>,
    pub scanned_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAuditKind {
    CompositionRelationshipAccepted,
    CompositionRelationshipDismissed,
    CapabilityGroupActivated,
    CapabilityGroupMemberRoleUpdated,
    CapabilityGroupMemberRemoved,
    VitalityTransitioned,
    SourceIndexRefreshed,
    SourceResolved,
    ArtifactFetched,
    ArtifactEvicted,
    ArtifactFetchFailed,
    RemoteInstallPlanned,
    RemoteUpdatePlanned,
    LifecycleTransitioned,
    Create,
    Patch,
    Edit,
    Delete,
    WriteFile,
    RemoveFile,
    HubInstall,
    HubUpdate,
    HubDetach,
    HubRemove,
    SyncPlanCreated,
    SyncApplyCompleted,
    GuardBlocked,
    GuardWarned,
}

impl From<SkillAuditKind> for SkillGovernanceTimelineKind {
    fn from(value: SkillAuditKind) -> Self {
        match value {
            SkillAuditKind::CompositionRelationshipAccepted => {
                Self::CompositionRelationshipAccepted
            }
            SkillAuditKind::CompositionRelationshipDismissed => {
                Self::CompositionRelationshipDismissed
            }
            SkillAuditKind::CapabilityGroupActivated => Self::CapabilityGroupActivated,
            SkillAuditKind::CapabilityGroupMemberRoleUpdated => {
                Self::CapabilityGroupMemberRoleUpdated
            }
            SkillAuditKind::CapabilityGroupMemberRemoved => Self::CapabilityGroupMemberRemoved,
            SkillAuditKind::VitalityTransitioned => Self::VitalityTransitioned,
            SkillAuditKind::SourceIndexRefreshed => Self::SourceIndexRefreshed,
            SkillAuditKind::SourceResolved => Self::SourceResolved,
            SkillAuditKind::ArtifactFetched => Self::ArtifactFetched,
            SkillAuditKind::ArtifactEvicted => Self::ArtifactEvicted,
            SkillAuditKind::ArtifactFetchFailed => Self::ArtifactFetchFailed,
            SkillAuditKind::RemoteInstallPlanned => Self::RemoteInstallPlanned,
            SkillAuditKind::RemoteUpdatePlanned => Self::RemoteUpdatePlanned,
            SkillAuditKind::LifecycleTransitioned => Self::LifecycleTransitioned,
            SkillAuditKind::Create => Self::Create,
            SkillAuditKind::Patch => Self::Patch,
            SkillAuditKind::Edit => Self::Edit,
            SkillAuditKind::Delete => Self::Delete,
            SkillAuditKind::WriteFile => Self::WriteFile,
            SkillAuditKind::RemoveFile => Self::RemoveFile,
            SkillAuditKind::HubInstall => Self::HubInstall,
            SkillAuditKind::HubUpdate => Self::HubUpdate,
            SkillAuditKind::HubDetach => Self::HubDetach,
            SkillAuditKind::HubRemove => Self::HubRemove,
            SkillAuditKind::SyncPlanCreated => Self::SyncPlanCreated,
            SkillAuditKind::SyncApplyCompleted => Self::SyncApplyCompleted,
            SkillAuditKind::GuardBlocked => Self::GuardBlocked,
            SkillAuditKind::GuardWarned => Self::GuardWarned,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillAuditEvent {
    pub event_id: String,
    pub kind: SkillAuditKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub actor: String,
    pub created_at: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
}
