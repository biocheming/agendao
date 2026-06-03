export interface RunTailUsageRecord {
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  total_cost: number;
}

export interface RunTailSummary {
  status: string;
  title: string;
  detail: string | null;
  tone: "neutral" | "info" | "success" | "warning" | "danger";
}

export interface BuildRunTailSummaryOptions {
  statusLine?: string | null;
  runtimeStatus?: string | null;
  latestRuntimeError?: string | null;
  awaitingUser?: boolean;
  pendingPermission?: boolean;
  usage?: RunTailUsageRecord | null;
  activeStageName?: string | null;
}

function normalizeStatus(value?: string | null) {
  return value?.trim().toLowerCase() || "";
}

function usageSummary(usage?: RunTailUsageRecord | null) {
  if (!usage) return null;
  return `input ${usage.input_tokens} · output ${usage.output_tokens} · reasoning ${usage.reasoning_tokens} · cost $${usage.total_cost.toFixed(4)}`;
}

export function buildRunTailSummary({
  statusLine = "ready",
  runtimeStatus = null,
  latestRuntimeError = null,
  awaitingUser = false,
  pendingPermission = false,
  usage = null,
  activeStageName = null,
}: BuildRunTailSummaryOptions): RunTailSummary {
  const normalizedRuntimeStatus = normalizeStatus(runtimeStatus);
  const normalizedStatusLine = normalizeStatus(statusLine);
  const effectiveStatus =
    normalizedStatusLine && normalizedStatusLine !== "ready"
      ? normalizedStatusLine
      : normalizedRuntimeStatus || "ready";

  if (latestRuntimeError) {
    return {
      status: "error",
      title: "Run failed",
      detail: latestRuntimeError,
      tone: "danger",
    };
  }

  if (pendingPermission) {
    return {
      status: "awaiting_permission",
      title: "Waiting for permission",
      detail: "Permission approval is required before execution continues.",
      tone: "warning",
    };
  }

  if (awaitingUser) {
    return {
      status: "awaiting_user",
      title: "Waiting for user input",
      detail: "Question response is required before execution continues.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "error") {
    return {
      status: "error",
      title: "Run failed",
      detail: "Execution ended with an error.",
      tone: "danger",
    };
  }

  if (effectiveStatus === "complete") {
    return {
      status: "complete",
      title: "Run complete",
      detail: usageSummary(usage) ?? "Execution completed.",
      tone: "success",
    };
  }

  if (effectiveStatus === "idle") {
    if (usage) {
      return {
        status: "idle",
        title: "Run idle",
        detail: usageSummary(usage),
        tone: "success",
      };
    }
    return {
      status: "idle",
      title: "Session idle",
      detail: "No active execution.",
      tone: "neutral",
    };
  }

  if (effectiveStatus === "running") {
    return {
      status: "running",
      title: "Running",
      detail: activeStageName
        ? `Current stage: ${activeStageName}`
        : "Execution activity is streaming.",
      tone: "info",
    };
  }

  if (effectiveStatus === "retrying") {
    return {
      status: "retrying",
      title: "Retrying",
      detail: "Waiting for automatic retry.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "compacting") {
    return {
      status: "compacting",
      title: "Compacting",
      detail: "Preparing a smaller context window.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "reconnecting") {
    return {
      status: "reconnecting",
      title: "Reconnecting stream",
      detail: "Waiting for the event stream to resume.",
      tone: "warning",
    };
  }

  return {
    status: effectiveStatus || "ready",
    title: effectiveStatus === "ready" || !effectiveStatus ? "Session ready" : "Session status",
    detail: effectiveStatus === "ready" || !effectiveStatus ? "No active execution." : null,
    tone: "neutral",
  };
}
