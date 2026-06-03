// Generated from crates/agendao-types/src/session.rs. Do not edit by hand.

export const CONTEXT_CLOSURE_GOVERNANCE_LABELS = {
  ready: "ready",
  compacted: "compacted",
  deferred: "deferred",
  blocked: "blocked",
} as const;

export const CONTEXT_CLOSURE_SEVERITY_LABELS = {
  stable: "stable",
  low_change: "low change",
  medium_change: "medium change",
  high_change: "high change",
} as const;

export const CONTEXT_CLOSURE_SOURCE_LABELS = {
  none: "no evidence",
  cache_evidence: "cache evidence",
  surface_evidence: "surface evidence",
  boundary_evidence: "boundary evidence",
} as const;

export const CONTEXT_CLOSURE_STATUS_LABELS = {
  prefix_changed: "prefix changed",
  stable_prefix: "stable prefix",
  boundary_recorded: "boundary recorded",
  boundary_clear: "boundary clear",
  cache_stable: "cache stable",
  cache_explained: "cache explained",
  cache_unexplained: "cache unexplained",
  leak_detected: "leak detected",
  isolated: "isolated",
  not_owner_local: "not owner-local",
  continuity_packet: "packet installed",
  raw_summary_fallback: "legacy summary fallback",
} as const;

export function contextClosureGovernanceStatusLabel(value?: string | null) {
  if (!value) return "--";
  return CONTEXT_CLOSURE_GOVERNANCE_LABELS[value as keyof typeof CONTEXT_CLOSURE_GOVERNANCE_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");
}

export function contextClosureSeverityLabel(value?: string | null) {
  if (!value) return "--";
  return CONTEXT_CLOSURE_SEVERITY_LABELS[value as keyof typeof CONTEXT_CLOSURE_SEVERITY_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");
}

export function contextClosureExplainabilitySourceLabel(value?: string | null) {
  if (!value) return "--";
  return CONTEXT_CLOSURE_SOURCE_LABELS[value as keyof typeof CONTEXT_CLOSURE_SOURCE_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");
}

export function contextClosurePrefixStatusLabel(prefix: { prefix_change_detected: boolean }) {
  return prefix.prefix_change_detected ? CONTEXT_CLOSURE_STATUS_LABELS.prefix_changed : CONTEXT_CLOSURE_STATUS_LABELS.stable_prefix;
}

export function contextClosureBoundaryStatusLabel(boundary: { boundary_recorded: boolean }) {
  return boundary.boundary_recorded ? CONTEXT_CLOSURE_STATUS_LABELS.boundary_recorded : CONTEXT_CLOSURE_STATUS_LABELS.boundary_clear;
}

export function contextClosureCacheStatusLabel(cache: { issue_present: boolean; explained: boolean }) {
  if (!cache.issue_present) return CONTEXT_CLOSURE_STATUS_LABELS.cache_stable;
  return cache.explained ? CONTEXT_CLOSURE_STATUS_LABELS.cache_explained : CONTEXT_CLOSURE_STATUS_LABELS.cache_unexplained;
}

export function contextClosureIsolationStatusLabel(isolation: { child_history_in_live_prefix_detected: boolean; owner_local_live_prefix: boolean }) {
  if (isolation.child_history_in_live_prefix_detected) return CONTEXT_CLOSURE_STATUS_LABELS.leak_detected;
  return isolation.owner_local_live_prefix ? CONTEXT_CLOSURE_STATUS_LABELS.isolated : CONTEXT_CLOSURE_STATUS_LABELS.not_owner_local;
}

export function compactionContinuitySourceLabel(continuity: { source: string }) {
  if (continuity.source === "continuity_packet") return CONTEXT_CLOSURE_STATUS_LABELS.continuity_packet;
  if (continuity.source === "raw_summary_fallback") return CONTEXT_CLOSURE_STATUS_LABELS.raw_summary_fallback;
  return continuity.source.replace(/[._-]+/g, " ").trim() || "--";
}

export function contextClosureCoarseDiagnosticLabel(contract: {
  prefix_stability: { prefix_change_detected: boolean };
  compaction_boundary: { boundary_recorded: boolean };
  cache_explainability: { issue_present: boolean; explained: boolean };
} | null | undefined) {
  if (!contract) return null;
  const parts: string[] = [];
  if (contract.cache_explainability.issue_present) {
    parts.push(contextClosureCacheStatusLabel(contract.cache_explainability));
  }
  if (contract.prefix_stability.prefix_change_detected) {
    parts.push(contextClosurePrefixStatusLabel(contract.prefix_stability));
  } else if (contract.compaction_boundary.boundary_recorded) {
    parts.push(contextClosureBoundaryStatusLabel(contract.compaction_boundary));
  }
  return parts.length === 0 ? null : Array.from(new Set(parts)).join(" · ");
}
