// Generated from crates/agendao-types/src/context_pressure.rs. Do not edit by hand.

export const CONTEXT_PRESSURE_THRESHOLDS = {
  warning: 80,
  autoCompactSoon: 90,
  critical: 95,
} as const;

export const CONTEXT_PRESSURE_LABELS = {
  warning: "warning",
  autoCompactSoon: "auto-compact soon",
  critical: "compact now",
} as const;

export type ContextPressureLabel = typeof CONTEXT_PRESSURE_LABELS[keyof typeof CONTEXT_PRESSURE_LABELS];

export function contextPressureLabel(percent?: number | null): ContextPressureLabel | null {
  if (percent == null || !Number.isFinite(percent)) return null;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.critical) return CONTEXT_PRESSURE_LABELS.critical;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.autoCompactSoon) return CONTEXT_PRESSURE_LABELS.autoCompactSoon;
  if (percent >= CONTEXT_PRESSURE_THRESHOLDS.warning) return CONTEXT_PRESSURE_LABELS.warning;
  return null;
}
