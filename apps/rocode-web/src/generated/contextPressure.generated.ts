// Generated from crates/rocode-types/src/context_pressure.rs. Do not edit by hand.

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
