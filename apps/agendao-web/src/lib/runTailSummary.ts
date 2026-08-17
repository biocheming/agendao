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
  /** Translator for display titles; defaults to the canonical English text. */
  t?: (key: string) => string;
}

const RUN_TAIL_TITLE_FALLBACKS: Record<string, string> = {
  "runTail.error": "Run failed",
  "runTail.waitingPermission": "Waiting for permission",
  "runTail.waitingUser": "Waiting for user input",
  "runTail.complete": "Run complete",
  "runTail.runIdle": "Run idle",
  "runTail.idle": "Session idle",
  "runTail.running": "Running",
  "runTail.retrying": "Retrying",
  "runTail.compacting": "Compacting",
  "runTail.cancelling": "Cancelling",
  "runTail.reconnecting": "Reconnecting stream",
  "runTail.ready": "Session ready",
  "runTail.status": "Session status",
};

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
  t,
}: BuildRunTailSummaryOptions): RunTailSummary {
  const title = (key: string) =>
    t ? t(key) : (RUN_TAIL_TITLE_FALLBACKS[key] ?? key);
  const normalizedRuntimeStatus = normalizeStatus(runtimeStatus);
  const normalizedStatusLine = normalizeStatus(statusLine);
  const effectiveStatus =
    normalizedStatusLine && normalizedStatusLine !== "ready"
      ? normalizedStatusLine
      : normalizedRuntimeStatus || "ready";

  if (latestRuntimeError) {
    return {
      status: "error",
      title: title("runTail.error"),
      detail: latestRuntimeError,
      tone: "danger",
    };
  }

  if (pendingPermission) {
    return {
      status: "awaiting_permission",
      title: title("runTail.waitingPermission"),
      detail: "Permission approval is required before execution continues.",
      tone: "warning",
    };
  }

  if (awaitingUser) {
    return {
      status: "awaiting_user",
      title: title("runTail.waitingUser"),
      detail: "Question response is required before execution continues.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "error") {
    return {
      status: "error",
      title: title("runTail.error"),
      detail: "Execution ended with an error.",
      tone: "danger",
    };
  }

  if (effectiveStatus === "complete") {
    return {
      status: "complete",
      title: title("runTail.complete"),
      detail: usageSummary(usage) ?? "Execution completed.",
      tone: "success",
    };
  }

  if (effectiveStatus === "idle") {
    if (usage) {
      return {
        status: "idle",
        title: title("runTail.runIdle"),
        detail: usageSummary(usage),
        tone: "success",
      };
    }
    return {
      status: "idle",
      title: title("runTail.idle"),
      detail: "No active execution.",
      tone: "neutral",
    };
  }

  if (effectiveStatus === "running") {
    return {
      status: "running",
      title: title("runTail.running"),
      detail: activeStageName
        ? `Current stage: ${activeStageName}`
        : "Execution activity is streaming.",
      tone: "info",
    };
  }

  if (effectiveStatus === "retrying") {
    return {
      status: "retrying",
      title: title("runTail.retrying"),
      detail: "Waiting for automatic retry.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "compacting") {
    return {
      status: "compacting",
      title: title("runTail.compacting"),
      detail: "Preparing a smaller context window.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "reconnecting") {
    return {
      status: "reconnecting",
      title: title("runTail.reconnecting"),
      detail: "Waiting for the event stream to resume.",
      tone: "warning",
    };
  }

  if (effectiveStatus === "cancelling") {
    return {
      status: "cancelling",
      title: title("runTail.cancelling"),
      detail: activeStageName,
      tone: "warning",
    };
  }

  return {
    status: effectiveStatus || "ready",
    title:
      effectiveStatus === "ready" || !effectiveStatus
        ? title("runTail.ready")
        : title("runTail.status"),
    detail: effectiveStatus === "ready" || !effectiveStatus ? "No active execution." : null,
    tone: "neutral",
  };
}
