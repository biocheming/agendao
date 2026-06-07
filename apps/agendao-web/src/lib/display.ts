import type { MessageRecord, FeedMessage, OutputBlock, RuntimeSurfaceOutputBlock } from "./history";
import type { SessionRecord } from "./session";
import type { WorkspaceContextRecord, RecentModelRecord } from "./workspace";
import type { ExecutionMode } from "./webRuntime";
import type { QuestionAnswerValue } from "./interaction";
import type { PendingCommandInvocationRecord } from "./session";
import { resolveWorkspacePath } from "./composerContext";

// ============================================================
// Runtime budget
// ============================================================

export function readRuntimeBudgetNumber(
  config: Record<string, unknown> | null | undefined,
  snakeKey: string,
  fallback: number,
): number {
  const runtimeBudget = config?.runtimeBudget;
  if (!runtimeBudget || typeof runtimeBudget !== "object" || Array.isArray(runtimeBudget)) {
    return fallback;
  }
  const record = runtimeBudget as Record<string, unknown>;
  const camelKey = snakeKey.replace(/_([a-z])/g, (_, chr: string) => chr.toUpperCase());
  const value = record[snakeKey] ?? record[camelKey];
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

// ============================================================
// Error formatting
// ============================================================

export function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}

// ============================================================
// Model selection
// ============================================================

export function resolveActiveModelRef(
  session: SessionRecord | null,
  selectedModel: string,
): string | null {
  const explicit = selectedModel.trim();
  if (explicit) return explicit;
  const hinted = session?.hints?.current_model?.trim();
  if (hinted) return hinted;
  const provider = session?.hints?.model_provider?.trim();
  const model = session?.hints?.model_id?.trim();
  if (provider && model) return `${provider}/${model}`;
  return model || null;
}

export function workspaceRecentModelScope(
  context: WorkspaceContextRecord | null,
): string | null {
  if (!context) return null;
  return `${context.mode}:${context.identity.workspace_key}`;
}

export function splitRecentModelRef(modelRef: string): RecentModelRecord | null {
  const trimmed = modelRef.trim();
  const separator = trimmed.indexOf("/");
  if (separator <= 0 || separator >= trimmed.length - 1) return null;
  const provider = trimmed.slice(0, separator).trim();
  const model = trimmed.slice(separator + 1).trim();
  if (!provider || !model) return null;
  return { provider, model };
}

export const RECENT_MODEL_LIMIT = 5;

export function pushRecentModel(
  recentModels: RecentModelRecord[],
  modelRef: string,
): RecentModelRecord[] {
  const next = splitRecentModelRef(modelRef);
  if (!next) return recentModels;
  return [
    next,
    ...recentModels.filter(
      (entry) =>
        !(
          entry.provider.toLowerCase() === next.provider.toLowerCase() &&
          entry.model.toLowerCase() === next.model.toLowerCase()
        ),
    ),
  ].slice(0, RECENT_MODEL_LIMIT);
}

// ============================================================
// Execution mode
// ============================================================

export function modeKey(mode: ExecutionMode): string {
  return `${mode.kind}:${mode.id}`;
}

// ============================================================
// Composer / prompt display
// ============================================================

export type PromptPart =
  | { type: "text"; text: string }
  | { type: "file"; url: string; filename?: string; mime?: string }
  | { type: "agent"; name: string }
  | { type: "subtask"; prompt: string; description?: string; agent: string };

export function promptPreviewText(content: string, parts: PromptPart[]): string {
  const trimmed = content.trim();
  if (trimmed) return trimmed;
  const attachmentCount = parts.filter((part) => part.type !== "text").length;
  if (attachmentCount === 0) return "";
  return attachmentCount === 1 ? "[1 attachment]" : `[${attachmentCount} attachments]`;
}

// ============================================================
// Shell / command helpers
// ============================================================

export function shellQuoteCommandValue(value: string): string {
  if (!value) return '""';
  if (/^[A-Za-z0-9/_.*:-]+$/.test(value)) return value;
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

export function splitRepeatableAnswer(answer: string): string[] {
  return answer
    .split(/[\n,\t]/)
    .flatMap((segment) => segment.split(/\s+/))
    .map((value) => value.trim())
    .filter(Boolean);
}

export type PendingCommandInvocation = PendingCommandInvocationRecord;

export function pendingCommandFromSession(
  session: SessionRecord,
  questionId: string,
): PendingCommandInvocation | null {
  const pending = session.pending_command_invocation ?? session.metadata?.pending_command_invocation;
  if (!pending || typeof pending !== "object") return null;
  const invocation = pending as PendingCommandInvocation;
  if (invocation.questionId && invocation.questionId !== questionId) {
    return null;
  }
  return invocation;
}

export function normalizedAnswerValues(
  answer: QuestionAnswerValue | undefined,
  multiple: boolean,
): string[] {
  if (Array.isArray(answer)) {
    return answer.map((value) => value.trim()).filter(Boolean);
  }
  const text = typeof answer === "string" ? answer.trim() : "";
  if (!text) return [];
  if (multiple || /[\n,\t]/.test(text)) {
    return splitRepeatableAnswer(text);
  }
  return [text];
}

export function mergePendingCommandArguments(
  pending: PendingCommandInvocation,
  answers: string[][],
): string {
  const parts: string[] = [];
  const raw = pending.rawArguments?.trim() ?? "";
  if (raw) parts.push(raw);
  for (const [index, field] of (pending.missingFields ?? []).entries()) {
    const values = (answers[index] ?? [])
      .flatMap((value) =>
        /[\n,\t]/.test(value) ? splitRepeatableAnswer(value) : [value],
      )
      .map((value) => value.trim())
      .filter(Boolean);
    if (!values.length) continue;
    parts.push(`--${field}`);
    parts.push(...values.map((value) => shellQuoteCommandValue(value)));
  }
  return parts.join(" ").trim();
}

// ============================================================
// Message feed helpers
// ============================================================

export function findLastMessage(
  messages: FeedMessage[],
  predicate: (message: FeedMessage) => boolean,
): FeedMessage | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (predicate(messages[index])) return messages[index];
  }
  return null;
}

export function metadataValue(
  metadata: Record<string, unknown> | null | undefined,
  dottedKey: string,
): unknown {
  if (!metadata) return undefined;
  if (dottedKey in metadata) return metadata[dottedKey];

  const segments = dottedKey.split(".");
  let current: unknown = metadata;
  for (const segment of segments) {
    if (!current || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

export function previewPathFromMessageMetadata(
  history: MessageRecord[],
  workspaceBasePath: string,
): string | null {
  for (let index = history.length - 1; index >= 0; index -= 1) {
    const message = history[index];
    if (message.role === "user") continue;

    const previewTarget =
      metadataValue(message.metadata, "ui.auto_preview") ??
      metadataValue(message.metadata, "embed.file_path") ??
      metadataValue(message.metadata, "file_path");
    if (typeof previewTarget !== "string" || !previewTarget.trim()) {
      continue;
    }

    return resolveWorkspacePath(workspaceBasePath, previewTarget.trim());
  }
  return null;
}

// ============================================================
// Runtime surface
// ============================================================

export function runtimeSurfacePreview(block: RuntimeSurfaceOutputBlock): string | null {
  const candidate = [
    block.display?.summary,
    block.summary,
    block.text,
    block.detail,
    block.preview,
    block.body,
  ].find((value) => typeof value === "string" && value.trim().length > 0);
  return typeof candidate === "string" ? candidate.trim() : null;
}

export function runtimeSurfaceLabel(block: RuntimeSurfaceOutputBlock): string {
  const candidate = [
    block.title,
    block.event,
    block.display?.header,
    block.kind,
  ].find((value) => typeof value === "string" && value.trim().length > 0);
  return typeof candidate === "string" ? candidate.trim() : block.kind;
}

export function runtimeSurfacePhase(block: RuntimeSurfaceOutputBlock): string | null {
  return typeof block.phase === "string" && block.phase.trim() ? block.phase.trim() : null;
}

export function runtimeSurfaceDebugDetail(block: OutputBlock): string | undefined {
  if (!("detail" in block)) return undefined;
  return typeof block.detail === "string" ? block.detail : undefined;
}

// ============================================================
// Ingress stabilization
// ============================================================

export function ingressStabilizationLabel(
  value: Record<string, unknown> | null | undefined,
): string | null {
  if (!value) return null;
  const sourceValue = value.source;
  const source =
    typeof sourceValue === "string"
      ? sourceValue
      : sourceValue &&
          typeof sourceValue === "object" &&
          "source" in sourceValue &&
          typeof sourceValue.source === "string"
        ? sourceValue.source
        : "unknown";
  const policy = typeof value.policy === "string" ? value.policy : "metadata_only";
  const batchCount = typeof value.batch_count === "number" ? value.batch_count : 1;
  return batchCount > 1
    ? `${source} · ${policy} · batch ${batchCount}`
    : `${source} · ${policy}`;
}
