export interface CacheBustSummaryRecord {
  status?: string | null;
  severity?: string | null;
  primary_cause?: string | null;
  change_count?: number | null;
}

export interface PromptSurfaceInvalidationRecord {
  severity?: string | null;
  reason?: string | null;
  changed_fields?: string[] | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function severityLabel(value: string | null | undefined) {
  switch (value) {
    case "HardBust":
    case "hard_bust":
    case "hardBust":
      return "hard bust";
    case "LikelyBust":
    case "likely_bust":
    case "likelyBust":
      return "likely bust";
    case "SoftDegradation":
    case "soft_degradation":
    case "softDegradation":
      return "soft degradation";
    case "Stable":
    case "stable":
      return "stable";
    default:
      return value?.replace(/[_-]+/g, " ").trim() || null;
  }
}

export function cacheBustSummaryFromMetadata(
  metadata: Record<string, unknown> | null | undefined,
): CacheBustSummaryRecord | null {
  const summary = metadata?.cache_bust_summary;
  if (!isRecord(summary)) return null;
  return {
    status: typeof summary.status === "string" ? summary.status : null,
    severity: typeof summary.severity === "string" ? summary.severity : null,
    primary_cause: typeof summary.primary_cause === "string" ? summary.primary_cause : null,
    change_count: typeof summary.change_count === "number" ? summary.change_count : null,
  };
}

export function cacheBustSummaryLabel(summary: CacheBustSummaryRecord | null | undefined) {
  if (!summary) return null;
  if (summary.status === "stable" || summary.status === "cold_start") return null;

  const severity = severityLabel(summary.severity);
  if (!severity || severity === "stable") return null;

  const cause = summary.primary_cause?.replace(/\s+/g, " ").trim() || "prompt surface changed";
  return `${severity} · ${cause}`;
}

export function promptSurfaceInvalidationFromTelemetry(
  telemetry: Record<string, unknown> | null | undefined,
): PromptSurfaceInvalidationRecord | null {
  const invalidation = telemetry?.prompt_surface_snapshot_invalidation;
  if (!isRecord(invalidation)) return null;
  return {
    severity:
      typeof invalidation.severity === "string" ? invalidation.severity : null,
    reason: typeof invalidation.reason === "string" ? invalidation.reason : null,
    changed_fields: Array.isArray(invalidation.changed_fields)
      ? invalidation.changed_fields.filter(
          (value): value is string => typeof value === "string",
        )
      : null,
  };
}
