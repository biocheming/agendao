import type { ProviderModelRecord } from "./provider";

export const REASONING_EFFORTS = ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"] as const;
export type ReasoningEffort = (typeof REASONING_EFFORTS)[number];

/** `""` is the explicit UI value for inheriting model/provider defaults. */
export function supportedReasoningEfforts(model: ProviderModelRecord | null): string[] {
  if (!model?.capabilities?.reasoning) return [];
  // `variants` names model presets (for example `fast` or `thinking`), not
  // the provider's wire-level reasoning vocabulary.  The server owns that
  // protocol-specific mapping and clamps unsupported levels there, so the
  // composer must not infer capability from unrelated variant names.
  return [...REASONING_EFFORTS];
}

export function reasoningLabel(value: string): string {
  return value.trim() ? (value === "none" ? "Off" : value) : "Auto";
}
