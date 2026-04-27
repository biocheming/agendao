import {
  CONTEXT_PRESSURE_LABELS,
  CONTEXT_PRESSURE_THRESHOLDS,
} from "../generated/contextPressure.generated";

export { CONTEXT_PRESSURE_LABELS, CONTEXT_PRESSURE_THRESHOLDS };

export type ContextPressureTone = "normal" | "warning" | "critical";

export function contextPressureLabel(percent?: number | null) {
  if (percent == null) return null;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.critical) return CONTEXT_PRESSURE_LABELS.critical;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.autoCompactSoon) return CONTEXT_PRESSURE_LABELS.autoCompactSoon;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.warning) return CONTEXT_PRESSURE_LABELS.warning;
  return null;
}

export function contextPressureTone(percent?: number | null): ContextPressureTone {
  if (percent == null) return "normal";
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.critical) return "critical";
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.warning) return "warning";
  return "normal";
}

export function contextUsagePercent(used?: number | null, limit?: number | null) {
  if (
    typeof used !== "number" ||
    typeof limit !== "number" ||
    !Number.isFinite(used) ||
    !Number.isFinite(limit) ||
    limit <= 0
  ) {
    return null;
  }
  return Math.round((used / limit) * 100);
}

export function isLiveStageStatus(status?: string | null): boolean {
  return status === "running" || status === "waiting" || status === "retrying" || status === "blocked" || status === "cancelling";
}

export function currentContextTokensFromSources(
  usageContextTokens?: number | null,
  activeStageContextTokens?: number | null,
) {
  const usage =
    typeof usageContextTokens === "number" && Number.isFinite(usageContextTokens) && usageContextTokens > 0
      ? usageContextTokens
      : null;
  const activeStage =
    typeof activeStageContextTokens === "number" && Number.isFinite(activeStageContextTokens) && activeStageContextTokens > 0
      ? activeStageContextTokens
      : null;

  if (usage != null && activeStage != null) return Math.max(usage, activeStage);
  return usage ?? activeStage;
}
