export type ConfigPolicyValidationOwnerRecord =
  | "scheduler"
  | "skill_tree"
  | "provider_profile"
  | "external_adapter";

export type ConfigPolicyValidationScopeKindRecord =
  | "scheduler_path"
  | "skill_tree"
  | "provider"
  | "external_adapter";

export type ConfigPolicyValidationSeverityRecord = "warning" | "error";

export type ConfigPolicyValidationEffectRecord =
  | "soft_fallback"
  | "fail_closed_bootstrap"
  | "fail_closed_request_gate";

export interface ConfigPolicyValidationScopeRecord {
  kind: ConfigPolicyValidationScopeKindRecord;
  subject_id?: string | null;
}

export interface ConfigPolicyValidationItemRecord {
  owner: ConfigPolicyValidationOwnerRecord;
  scope: ConfigPolicyValidationScopeRecord;
  path: string;
  severity: ConfigPolicyValidationSeverityRecord;
  effect: ConfigPolicyValidationEffectRecord;
  code: string;
  message: string;
  fallback?: string | null;
}

export interface ConfigPolicyValidationSnapshotRecord {
  revision: number;
  generated_at_ms: number;
  reports: ConfigPolicyValidationItemRecord[];
}
