import type { OutputField, OutputPreview } from "./history";

export type LiveExecutionKind = "tool" | "skill";

export interface LiveExecutionEntry {
  id: string;
  label: string;
  status: string;
  kind: LiveExecutionKind;
  summary: string | null;
  fields: OutputField[];
  preview: OutputPreview | null;
  toolCallId: string | null;
  stageId: string | null;
  updatedAt: number;
}

export interface PartitionedLiveExecutions<T> {
  current: T[];
  recent: T[];
}

export function canonicalLiveExecutionStatus(status?: string | null) {
  const normalized = status?.trim().toLowerCase() || "running";
  switch (normalized) {
    case "start":
    case "running":
      return "running";
    case "done":
    case "result":
    case "end":
    case "full":
    case "snapshot":
      return "done";
    case "error":
      return "error";
    default:
      return normalized;
  }
}

export function isActiveLiveExecutionStatus(status?: string | null) {
  return canonicalLiveExecutionStatus(status) === "running";
}

export function partitionLiveExecutions<T extends { status: string; updatedAt: number }>(
  entries: T[],
  {
    currentLimit = 4,
    recentLimit = 6,
  }: {
    currentLimit?: number;
    recentLimit?: number;
  } = {},
): PartitionedLiveExecutions<T> {
  const sorted = [...entries].sort((left, right) => right.updatedAt - left.updatedAt);
  const current = sorted
    .filter((entry) => isActiveLiveExecutionStatus(entry.status))
    .slice(0, currentLimit);
  const recent = sorted
    .filter((entry) => !isActiveLiveExecutionStatus(entry.status))
    .slice(0, recentLimit);
  return { current, recent };
}
