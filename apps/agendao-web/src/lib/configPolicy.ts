export type ConfigPolicyValidationOwnerRecord =
  | "provider_profile"
  | "external_adapter";

export type ConfigPolicyValidationScopeKindRecord =
  | "provider"
  | "external_adapter";

export type ConfigPolicyValidationSeverityRecord = "warning" | "error";

export type ConfigPolicyValidationEffectRecord =
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
}

export interface ConfigPolicyValidationSnapshotRecord {
  revision: number;
  generated_at_ms: number;
  reports: ConfigPolicyValidationItemRecord[];
}
