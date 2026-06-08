import type { FeedBlock, FeedMessage } from "./history";
import type {
  ContextCompactionLifecycleSummaryRecord,
  ContextCompactionSummaryRecord,
} from "./sessionActivity";

export const SYNTHETIC_COMPACTION_MARKER = "agendao_web_synthetic_compaction";

function normalizeStatus(value?: string | null) {
  return value?.trim().toLowerCase() || "";
}

function compactNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) {
    const compact = value / 1_000_000;
    return Number.isInteger(compact) ? `${compact.toFixed(0)}M` : `${compact.toFixed(1)}M`;
  }
  if (value >= 1_000) {
    const compact = value / 1_000;
    return Number.isInteger(compact) ? `${compact.toFixed(0)}K` : `${compact.toFixed(1)}K`;
  }
  return String(Math.round(value));
}

function compactOptionalNumber(value?: number | null) {
  return typeof value === "number" && Number.isFinite(value) ? compactNumber(value) : null;
}

function contextUsagePercent(used?: number | null, limit?: number | null) {
  if (
    typeof used !== "number"
    || !Number.isFinite(used)
    || typeof limit !== "number"
    || !Number.isFinite(limit)
    || limit <= 0
  ) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((used / limit) * 100)));
}

function compactionUsedTokens(
  summary?: ContextCompactionSummaryRecord | null,
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null,
) {
  return lifecycle?.request_context_tokens
    ?? lifecycle?.live_context_tokens
    ?? summary?.request_context_tokens
    ?? summary?.live_context_tokens
    ?? null;
}

function compactionBodyChars(
  summary?: ContextCompactionSummaryRecord | null,
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null,
) {
  return lifecycle?.body_chars ?? summary?.body_chars ?? null;
}

function compactionLimitTokens(
  summary?: ContextCompactionSummaryRecord | null,
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null,
) {
  return lifecycle?.limit_tokens ?? summary?.limit_tokens ?? null;
}

function cleanLabel(value?: string | null) {
  const normalized = value?.trim();
  return normalized ? normalized.replaceAll("_", " ") : null;
}

export function compactionStatusLine(
  summary?: ContextCompactionSummaryRecord | null,
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null,
) {
  const used = compactionUsedTokens(summary, lifecycle);
  const bodyChars = compactionBodyChars(summary, lifecycle);
  const messageCount = summary?.message_count_before ?? null;

  const parts: string[] = [];
  if (typeof messageCount === "number" && Number.isFinite(messageCount) && messageCount > 0) {
    parts.push(`compressing ${messageCount} messages`);
  } else if (used != null || bodyChars != null) {
    parts.push("compressing conversation");
  }
  if (used != null) {
    parts.push(`~${compactNumber(used)} tok`);
  }
  if (bodyChars != null) {
    parts.push(`${compactNumber(bodyChars)} chars`);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function compactionDetailLine(
  summary?: ContextCompactionSummaryRecord | null,
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null,
) {
  const reason = cleanLabel(lifecycle?.reason ?? summary?.reason ?? null);
  const phase = cleanLabel(lifecycle?.phase ?? summary?.phase ?? null);
  const used = compactionUsedTokens(summary, lifecycle);
  const limit = compactionLimitTokens(summary, lifecycle);
  const percent = contextUsagePercent(used, limit);

  const parts: string[] = [];
  if (reason) parts.push(reason);
  if (phase) parts.push(phase);
  if (used != null && limit != null) {
    const usedLabel = compactOptionalNumber(used);
    const limitLabel = compactOptionalNumber(limit);
    if (usedLabel && limitLabel) {
      parts.push(percent == null ? `${usedLabel}/${limitLabel}` : `${usedLabel}/${limitLabel} ${percent}%`);
    }
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function isSyntheticCompactionMessage(message: FeedMessage) {
  return message.kind === "status" && message.metadata?.[SYNTHETIC_COMPACTION_MARKER] === true;
}

export function syntheticCompactionLines(message: FeedBlock<"status">) {
  const metadata = message.metadata ?? {};
  const statusLine =
    typeof metadata.agendao_web_compaction_status_line === "string"
      ? metadata.agendao_web_compaction_status_line
      : null;
  const detailLine =
    typeof metadata.agendao_web_compaction_detail_line === "string"
      ? metadata.agendao_web_compaction_detail_line
      : null;
  return { statusLine, detailLine };
}

export function buildSyntheticCompactionFeedMessage({
  sessionId,
  runStatus,
  summary = null,
  lifecycle = null,
}: {
  sessionId: string;
  runStatus?: string | null;
  summary?: ContextCompactionSummaryRecord | null;
  lifecycle?: ContextCompactionLifecycleSummaryRecord | null;
}): FeedBlock<"status"> | null {
  if (normalizeStatus(runStatus) !== "compacting") {
    return null;
  }

  const statusLine = compactionStatusLine(summary, lifecycle);
  const detailLine = compactionDetailLine(summary, lifecycle);
  const fallbackText = "Preparing a smaller context window.";

  return {
    kind: "status",
    role: "system",
    tone: "warning",
    id: `__compaction__:${sessionId}`,
    feedId: `__compaction__:${sessionId}`,
    title: "Compacting conversation",
    summary: statusLine ?? fallbackText,
    text: detailLine ?? statusLine ?? fallbackText,
    metadata: {
      [SYNTHETIC_COMPACTION_MARKER]: true,
      agendao_web_compaction_status_line: statusLine,
      agendao_web_compaction_detail_line: detailLine,
    },
  };
}

export function withSyntheticCompactionMessage(
  messages: FeedMessage[],
  options: {
    sessionId: string | null;
    runStatus?: string | null;
    summary?: ContextCompactionSummaryRecord | null;
    lifecycle?: ContextCompactionLifecycleSummaryRecord | null;
  },
) {
  if (!options.sessionId) return messages;
  const synthetic = buildSyntheticCompactionFeedMessage({
    sessionId: options.sessionId,
    runStatus: options.runStatus,
    summary: options.summary,
    lifecycle: options.lifecycle,
  });
  if (!synthetic) return messages;
  return [...messages.filter((message) => !isSyntheticCompactionMessage(message)), synthetic];
}
