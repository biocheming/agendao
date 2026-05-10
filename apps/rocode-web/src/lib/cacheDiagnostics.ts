import {
  CONTEXT_CLOSURE_SEVERITY_LABELS,
  CONTEXT_CLOSURE_STATUS_LABELS,
} from "../generated/contextClosure.generated";

export interface CacheEvidenceSummaryRecord {
  status?: string | null;
  severity?: string | null;
  primary_cause?: string | null;
  change_count?: number | null;
}

export interface PromptSurfaceEvidenceRecord {
  severity?: string | null;
  reason?: string | null;
  changed_fields?: string[] | null;
}

export interface CacheSemanticsRecord {
  label?: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function severityLabel(value: string | null | undefined) {
  switch (value) {
    case "HighChange":
    case "hard_bust":
    case "hardBust":
      return CONTEXT_CLOSURE_SEVERITY_LABELS.high_change;
    case "MediumChange":
    case "likely_bust":
    case "likelyBust":
      return CONTEXT_CLOSURE_SEVERITY_LABELS.medium_change;
    case "LowChange":
    case "soft_degradation":
    case "softDegradation":
      return CONTEXT_CLOSURE_SEVERITY_LABELS.low_change;
    case "Stable":
    case "stable":
      return CONTEXT_CLOSURE_SEVERITY_LABELS.stable;
    default:
      return value?.replace(/[_-]+/g, " ").trim() || null;
  }
}

export function cacheBustSummaryFromMetadata(
  metadata: Record<string, unknown> | null | undefined,
): CacheEvidenceSummaryRecord | null {
  const summary = metadata?.cache_evidence;
  if (!isRecord(summary)) return null;
  return {
    status: typeof summary.status === "string" ? summary.status : null,
    severity: typeof summary.severity === "string" ? summary.severity : null,
    primary_cause: typeof summary.primary_cause === "string" ? summary.primary_cause : null,
    change_count: typeof summary.change_count === "number" ? summary.change_count : null,
  };
}

export function cacheBustSummaryLabel(summary: CacheEvidenceSummaryRecord | null | undefined) {
  if (!summary) return null;
  if (summary.status === "stable" || summary.status === "cold_start") return null;

  const severity = severityLabel(summary.severity);
  if (!severity || severity === "stable") return null;

  const cause = summary.primary_cause?.replace(/\s+/g, " ").trim() || "surface changed";
  return `${severity} · ${cause}`;
}

export function cacheBustSummaryStatusLabel(
  summary: CacheEvidenceSummaryRecord | null | undefined,
) {
  if (!summary) return null;
  if (summary.status === "stable" || summary.status === "cold_start") return null;
  return summary.primary_cause?.trim()
    ? CONTEXT_CLOSURE_STATUS_LABELS.cache_explained
    : CONTEXT_CLOSURE_STATUS_LABELS.cache_unexplained;
}

export function promptSurfaceEvidenceFromTelemetry(
  telemetry: unknown,
): PromptSurfaceEvidenceRecord | null {
  if (!isRecord(telemetry)) return null;
  const evidence = telemetry.prompt_surface_evidence;
  if (!isRecord(evidence)) return null;
  return {
    severity:
      typeof evidence.severity === "string" ? evidence.severity : null,
    reason: typeof evidence.reason === "string" ? evidence.reason : null,
    changed_fields: Array.isArray(evidence.changed_fields)
      ? evidence.changed_fields.filter(
          (value): value is string => typeof value === "string",
        )
      : null,
  };
}

export function cacheSemanticsFromTelemetry(
  telemetry: unknown,
): CacheSemanticsRecord | null {
  if (!isRecord(telemetry)) return null;
  const summary = telemetry.cache_semantics;
  if (!isRecord(summary)) return null;
  return {
    label: typeof summary.label === "string" ? summary.label : null,
  };
}
