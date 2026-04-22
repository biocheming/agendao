export interface SkillCatalogEntry {
  name: string;
  description: string;
  category?: string | null;
  location: string;
  writable: boolean;
  supporting_files: string[];
}

export interface SkillFileRefRecord {
  relative_path: string;
  location: string;
}

export interface LoadedSkillMetaRecord {
  name: string;
  description: string;
  category?: string | null;
  location: string;
  supporting_files: SkillFileRefRecord[];
}

export interface LoadedSkillRecord {
  meta: LoadedSkillMetaRecord;
  content: string;
}

export interface SkillDetailResponseRecord {
  skill: LoadedSkillRecord;
  source: string;
  writable: boolean;
}

export interface SkillGovernanceWriteResultRecord {
  action: string;
  skill_name: string;
  location: string;
  supporting_file?: string | null;
}

export interface SkillMethodologyStepRecord {
  title: string;
  action: string;
  outcome?: string | null;
}

export interface SkillMethodologyReferenceRecord {
  label: string;
  path: string;
}

export interface SkillMethodologyTemplateRecord {
  when_to_use: string[];
  when_not_to_use: string[];
  prerequisites: string[];
  core_steps: SkillMethodologyStepRecord[];
  success_criteria: string[];
  validation: string[];
  pitfalls: string[];
  references: SkillMethodologyReferenceRecord[];
}

export interface SkillMethodologyPreviewResponseRecord {
  body: string;
}

export interface SkillMethodologyExtractResponseRecord {
  matched: boolean;
  methodology?: SkillMethodologyTemplateRecord | null;
}

export interface SkillGuardViolationRecord {
  rule_id: string;
  severity: "info" | "warn" | "error";
  message: string;
  file_path?: string | null;
}

export interface SkillGuardReportRecord {
  skill_name: string;
  status: "passed" | "warn" | "blocked";
  violations: SkillGuardViolationRecord[];
  scanned_at: number;
}

export interface SkillSourceRefRecord {
  source_id: string;
  source_kind: "bundled" | "local_path" | "git" | "archive" | "registry";
  locator: string;
  revision?: string | null;
}

export interface ManagedSkillRecord {
  skill_name: string;
  source?: SkillSourceRefRecord | null;
  installed_revision?: string | null;
  local_hash?: string | null;
  last_synced_at?: number | null;
  locally_modified: boolean;
  deleted_locally: boolean;
}

export interface SkillSourceIndexEntryRecord {
  skill_name: string;
  description?: string | null;
  category?: string | null;
  revision?: string | null;
}

export interface SkillSourceIndexSnapshotRecord {
  source: SkillSourceRefRecord;
  updated_at: number;
  entries: SkillSourceIndexEntryRecord[];
}

export interface SkillAuditEventRecord {
  event_id: string;
  kind: string;
  skill_name?: string | null;
  source_id?: string | null;
  actor: string;
  created_at: number;
  payload: unknown;
}

export interface SkillHubManagedResponseRecord {
  managed_skills: ManagedSkillRecord[];
}

export interface SkillHubIndexResponseRecord {
  source_indices: SkillSourceIndexSnapshotRecord[];
}

export interface SkillHubIndexRefreshResponseRecord {
  snapshot: SkillSourceIndexSnapshotRecord;
}

export interface SkillArtifactRefRecord {
  artifact_id: string;
  kind: string;
  locator: string;
  checksum?: string | null;
  size_bytes?: number | null;
}

export interface SkillDistributionReleaseRecord {
  version?: string | null;
  revision?: string | null;
  checksum?: string | null;
  manifest_path?: string | null;
  published_at?: number | null;
}

export interface SkillDistributionResolutionRecord {
  resolved_at: number;
  resolver_kind: string;
  artifact: SkillArtifactRefRecord;
}

export interface SkillInstalledDistributionRecord {
  installed_at: number;
  workspace_skill_path: string;
  installed_revision?: string | null;
  local_hash?: string | null;
}

export interface SkillDistributionRecord {
  distribution_id: string;
  source: SkillSourceRefRecord;
  skill_name: string;
  release: SkillDistributionReleaseRecord;
  resolution: SkillDistributionResolutionRecord;
  installed?: SkillInstalledDistributionRecord | null;
  lifecycle: string;
}

export interface SkillManagedLifecycleRecord {
  distribution_id: string;
  source_id: string;
  skill_name: string;
  state: string;
  updated_at: number;
  error?: string | null;
}

export interface SkillHubDistributionResponseRecord {
  distributions: SkillDistributionRecord[];
}

export interface SkillArtifactCacheEntryRecord {
  artifact: SkillArtifactRefRecord;
  cached_at: number;
  local_path: string;
  extracted_path?: string | null;
  status: string;
  error?: string | null;
}

export interface SkillHubArtifactCacheResponseRecord {
  artifact_cache: SkillArtifactCacheEntryRecord[];
}

export interface SkillHubPolicyRecord {
  artifact_cache_retention_seconds: number;
  fetch_timeout_ms: number;
  max_download_bytes: number;
  max_extract_bytes: number;
}

export interface SkillHubPolicyResponseRecord {
  policy: SkillHubPolicyRecord;
}

export interface SkillHubLifecycleResponseRecord {
  lifecycle: SkillManagedLifecycleRecord[];
}

export interface SkillHubAuditResponseRecord {
  audit_events: SkillAuditEventRecord[];
}

export interface SkillGovernanceTimelineEntryRecord {
  entry_id: string;
  kind: string;
  created_at: number;
  skill_name?: string | null;
  source_id?: string | null;
  actor?: string | null;
  title: string;
  summary: string;
  status: "info" | "success" | "warn" | "error";
  managed_record?: ManagedSkillRecord | null;
  guard_report?: SkillGuardReportRecord | null;
}

export interface SkillHubTimelineResponseRecord {
  entries: SkillGovernanceTimelineEntryRecord[];
}

export interface SkillSyncEntryRecord {
  skill_name: string;
  action: string;
  reason: string;
}

export interface SkillSyncPlanRecord {
  source_id: string;
  entries: SkillSyncEntryRecord[];
}

export interface SkillHubSyncPlanResponseRecord {
  plan: SkillSyncPlanRecord;
  guard_reports?: SkillGuardReportRecord[];
}

export interface SkillHubGuardRunRequestRecord {
  skill_name?: string;
  source?: SkillSourceRefRecord;
}

export interface SkillHubGuardRunResponseRecord {
  reports: SkillGuardReportRecord[];
}

export interface SkillRemoteInstallEntryRecord {
  distribution_id: string;
  source_id: string;
  skill_name: string;
  action: "install" | "update";
  reason: string;
}

export interface SkillRemoteInstallPlanRecord {
  source_id: string;
  distribution: SkillDistributionRecord;
  entry: SkillRemoteInstallEntryRecord;
}

export interface SkillManageResponseRecord {
  result: SkillGovernanceWriteResultRecord;
  guard_report?: SkillGuardReportRecord | null;
}

export interface SkillRemoteInstallResponseRecord {
  plan: SkillRemoteInstallPlanRecord;
  artifact_cache: SkillArtifactCacheEntryRecord;
  guard_report?: SkillGuardReportRecord | null;
  result: SkillGovernanceWriteResultRecord;
}

export interface SkillHubManagedDetachResponseRecord {
  lifecycle: SkillManagedLifecycleRecord;
}

export interface SkillHubManagedRemoveResponseRecord {
  lifecycle: SkillManagedLifecycleRecord;
  deleted_from_workspace: boolean;
  result?: SkillGovernanceWriteResultRecord | null;
}
