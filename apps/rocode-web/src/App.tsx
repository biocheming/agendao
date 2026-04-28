import {
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ComposerSection } from "./components/ComposerSection";
import { ConversationFeedPanel } from "./components/ConversationFeedPanel";
import { DeferredTerminalPanel } from "./components/DeferredTerminalPanel";
import { InteractionOverlays } from "./components/InteractionOverlays";
import { SessionSidebar } from "./components/SessionSidebar";
import { WorkspacePanel, type WorkspacePanelTab } from "./components/WorkspacePanel";
import { loadWebPlugins } from "./web-plugin-loader";
import { api, apiJson, apiUrl, parseSSE } from "./lib/api";
import { cn } from "./lib/utils";
import { useConversationJump } from "./hooks/useConversationJump";
import { useExecutionActivity } from "./hooks/useExecutionActivity";
import { useMultimodalComposer } from "./hooks/useMultimodalComposer";
import { useSchedulerNavigation } from "./hooks/useSchedulerNavigation";
import { useTerminalSessions } from "./hooks/useTerminalSessions";
import { useResizableHeight, useResizableWidth } from "./hooks/useResizableWidth";
import { prepareComposerAttachments } from "./lib/composerAttachments";
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "./lib/contextPressure";
import {
  buildWebSessionUrl,
  readWebSessionRoute,
  writeWebSessionRoute,
} from "./lib/webSessionUrl";
import {
  attachmentContainsWorkspacePath,
  attachmentLabel,
  attachmentWorkspacePath,
  appendReferenceToken,
  droppedFiles,
  extractPromptReferences,
  fileUrlFromPath,
  findFirstFile,
  findNodeByPath,
  guessWorkspaceMime,
  parentDirectory,
  removePromptReference,
  resolveWorkspacePath,
  toWorkspaceReferencePath,
} from "./lib/composerContext";
import {
  buildMultimodalHistoryBlocks,
} from "./lib/multimodal";
import type {
  FeedMessage,
  MessagePartRecord,
  MessageRecord,
  OutputBlock,
  OutputField,
} from "./lib/history";
import {
  type PermissionInteractionRecord,
  type PromptResponseRecord,
  type QuestionAnswerValue,
  type QuestionInfoResponseRecord,
  type QuestionInteractionRecord,
  permissionInteractionFromEvent,
  questionInteractionFromEvent,
  questionInteractionFromInfo,
} from "./lib/interaction";
import type {
  PendingCommandInvocationRecord,
  SessionListResponseRecord,
  SessionRecord,
} from "./lib/session";
import {
  type ConfigProvidersResponseRecord,
  type ConnectProtocolOption,
  type KnownProviderEntry,
  type ProviderRecord,
  type ProviderConnectSchemaResponseRecord,
  type ResolveProviderConnectResponseRecord,
  flattenProviderModels,
} from "./lib/provider";
import {
  basenamePath,
  buildSessionTree,
  buildWorkspaceSummaries,
  normalizeSessionRecord,
  normalizeSessionRecords,
} from "./lib/sidebar";
import type { SessionTreeNode, WorkspaceSummary } from "./lib/sidebar";
import {
  type DirectoryCreateResponseRecord,
  type FileContentResponseRecord,
  type FileTreeNodeRecord,
  type PathsResponseRecord,
  type UploadFilesResponseRecord,
  type WorkspaceContextRecord,
  workspaceModeFromContext,
  workspaceRootFromContext,
} from "./lib/workspace";
import {
  AlertTriangleIcon,
  FolderTreeIcon,
  PanelLeftIcon,
  SettingsIcon,
  TerminalSquareIcon,
  XIcon,
} from "lucide-react";

type ThemeId = "daylight" | "sunset" | "cobalt";

interface ExecutionMode {
  id: string;
  name: string;
  kind: string;
  hidden?: boolean;
  mode?: string;
}

type PromptPart =
  | {
      type: "text";
      text: string;
    }
  | {
      type: "file";
      url: string;
      filename?: string;
      mime?: string;
    }
  | {
      type: "agent";
      name: string;
    }
  | {
      type: "subtask";
      prompt: string;
      description?: string;
      agent: string;
    };

type SessionLiveBlockCache = Record<string, OutputBlock[]>;
type SessionOptimisticFeedCache = Record<string, FeedMessage[]>;

type PendingCommandInvocation = PendingCommandInvocationRecord;

const THEMES: Array<{ id: ThemeId; label: string }> = [
  { id: "daylight", label: "Daylight" },
  { id: "sunset", label: "Sunset" },
  { id: "cobalt", label: "Cobalt" },
];

function resolveActiveModelRef(session: SessionRecord | null, selectedModel: string) {
  const explicit = selectedModel.trim();
  if (explicit) return explicit;
  const hinted = session?.hints?.current_model?.trim();
  if (hinted) return hinted;
  const provider = session?.hints?.model_provider?.trim();
  const model = session?.hints?.model_id?.trim();
  if (provider && model) return `${provider}/${model}`;
  return model || null;
}

const SettingsDrawer = lazy(async () => {
  const module = await import("./components/SettingsDrawer");
  return { default: module.SettingsDrawer };
});

let feedSequence = 0;

function nextFeedId() {
  feedSequence += 1;
  return `feed-${feedSequence}`;
}

function shellQuoteCommandValue(value: string): string {
  if (!value) return '""';
  if (/^[A-Za-z0-9/_.*:-]+$/.test(value)) return value;
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function splitRepeatableAnswer(answer: string): string[] {
  return answer
    .split(/[\n,\t]/)
    .flatMap((segment) => segment.split(/\s+/))
    .map((value) => value.trim())
    .filter(Boolean);
}

function pendingCommandFromSession(
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

function normalizedAnswerValues(
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

function mergePendingCommandArguments(
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

function promptPreviewText(content: string, parts: PromptPart[]): string {
  const trimmed = content.trim();
  if (trimmed) return trimmed;
  const attachmentCount = parts.filter((part) => part.type !== "text").length;
  if (attachmentCount === 0) return "";
  return attachmentCount === 1 ? "[1 attachment]" : `[${attachmentCount} attachments]`;
}

function normalizeBlockText(block: OutputBlock): string {
  if (block.text?.trim()) return block.text;
  if (block.display?.summary?.trim()) return block.display.summary;
  if (block.detail?.trim()) return block.detail;
  if (block.summary?.trim()) return block.summary;
  if (block.preview?.trim()) return block.preview;
  if (block.body?.trim()) return block.body;
  if (block.display?.preview?.text?.trim()) return block.display.preview.text;
  if (block.display?.fields?.length) {
    return block.display.fields
      .map((field) => `${field.label ?? "Field"}: ${String(field.value ?? "")}`)
      .join("\n");
  }
  if (block.fields?.length) {
    return block.fields
      .map((field) => `${field.label ?? "Field"}: ${String(field.value ?? "")}`)
      .join("\n");
  }
  return "";
}

function toFeedMessage(block: OutputBlock): FeedMessage {
  return {
    ...block,
    feedId: nextFeedId(),
    anchorId: block.id,
    text: normalizeBlockText(block),
  };
}

function presentationRank(block: OutputBlock): number {
  return typeof block.presentation?.rank === "number"
    ? block.presentation.rank
    : outputBlockSemanticRank(block);
}

function presentationSequence(block: OutputBlock): number {
  return typeof block.presentation?.sequence === "number" ? block.presentation.sequence : 0;
}

function outputBlockSemanticRank(block: OutputBlock): number {
  switch (block.kind) {
    case "queue_item":
      return 0;
    case "status":
      return 5;
    case "reasoning":
      return 10;
    case "tool":
      return 20;
    case "session_event":
      return 25;
    case "scheduler_stage":
      return 30;
    case "inspect":
      return 40;
    case "message":
      return block.role === "assistant" ? 90 : 0;
    default:
      return 50;
  }
}

function messagePartSemanticRank(part: MessagePartRecord): number {
  if (part.output_block) {
    return outputBlockSemanticRank(part.output_block);
  }
  switch (part.type) {
    case "reasoning":
      return 1;
    case "tool_call":
    case "tool_result":
      return 2;
    case "text":
      return 4;
    default:
      return 3;
  }
}

function metadataString(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): string | undefined {
  const value = metadata?.[key];
  return typeof value === "string" && value.trim() ? value : undefined;
}

function metadataNumber(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): number | undefined {
  const value = metadata?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function metadataBoolean(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): boolean | undefined {
  const value = metadata?.[key];
  return typeof value === "boolean" ? value : undefined;
}

function metadataStringArray(
  metadata: Record<string, unknown> | null | undefined,
  key: string,
): string[] {
  const value = metadata?.[key];
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
}

function schedulerStageTitleFromText(text: string): { title: string; body: string } {
  const trimmed = text.trim();
  const heading = trimmed.match(/^##\s+([^\n]+)(?:\n([\s\S]*))?$/);
  if (!heading) return { title: "", body: text };
  return {
    title: heading[1]?.trim() ?? "",
    body: heading[2]?.trimStart() ?? "",
  };
}

function prettifySchedulerToken(value: string): string {
  return value
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function schedulerDecisionFromMetadata(
  metadata: Record<string, unknown> | null | undefined,
): OutputBlock["decision"] | null {
  const kind = metadataString(metadata, "scheduler_decision_kind");
  if (!kind) return null;

  const rawFields = metadata?.scheduler_decision_fields;
  const rawSections = metadata?.scheduler_decision_sections;
  const fields = Array.isArray(rawFields)
    ? rawFields.flatMap((field) => {
        if (!field || typeof field !== "object") return [];
        const item = field as Record<string, unknown>;
        const label = typeof item.label === "string" ? item.label : undefined;
        const value = typeof item.value === "string" ? item.value : undefined;
        if (!label || value === undefined) return [];
        return [{
          label,
          value,
          tone: typeof item.tone === "string" ? item.tone : undefined,
        }];
      })
    : [];
  const sections = Array.isArray(rawSections)
    ? rawSections.flatMap((section) => {
        if (!section || typeof section !== "object") return [];
        const item = section as Record<string, unknown>;
        const title = typeof item.title === "string" ? item.title : undefined;
        const body = typeof item.body === "string" ? item.body : undefined;
        if (!title || body === undefined) return [];
        return [{ title, body }];
      })
    : [];

  return {
    title: metadataString(metadata, "scheduler_decision_title") ?? "Decision",
    fields,
    sections,
  };
}

function schedulerStageBlockFromHistoryMessage(message: MessageRecord): OutputBlock | null {
  const metadata = message.metadata;
  const stage = metadataString(metadata, "scheduler_stage");
  if (!stage) return null;

  const text = (message.parts ?? [])
    .filter((part) => part.type === "text" && !part.ignored)
    .map((part) => part.text ?? "")
    .join("");
  const { title, body } = schedulerStageTitleFromText(text);
  const profile =
    metadataString(metadata, "resolved_scheduler_profile") ??
    metadataString(metadata, "scheduler_profile");
  const fallbackTitle = profile
    ? `${profile} · ${prettifySchedulerToken(stage)}`
    : prettifySchedulerToken(stage);

  return {
    id: message.id,
    kind: "scheduler_stage",
    role: "assistant",
    stage_id: metadataString(metadata, "scheduler_stage_id"),
    profile,
    stage,
    title: title || fallbackTitle,
    text: body,
    stage_index: metadataNumber(metadata, "scheduler_stage_index"),
    stage_total: metadataNumber(metadata, "scheduler_stage_total"),
    step: metadataNumber(metadata, "scheduler_stage_step"),
    status: metadataString(metadata, "scheduler_stage_status"),
    focus: metadataString(metadata, "scheduler_stage_focus"),
    last_event: metadataString(metadata, "scheduler_stage_last_event"),
    waiting_on: metadataString(metadata, "scheduler_stage_waiting_on"),
    activity: metadataString(metadata, "scheduler_stage_activity"),
    child_session_id: metadataString(metadata, "scheduler_stage_child_session_id"),
    active_skills: metadataStringArray(metadata, "scheduler_stage_active_skills"),
    active_agents: metadataStringArray(metadata, "scheduler_stage_active_agents"),
    active_categories: metadataStringArray(metadata, "scheduler_stage_active_categories"),
    prompt_tokens: metadataNumber(metadata, "scheduler_stage_prompt_tokens"),
    completion_tokens: metadataNumber(metadata, "scheduler_stage_completion_tokens"),
    reasoning_tokens: metadataNumber(metadata, "scheduler_stage_reasoning_tokens"),
    cache_read_tokens: metadataNumber(metadata, "scheduler_stage_cache_read_tokens"),
    cache_write_tokens: metadataNumber(metadata, "scheduler_stage_cache_write_tokens"),
    decision: schedulerDecisionFromMetadata(metadata),
    presentation: {
      group: "scheduler",
      slot: metadataString(metadata, "scheduler_stage_id") ?? stage,
      rank: 30,
      sequence: metadataNumber(metadata, "scheduler_stage_index"),
    },
    structured: {
      loop_budget: metadataString(metadata, "scheduler_stage_loop_budget"),
      available_skill_count: metadataNumber(metadata, "scheduler_stage_available_skill_count"),
      available_agent_count: metadataNumber(metadata, "scheduler_stage_available_agent_count"),
      available_category_count: metadataNumber(metadata, "scheduler_stage_available_category_count"),
      done_agent_count: metadataNumber(metadata, "scheduler_stage_done_agent_count"),
      total_agent_count: metadataNumber(metadata, "scheduler_stage_total_agent_count"),
      estimated_context_tokens: metadataNumber(metadata, "scheduler_stage_estimated_context_tokens"),
      skill_tree_budget: metadataNumber(metadata, "scheduler_stage_skill_tree_budget"),
      skill_tree_truncated: metadataBoolean(metadata, "scheduler_stage_skill_tree_truncated"),
      skill_tree_truncation_strategy: metadataString(metadata, "scheduler_stage_skill_tree_truncation_strategy"),
      retry_attempt: metadataNumber(metadata, "scheduler_stage_retry_attempt"),
    },
  };
}

function lastTurnStartIndex(messages: FeedMessage[]): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.kind === "message" && message.role === "user") {
      return index;
    }
  }
  return 0;
}

function insertFeedMessageByPresentation(
  messages: FeedMessage[],
  incoming: FeedMessage,
): FeedMessage[] {
  if (incoming.kind === "message" && incoming.role === "user") {
    return [...messages, incoming];
  }

  const rank = presentationRank(incoming);
  const sequence = presentationSequence(incoming);
  const start = lastTurnStartIndex(messages);
  let insertIndex = messages.length;

  for (let index = messages.length - 1; index >= start; index -= 1) {
    const candidate = messages[index];
    const candidateRank = presentationRank(candidate);
    const candidateSequence = presentationSequence(candidate);
    if (
      candidateRank > rank
      || (candidateRank === rank && candidateSequence > sequence)
    ) {
      insertIndex = index;
      continue;
    }
    break;
  }

  if (insertIndex >= messages.length) {
    return [...messages, incoming];
  }

  const next = [...messages];
  next.splice(insertIndex, 0, incoming);
  return next;
}

function orderedMessageParts(parts: MessagePartRecord[] = []): MessagePartRecord[] {
  return parts
    .map((part, index) => ({ part, index }))
    .sort((left, right) => {
      const rankDelta = messagePartSemanticRank(left.part) - messagePartSemanticRank(right.part);
      return rankDelta || left.index - right.index;
    })
    .map(({ part }) => part);
}

function orderRelatedFeedMessages(messages: FeedMessage[]): FeedMessage[] {
  return messages
    .map((message, index) => ({ message, index }))
    .sort((left, right) => {
      if (!left.message.id || left.message.id !== right.message.id) {
        return left.index - right.index;
      }
      const rankDelta = presentationRank(left.message) - presentationRank(right.message);
      return rankDelta || left.index - right.index;
    })
    .map(({ message }) => message);
}

function createOptimisticUserFeedMessage(text: string): FeedMessage {
  const feedId = nextFeedId();
  return {
    kind: "message",
    phase: "full",
    role: "user",
    text,
    feedId,
    anchorId: feedId,
  };
}

function isEquivalentUserMessage(message: FeedMessage, optimistic: FeedMessage): boolean {
  return (
    message.kind === "message" &&
    message.role === "user" &&
    message.text.trim() === optimistic.text.trim()
  );
}

function mergeOptimisticMessages(
  messages: FeedMessage[],
  optimistic: FeedMessage[],
): { messages: FeedMessage[]; remaining: FeedMessage[] } {
  if (optimistic.length === 0) {
    return { messages, remaining: optimistic };
  }

  const remaining = [...optimistic];
  for (const message of messages) {
    const matchIndex = remaining.findIndex((candidate) =>
      isEquivalentUserMessage(message, candidate),
    );
    if (matchIndex >= 0) {
      remaining.splice(matchIndex, 1);
    }
  }

  return {
    messages: remaining.length > 0 ? [...messages, ...remaining] : messages,
    remaining,
  };
}

function upsertFeedMessage(
  messages: FeedMessage[],
  block: OutputBlock,
  overrides: Partial<FeedMessage> = {},
): FeedMessage[] {
  if (!block.id) {
    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage(block),
      ...overrides,
    });
  }

  const index = messages.findIndex(
    (message) => message.kind === block.kind && message.id === block.id,
  );
  if (index < 0) {
    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage(block),
      ...overrides,
    });
  }

  const next = [...messages];
  next[index] = {
    ...next[index],
    ...block,
    ...overrides,
    feedId: next[index].feedId,
    anchorId: next[index].anchorId ?? block.id,
  };
  return next;
}

function updateLastMatchingMessage(
  messages: FeedMessage[],
  predicate: (message: FeedMessage) => boolean,
  incomingText: string,
): FeedMessage[] {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const candidate = messages[index];
    if (!predicate(candidate)) continue;
    const next = [...messages];
    next[index] = { ...candidate, text: `${candidate.text}${incomingText}` };
    return next;
  }
  return messages;
}

function appendStreamingDelta(
  messages: FeedMessage[],
  block: OutputBlock,
  predicate: (message: FeedMessage) => boolean,
): FeedMessage[] {
  const incomingText = block.text ?? "";
  if (block.id) {
    const index = messages.findIndex(
      (message) => message.kind === block.kind && message.id === block.id,
    );
    if (index >= 0) {
      const next = [...messages];
      const candidate = next[index];
      next[index] = {
        ...candidate,
        ...block,
        text: `${candidate.text}${incomingText}`,
        feedId: candidate.feedId,
        anchorId: candidate.anchorId ?? block.id,
      };
      return next;
    }

    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage({ ...block, text: incomingText }),
      text: incomingText,
    });
  }

  return updateLastMatchingMessage(messages, predicate, incomingText);
}

function applyOutputBlock(
  messages: FeedMessage[],
  block: OutputBlock,
  showThinking: boolean,
): FeedMessage[] {
  if (block.kind === "reasoning" && !showThinking) {
    return messages;
  }
  if (block.kind === "status" && block.silent) {
    return messages;
  }

  if (block.kind === "message") {
    if (block.phase === "start") {
      return upsertFeedMessage(messages, block, { text: "" });
    }
    if (block.phase === "delta") {
      return appendStreamingDelta(
        messages,
        block,
        (message) => message.kind === "message" && message.role === block.role,
      );
    }
    if (block.phase === "end") {
      return messages;
    }
    return insertFeedMessageByPresentation(messages, toFeedMessage(block));
  }

  if (block.kind === "reasoning") {
    if (block.phase === "start") {
      return upsertFeedMessage(messages, block, { text: "" });
    }
    if (block.phase === "delta") {
      return appendStreamingDelta(
        messages,
        block,
        (message) => message.kind === "reasoning",
      );
    }
    if (block.phase === "end") {
      return messages;
    }
    return insertFeedMessageByPresentation(messages, toFeedMessage(block));
  }

  if (block.id) {
    return upsertFeedMessage(messages, block, {
      text: normalizeBlockText(block),
    });
  }

  return insertFeedMessageByPresentation(messages, toFeedMessage(block));
}

function buildFeedFromHistory(history: MessageRecord[], showThinking: boolean): FeedMessage[] {
  feedSequence = 0;
  let messages: FeedMessage[] = [];

  for (const message of history || []) {
    const schedulerStageBlock = schedulerStageBlockFromHistoryMessage(message);
    if (schedulerStageBlock) {
      messages = applyOutputBlock(messages, schedulerStageBlock, showThinking);
      continue;
    }

    let startedReasoning = false;
    let startedText = false;

    for (const part of orderedMessageParts(message.parts)) {
      if (part.ignored) {
        continue;
      }
      if (part.output_block) {
        messages = applyOutputBlock(messages, part.output_block, showThinking);
        continue;
      }

      if (part.type === "reasoning" && part.text) {
        const blockId = `${message.id}:reasoning`;
        if (!startedReasoning) {
          messages = applyOutputBlock(
            messages,
            {
              id: blockId,
              kind: "reasoning",
              phase: "start",
              role: message.role,
              text: "",
            },
            showThinking,
          );
          startedReasoning = true;
        }
        messages = applyOutputBlock(
          messages,
          {
            id: blockId,
            kind: "reasoning",
            phase: "delta",
            role: message.role,
            text: part.text,
          },
          showThinking,
        );
        continue;
      }

      if (part.type === "text" && part.text) {
        const blockId = `${message.id}:message`;
        if (!startedText) {
          messages = applyOutputBlock(
            messages,
            {
              id: blockId,
              kind: "message",
              phase: "start",
              role: message.role,
              text: "",
            },
            showThinking,
          );
          startedText = true;
        }
        messages = applyOutputBlock(
          messages,
          {
            id: blockId,
            kind: "message",
            phase: "delta",
            role: message.role,
            text: part.text,
          },
          showThinking,
        );
      }
    }

    for (const block of buildMultimodalHistoryBlocks(message)) {
      if (block.kind === "message" && startedText) {
        continue;
      }
      messages = applyOutputBlock(messages, block, showThinking);
      if (block.kind === "message") {
        startedText = true;
      }
    }

    if (startedReasoning) {
      messages = applyOutputBlock(
        messages,
        {
          id: `${message.id}:reasoning`,
          kind: "reasoning",
          phase: "end",
          role: message.role,
          text: "",
        },
        showThinking,
      );
    }

    if (startedText) {
      messages = applyOutputBlock(
        messages,
        {
          id: `${message.id}:message`,
          kind: "message",
          phase: "end",
          role: message.role,
          text: "",
        },
        showThinking,
      );
    }
  }

  return messages;
}

function estimateContextTokensFromHistory(history: MessageRecord[]): number | null {
  const tailStart = Math.max(
    0,
    history.findLastIndex((message) =>
      (message.parts ?? []).some(
        (part) => part.type === "compaction" || (part.type === "text" && part.text?.startsWith("Compacted ")),
      ),
    ),
  );
  const tail = history.slice(tailStart);

  for (let index = tail.length - 1; index >= 0; index -= 1) {
    const message = tail[index];
    if (message?.role !== "assistant") continue;
    const contextTokens = message.tokens?.context;
    if (typeof contextTokens === "number" && Number.isFinite(contextTokens) && contextTokens > 0) {
      return contextTokens;
    }
  }

  let totalChars = 0;
  for (const message of tail) {
    for (const part of message.parts ?? []) {
      if (part.type === "text" || part.type === "reasoning") {
        totalChars += part.text?.length ?? 0;
      } else if (part.type === "file") {
        totalChars += (part.file?.url?.length ?? 0) + (part.file?.filename?.length ?? 0) + (part.file?.mime?.length ?? 0);
      } else if (part.output_block) {
        totalChars += normalizeBlockText(part.output_block).length;
      }
    }
  }

  return totalChars > 0 ? Math.max(1, Math.floor(totalChars / 4)) : null;
}

function isStreamingTextBlock(block: OutputBlock): boolean {
  return block.kind === "message" || block.kind === "reasoning";
}

function shouldRetainLiveBlock(block: OutputBlock): boolean {
  return Boolean(block.id);
}

function liveTextSnapshot(block: OutputBlock, previous?: OutputBlock): OutputBlock {
  if (block.phase === "start") {
    return { ...previous, ...block, text: "" };
  }
  if (block.phase === "delta") {
    return {
      ...previous,
      ...block,
      text: `${previous?.text ?? ""}${block.text ?? ""}`,
    };
  }
  return {
    ...previous,
    ...block,
    text: normalizeBlockText(block),
  };
}

function appendLiveBlock(blocks: OutputBlock[], block: OutputBlock): OutputBlock[] {
  if (!shouldRetainLiveBlock(block)) {
    return blocks;
  }

  const next = blocks.slice();
  const existingIndex = next.findIndex(
    (candidate) => candidate.kind === block.kind && candidate.id === block.id,
  );
  if (block.phase === "end") {
    if (existingIndex >= 0) {
      next.splice(existingIndex, 1);
    }
    return next;
  }

  const previous = existingIndex >= 0 ? next[existingIndex] : undefined;
  const retained = isStreamingTextBlock(block) ? liveTextSnapshot(block, previous) : block;
  if (existingIndex >= 0) {
    next[existingIndex] = retained;
    return next;
  }
  next.push(retained);
  return next;
}

function mergeLiveTextBlock(messages: FeedMessage[], block: OutputBlock, showThinking: boolean): FeedMessage[] {
  if (block.kind === "reasoning" && !showThinking) {
    return messages;
  }

  const blockText = block.text ?? "";
  const matchIndex = block.id
    ? messages.findIndex((message) => message.kind === block.kind && message.id === block.id)
    : -1;
  const fallbackIndex =
    matchIndex >= 0
      ? matchIndex
      : (() => {
          for (let index = messages.length - 1; index >= 0; index -= 1) {
            const candidate = messages[index];
            if (candidate.kind !== block.kind) continue;
            if (block.kind === "message" && candidate.role !== block.role) continue;
            return index;
          }
          return -1;
        })();

  if (fallbackIndex >= 0) {
    const next = [...messages];
    const candidate = next[fallbackIndex];
    next[fallbackIndex] = {
      ...candidate,
      ...block,
      text: blockText,
      feedId: candidate.feedId,
      anchorId: candidate.anchorId ?? block.id,
    };
    return next;
  }

  return insertFeedMessageByPresentation(messages, {
    ...toFeedMessage(block),
    text: blockText,
  });
}

function mergeHistoryWithLiveBlocks(
  history: MessageRecord[],
  liveBlocks: OutputBlock[],
  showThinking: boolean,
): FeedMessage[] {
  return orderRelatedFeedMessages(liveBlocks.reduce((current, block) => {
    if (isStreamingTextBlock(block)) {
      return mergeLiveTextBlock(current, block, showThinking);
    }
    return applyOutputBlock(current, block, showThinking);
  }, buildFeedFromHistory(history, showThinking)));
}

function modeKey(mode: ExecutionMode): string {
  return `${mode.kind}:${mode.id}`;
}

function applyPreferences(config: Record<string, unknown>) {
  const ui = (config.uiPreferences ?? config.ui_preferences ?? {}) as Record<string, unknown>;
  return {
    theme: String(ui.webTheme ?? ui.web_theme ?? "daylight") as ThemeId,
    mode: String(ui.webMode ?? ui.web_mode ?? ""),
    model: String(ui.webModel ?? ui.web_model ?? ""),
    showThinking: Boolean(ui.showThinking ?? ui.show_thinking ?? false),
  };
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}

function findLastMessage(messages: FeedMessage[], predicate: (message: FeedMessage) => boolean) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (predicate(messages[index])) return messages[index];
  }
  return null;
}

function metadataValue(
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

function previewPathFromMessageMetadata(
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

export default function App() {
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<FeedMessage[]>([]);
  const [messageHistory, setMessageHistory] = useState<MessageRecord[]>([]);
  const [selectedMessageIds, setSelectedMessageIds] = useState<Set<string>>(() => new Set());
  const [composer, setComposer] = useState("");
  const [attachments, setAttachments] = useState<PromptPart[]>([]);
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [knownProviders, setKnownProviders] = useState<KnownProviderEntry[]>([]);
  const [connectProtocols, setConnectProtocols] = useState<ConnectProtocolOption[]>([]);
  const [modes, setModes] = useState<ExecutionMode[]>([]);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContextRecord | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedMode, setSelectedMode] = useState("");
  const [connectQuery, setConnectQuery] = useState("");
  const [connectProviderId, setConnectProviderId] = useState("");
  const [leftSidebarOpen, setLeftSidebarOpen] = useState(true);
  const [rightSidebarOpen, setRightSidebarOpen] = useState(true);
  const leftResize = useResizableWidth(312, 220, 520, "left");
  const rightResize = useResizableWidth(420, 320, 880, "right");
  const terminalResize = useResizableHeight(320, 180, 640);
  const [connectProtocol, setConnectProtocol] = useState("");
  const [connectApiKey, setConnectApiKey] = useState("");
  const [connectBaseUrl, setConnectBaseUrl] = useState("");
  const [connectResolution, setConnectResolution] =
    useState<ResolveProviderConnectResponseRecord | null>(null);
  const [connectResolveBusy, setConnectResolveBusy] = useState(false);
  const [connectResolveError, setConnectResolveError] = useState<string | null>(null);
  const [connectBusy, setConnectBusy] = useState(false);
  const [theme, setTheme] = useState<ThemeId>("daylight");
  const [showThinking, setShowThinking] = useState(false);
  const [streaming, setStreaming] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [statusLine, setStatusLine] = useState("ready");
  const [banner, setBanner] = useState<string | null>(null);
  const [deletingSessions, setDeletingSessions] = useState(false);
  const [question, setQuestion] = useState<QuestionInteractionRecord | null>(null);
  const [permission, setPermission] = useState<PermissionInteractionRecord | null>(null);
  const [questionAnswers, setQuestionAnswers] = useState<Record<number, QuestionAnswerValue>>({});
  const [questionSubmitting, setQuestionSubmitting] = useState(false);
  const [permissionSubmitting, setPermissionSubmitting] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [composerDragActive, setComposerDragActive] = useState(false);
  const [selectedAttachmentIndex, setSelectedAttachmentIndex] = useState<number | null>(null);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [fileTree, setFileTree] = useState<FileTreeNodeRecord | null>(null);
  const [serviceRootPath, setServiceRootPath] = useState("");
  const [currentWorkspacePath, setCurrentWorkspacePath] = useState<string | null>(null);
  const [workspaceRootPath, setWorkspaceRootPath] = useState("");
  const [workspaceLoading, setWorkspaceLoading] = useState(false);
  const [selectedWorkspacePath, setSelectedWorkspacePath] = useState<string | null>(null);
  const [selectedWorkspaceType, setSelectedWorkspaceType] = useState<"file" | "directory">(
    "directory",
  );
  const [workspacePanelTab, setWorkspacePanelTab] = useState<WorkspacePanelTab>("files");
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [selectedFileContent, setSelectedFileContent] = useState("");
  const [savedFileContent, setSavedFileContent] = useState("");
  const [fileLoading, setFileLoading] = useState(false);
  const [fileSaving, setFileSaving] = useState(false);
  const [fileDeleting, setFileDeleting] = useState(false);
  const [fileUploading, setFileUploading] = useState(false);
  const [workspaceReloadToken, setWorkspaceReloadToken] = useState(0);
  const [pendingWorkspaceSelection, setPendingWorkspaceSelection] = useState<{
    path: string;
    type: "file" | "directory";
  } | null>(null);
  const feedRef = useRef<HTMLDivElement | null>(null);
  const preferencesReadyRef = useRef(false);
  const routeSyncSourceRef = useRef<"app" | "browser">("app");
  const routeInitializedRef = useRef(false);
  const selectedSessionRef = useRef<string | null>(null);
  const autoPreviewSignatureRef = useRef<string>("");
  const liveBlocksRef = useRef<SessionLiveBlockCache>({});
  const optimisticMessagesRef = useRef<SessionOptimisticFeedCache>({});
  const connectResolveRequestRef = useRef(0);
  const [messageReloadToken, setMessageReloadToken] = useState(0);

  const modelOptions = useMemo(() => flattenProviderModels(providers), [providers]);
  const settingsModeOptions = useMemo(
    () =>
      modes.map((mode) => ({
        key: modeKey(mode),
        label: mode.kind === "agent" ? mode.name : `${mode.kind}:${mode.name}`,
      })),
    [modes],
  );
  const composerReferences = useMemo(() => extractPromptReferences(composer), [composer]);
  const currentSession = useMemo(() => sessions.find((session) => session.id === selectedSessionId) ?? null, [selectedSessionId, sessions]);
  const activeModelRef = useMemo(
    () => resolveActiveModelRef(currentSession, selectedModel),
    [currentSession, selectedModel],
  );
  const activeProviderModel = useMemo(() => {
    if (!activeModelRef) return null;
    const target = activeModelRef.trim();
    for (const provider of providers) {
      for (const model of provider.models ?? []) {
        const fullId = `${provider.id}/${model.id}`;
        if (
          fullId === target ||
          model.id === target ||
          fullId.endsWith(`/${target}`)
        ) {
          return {
            ...model,
            fullId,
            providerId: provider.id,
            providerName: provider.name,
          };
        }
      }
    }
    return null;
  }, [activeModelRef, providers]);
  const workspaceSummaries = useMemo(
    () => buildWorkspaceSummaries(sessions, serviceRootPath),
    [serviceRootPath, sessions],
  );
  const currentWorkspaceSummary = useMemo(
    () =>
      workspaceSummaries.find((workspace) => workspace.path === currentWorkspacePath) ??
      workspaceSummaries[0] ??
      null,
    [currentWorkspacePath, workspaceSummaries],
  );
  const pluginWorkspacePath =
    currentWorkspaceSummary?.path ||
    currentWorkspacePath ||
    workspaceRootFromContext(workspaceContext) ||
    serviceRootPath ||
    null;
  const resolvedWorkspaceRootPath = workspaceRootFromContext(workspaceContext) || serviceRootPath;
  const resolvedWorkspaceMode = workspaceModeFromContext(workspaceContext);
  const sessionTree = useMemo(
    () => buildSessionTree(sessions, currentWorkspaceSummary?.path ?? null),
    [currentWorkspaceSummary?.path, sessions],
  );
  const selectedAttachment = (selectedAttachmentIndex !== null && attachments[selectedAttachmentIndex]) || attachments[attachments.length - 1] || null;
  const workspaceDirty = Boolean(selectedFilePath) && selectedFileContent !== savedFileContent;
  const workspaceBasePath =
    currentSession?.directory?.trim() ||
    currentWorkspaceSummary?.path ||
    workspaceRootFromContext(workspaceContext) ||
    workspaceRootPath ||
    serviceRootPath ||
    "";
  const workspaceTargetDirectory =
    selectedWorkspaceType === "directory" && selectedWorkspacePath
      ? selectedWorkspacePath
      : selectedFilePath
        ? parentDirectory(selectedFilePath) || workspaceBasePath
        : workspaceBasePath;
  const selectedWorkspaceReference = selectedWorkspacePath ? toWorkspaceReferencePath(selectedWorkspacePath, workspaceBasePath || workspaceRootPath) : null;
  const selectedWorkspaceFilename = selectedWorkspacePath ? selectedWorkspacePath.split("/").filter(Boolean).pop() || selectedWorkspacePath : null;
  const selectedWorkspaceIsRoot = Boolean(selectedWorkspacePath) && selectedWorkspaceType === "directory" && selectedWorkspacePath === (workspaceRootPath || workspaceBasePath);
  const multimodalComposer = useMultimodalComposer({
    apiJson,
    selectedModel,
    attachments,
    scopeKey: `${workspaceContext?.mode ?? "none"}:${workspaceContext?.identity?.workspace_root ?? ""}`,
  });
  const executionActivity = useExecutionActivity({
    selectedSessionId,
    apiJson,
    onError: setBanner,
    onInfo: setBanner,
  });
  const routeHighlightIds = useMemo(() => {
    const route = readWebSessionRoute();
    return route.sessionId === selectedSessionId ? new Set(route.highlightIds) : new Set<string>();
  }, [selectedSessionId, messages.length]);
  const sessionUsage = executionActivity.sessionUsage ?? currentSession?.telemetry?.usage ?? null;
  const composerContextTokens = useMemo(() => {
    const activeEstimate =
      executionActivity.activeStageSummary && isLiveStageStatus(executionActivity.activeStageSummary.status)
        ? executionActivity.activeStageSummary.estimated_context_tokens
        : undefined;
    return currentContextTokensFromSources(sessionUsage?.context_tokens, activeEstimate)
      ?? estimateContextTokensFromHistory(messageHistory);
  }, [executionActivity.activeStageSummary, messageHistory, sessionUsage?.context_tokens]);
  const effectiveRightPanelWidth = useMemo(() => {
    if (workspacePanelTab === "preview") return Math.max(rightResize.width, 640);
    if (workspacePanelTab === "insights") return Math.max(rightResize.width, 460);
    return rightResize.width;
  }, [rightResize.width, workspacePanelTab]);
  const lastAssistantTurnTokens = useMemo(() => {
    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const tokens = message.tokens;
      if (!tokens) continue;
      return {
        input: tokens.input ?? 0,
        output: tokens.output ?? 0,
      };
    }
    return null;
  }, [messageHistory]);
  const refreshExecutionActivity = executionActivity.refreshExecutionActivity;
  const conversationJump = useConversationJump({
    messages,
    feedRef,
    onMiss: setBanner,
  });
  useEffect(() => {
    const route = readWebSessionRoute();
    const messageId = route.messageId || route.highlightIds[0] || null;
    if (!messageId || route.sessionId !== selectedSessionId) return;
    conversationJump.jumpOrQueueConversationTarget({ messageId, label: messageId });
  }, [conversationJump, messages.length, selectedSessionId]);
  const schedulerNavigation = useSchedulerNavigation({
    sessions,
    selectedSessionId,
    currentSession,
    setSessions,
    setSelectedSessionId,
    apiJson,
    setBanner,
    executionActivity,
    jumpToConversationTarget: conversationJump.jumpOrQueueConversationTarget,
    queueConversationJumpTarget: conversationJump.queueConversationJumpTarget,
  });
  const workspaceLinkLabel = schedulerNavigation.activeStageId ? `stage ${schedulerNavigation.activeStageId}` : schedulerNavigation.currentBreadcrumbProvenance?.toolCallId ? `tool ${schedulerNavigation.currentBreadcrumbProvenance.toolCallId}` : schedulerNavigation.currentBreadcrumbProvenance?.stageId ? `stage ${schedulerNavigation.currentBreadcrumbProvenance.stageId}` : null;
  const workspaceLinkStageId = schedulerNavigation.activeStageId ?? schedulerNavigation.currentBreadcrumbProvenance?.stageId ?? null;
  const terminalSessions = useTerminalSessions({
    api,
    apiJson,
    setBanner,
    enabled: terminalOpen,
    defaultCwd: workspaceBasePath || currentSession?.directory || "",
  });

  const loadPendingQuestion = async (requestId: string, sessionId?: string | null) => {
    const questions = await apiJson<QuestionInfoResponseRecord[]>("/question");
    const pending = (questions ?? []).find((candidate) => candidate.id === requestId);
    if (!pending) return;
    const interaction = questionInteractionFromInfo(pending);
    if (sessionId && interaction.session_id && interaction.session_id !== sessionId) {
      return;
    }
    setQuestion(interaction);
    setQuestionAnswers({});
  };

  const sendPromptRequest = async (
    sessionId: string,
    payload: Record<string, unknown>,
  ): Promise<PromptResponseRecord> =>
    apiJson<PromptResponseRecord>(`/session/${sessionId}/prompt`, {
      method: "POST",
      body: JSON.stringify(payload),
    });

  const fetchSessions = async (): Promise<SessionRecord[]> => {
    const sessionData = await apiJson<SessionListResponseRecord>("/session?limit=500");
    return normalizeSessionRecords(sessionData?.items ?? []);
  };

  const copyMessageLink = async (message: FeedMessage) => {
    if (!selectedSessionId || !message.anchorId) return;
    const relative = buildWebSessionUrl({
      sessionId: selectedSessionId,
      messageId: message.anchorId,
      highlightIds: [],
    });
    const url = new URL(relative, window.location.origin).toString();
    await navigator.clipboard.writeText(url);
    setBanner("Copied message link");
  };

  const toggleMessageSelected = (message: FeedMessage) => {
    if (!message.anchorId) return;
    setSelectedMessageIds((current) => {
      const next = new Set(current);
      if (next.has(message.anchorId!)) next.delete(message.anchorId!);
      else next.add(message.anchorId!);
      return next;
    });
  };

  const copySelectedMessageLink = async () => {
    if (!selectedSessionId || selectedMessageIds.size === 0) return;
    const highlightIds = Array.from(selectedMessageIds);
    const relative = buildWebSessionUrl({
      sessionId: selectedSessionId,
      messageId: highlightIds[0] ?? null,
      highlightIds,
    });
    await navigator.clipboard.writeText(new URL(relative, window.location.origin).toString());
    setBanner(`Copied link for ${highlightIds.length} selected message${highlightIds.length === 1 ? "" : "s"}`);
  };

  const copySelectedMessagesMarkdown = async () => {
    const selected = messages.filter((message) => message.anchorId && selectedMessageIds.has(message.anchorId));
    if (selected.length === 0) return;
    const markdown = selected
      .map((message) => {
        const role = message.role === "user" ? "User" : message.role === "assistant" ? "Assistant" : message.kind;
        const title = message.title?.trim() ? ` - ${message.title.trim()}` : "";
        const text = message.text?.trim() || message.summary?.trim() || "[no text]";
        return `### ${role}${title}\n\n${text}`;
      })
      .join("\n\n---\n\n");
    await navigator.clipboard.writeText(markdown);
    setBanner(`Copied ${selected.length} selected message${selected.length === 1 ? "" : "s"} as Markdown`);
  };

  const reloadCoreSettingsData = async () => {
    try {
      const [providersData, modeData, connectSchema, context] = await Promise.all([
        apiJson<ConfigProvidersResponseRecord>("/config/providers"),
        apiJson<ExecutionMode[]>("/mode"),
        apiJson<ProviderConnectSchemaResponseRecord>(
          "/provider/connect/schema",
        ),
        apiJson<WorkspaceContextRecord>("/workspace/context"),
      ]);
      const prefs = applyPreferences(context.config ?? {});
      setProviders(providersData.providers ?? providersData.all ?? []);
      setKnownProviders(connectSchema.providers ?? []);
      setConnectProtocols(connectSchema.protocols ?? []);
      setWorkspaceContext(context);
      setServiceRootPath((current) => workspaceRootFromContext(context) || current);
      setTheme(THEMES.some((item) => item.id === prefs.theme) ? prefs.theme : "daylight");
      setSelectedMode(prefs.mode);
      setSelectedModel(prefs.model);
      setShowThinking(prefs.showThinking);
      setModes(
        (modeData ?? [])
          .filter((mode) => mode.hidden !== true)
          .filter((mode) => mode.kind !== "agent" || mode.mode !== "subagent"),
      );
    } catch (error) {
      setBanner(`Failed to refresh config data: ${formatError(error)}`);
    }
  };

  useEffect(() => {
    if (!selectedWorkspacePath) return;
    const nextIndex = attachments.findIndex((attachment) =>
      attachmentContainsWorkspacePath(attachment, selectedWorkspacePath),
    );
    if (nextIndex >= 0 && nextIndex !== selectedAttachmentIndex) {
      setSelectedAttachmentIndex(nextIndex);
    }
  }, [attachments, selectedAttachmentIndex, selectedWorkspacePath]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    selectedSessionRef.current = selectedSessionId;
  }, [selectedSessionId]);

  const selectSession = useCallback((sessionId: string | null) => {
    routeSyncSourceRef.current = "app";
    setSelectedSessionId(sessionId);
  }, []);

  useEffect(() => {
    if (!selectedSessionId) return;
    if (routeSyncSourceRef.current === "browser") {
      routeSyncSourceRef.current = "app";
      routeInitializedRef.current = true;
      return;
    }
    const route = readWebSessionRoute();
    if (!routeInitializedRef.current && route.sessionId === selectedSessionId) {
      routeInitializedRef.current = true;
      return;
    }
    if (
      route.sessionId === selectedSessionId &&
      (route.messageId || route.highlightIds.length > 0)
    ) {
      routeInitializedRef.current = true;
      return;
    }
    routeInitializedRef.current = true;
    writeWebSessionRoute({ sessionId: selectedSessionId, messageId: null, highlightIds: [] });
  }, [selectedSessionId]);

  useEffect(() => {
    const handlePopState = () => {
      const route = readWebSessionRoute();
      routeSyncSourceRef.current = "browser";
      setSelectedSessionId(route.sessionId);
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);

  useEffect(() => {
    autoPreviewSignatureRef.current = "";
    setMessageHistory([]);
    setSelectedMessageIds(new Set());
  }, [selectedSessionId]);

  useEffect(() => {
    const query = connectQuery.trim();
    if (!query) {
      connectResolveRequestRef.current += 1;
      setConnectResolveBusy(false);
      setConnectResolveError(null);
      setConnectResolution(null);
      return;
    }

    const requestId = connectResolveRequestRef.current + 1;
    connectResolveRequestRef.current = requestId;
    const timer = window.setTimeout(() => {
      setConnectResolveBusy(true);
      setConnectResolveError(null);
      void (async () => {
        try {
          const response = await apiJson<ResolveProviderConnectResponseRecord>(
            "/provider/connect/resolve",
            {
              method: "POST",
              body: JSON.stringify({ query }),
            },
          );
          if (connectResolveRequestRef.current !== requestId) return;
          setConnectResolution(response);
          setConnectProviderId(response.draft.provider_id);
          setConnectBaseUrl(response.draft.base_url ?? "");
          setConnectProtocol(
            response.draft.protocol ?? connectProtocols[0]?.id ?? "openai",
          );
        } catch (error) {
          if (connectResolveRequestRef.current !== requestId) return;
          setConnectResolution(null);
          setConnectResolveError(formatError(error));
        } finally {
          if (connectResolveRequestRef.current === requestId) {
            setConnectResolveBusy(false);
          }
        }
      })();
    }, 120);

    return () => window.clearTimeout(timer);
  }, [apiJson, connectProtocols, connectQuery, knownProviders]);

  useEffect(() => {
    const selectedWorkspace = currentSession?.directory?.trim();
    if (selectedWorkspace) {
      setCurrentWorkspacePath(selectedWorkspace);
      return;
    }
    setCurrentWorkspacePath((current) => {
      if (current && workspaceSummaries.some((workspace) => workspace.path === current)) {
        return current;
      }
      return workspaceSummaries[0]?.path ?? serviceRootPath ?? null;
    });
  }, [currentSession?.directory, serviceRootPath, workspaceSummaries]);

  useEffect(() => {
    if (!feedRef.current) return;
    feedRef.current.scrollTop = feedRef.current.scrollHeight;
  }, [messages]);

  useEffect(() => {
    let cancelled = false;

    const loadBootstrap = async () => {
      try {
        const [sessionData, providersData, modeData, context, connectSchema, paths] = await Promise.all([
          fetchSessions(),
          apiJson<ConfigProvidersResponseRecord>("/config/providers"),
          apiJson<ExecutionMode[]>("/mode"),
          apiJson<WorkspaceContextRecord>("/workspace/context"),
          apiJson<ProviderConnectSchemaResponseRecord>(
            "/provider/connect/schema",
          ),
          apiJson<PathsResponseRecord>("/path"),
        ]);

        if (cancelled) return;

        const nextProviders = providersData.providers ?? providersData.all ?? [];
        const nextModes = (modeData ?? [])
          .filter((mode) => mode.hidden !== true)
          .filter((mode) => mode.kind !== "agent" || mode.mode !== "subagent");
        const prefs = applyPreferences(context.config ?? {});
        const workspaceRoot = workspaceRootFromContext(context);

        setServiceRootPath(workspaceRoot || paths.cwd || "");
        setSessions(sessionData);
        setProviders(nextProviders);
        setKnownProviders(connectSchema.providers ?? []);
        setConnectProtocols(connectSchema.protocols ?? []);
        setWorkspaceContext(context);
        setModes(nextModes);
        setTheme(THEMES.some((item) => item.id === prefs.theme) ? prefs.theme : "daylight");
        setSelectedMode(prefs.mode);
        setSelectedModel(prefs.model);
        setShowThinking(prefs.showThinking);
        setConnectProtocol((current) => current || connectSchema.protocols?.[0]?.id || "");
        const routeSessionId = readWebSessionRoute().sessionId;
        const routeSessionExists = Boolean(
          routeSessionId && sessionData.some((session) => session.id === routeSessionId),
        );
        setSelectedSessionId((current) => current ?? (routeSessionExists ? routeSessionId : sessionData[0]?.id ?? null));
        preferencesReadyRef.current = true;
      } catch (error) {
        if (!cancelled) {
          setBanner(`Bootstrap failed: ${formatError(error)}`);
        }
      }
    };

    void loadBootstrap();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        await loadWebPlugins(apiJson, { workspacePath: pluginWorkspacePath });
      } catch (error) {
        if (!cancelled) {
          console.warn("[web-plugin] Reload failed", error);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [pluginWorkspacePath]);

  useEffect(() => {
    if (!preferencesReadyRef.current) return;
    const timer = window.setTimeout(() => {
      void api("/config", {
        method: "PATCH",
        body: JSON.stringify({
          uiPreferences: {
            webTheme: theme,
            webMode: selectedMode || null,
            webModel: selectedModel || null,
            showThinking,
          },
        }),
      }).catch((error) => {
        setBanner(`Failed to persist settings: ${formatError(error)}`);
      });
    }, 150);

    return () => window.clearTimeout(timer);
  }, [theme, selectedMode, selectedModel, showThinking]);

  useEffect(() => {
    if (!selectedSessionId) {
      setMessages([]);
      setMessageHistory([]);
      autoPreviewSignatureRef.current = "";
      return;
    }

    let cancelled = false;

    const loadHistory = async () => {
      setHistoryLoading(true);
      try {
        const history = await apiJson<MessageRecord[]>(`/session/${selectedSessionId}/message`);
        if (cancelled) return;
        setMessageHistory(history);
        const mergedHistory = mergeHistoryWithLiveBlocks(
          history,
          liveBlocksRef.current[selectedSessionId] ?? [],
          showThinking,
        );
        const merged = mergeOptimisticMessages(
          mergedHistory,
          optimisticMessagesRef.current[selectedSessionId] ?? [],
        );
        optimisticMessagesRef.current = {
          ...optimisticMessagesRef.current,
          [selectedSessionId]: merged.remaining,
        };
        setMessages(merged.messages);
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to load messages: ${formatError(error)}`);
        }
      } finally {
        if (!cancelled) {
          setHistoryLoading(false);
        }
      }
    };

    void loadHistory();
    return () => {
      cancelled = true;
    };
  }, [messageReloadToken, selectedSessionId, showThinking]);

  useEffect(() => {
    let cancelled = false;

    const loadTree = async () => {
      setWorkspaceLoading(true);
      setFileTree(null);
      setSelectedWorkspacePath(null);
      setSelectedWorkspaceType("directory");
      setSelectedFilePath(null);
      setSelectedFileContent("");
      setSavedFileContent("");

      try {
        const query =
          currentSession?.directory && currentSession.directory.trim()
            ? `?path=${encodeURIComponent(currentSession.directory)}`
            : "";
        const tree = await apiJson<FileTreeNodeRecord>(`/file/tree${query}`);
        if (cancelled) return;
        setFileTree(tree);
        setWorkspaceRootPath(tree.path);
        const preferredNode = pendingWorkspaceSelection
          ? findNodeByPath(tree, pendingWorkspaceSelection.path)
          : null;
        const fallbackFilePath = findFirstFile(tree);
        const fallbackNode = fallbackFilePath ? findNodeByPath(tree, fallbackFilePath) : tree;
        const nextNode = preferredNode ?? fallbackNode;

        setSelectedWorkspacePath(nextNode?.path ?? null);
        setSelectedWorkspaceType(nextNode?.type ?? "directory");
        setSelectedFilePath(nextNode?.type === "file" ? nextNode.path : null);
        setPendingWorkspaceSelection(null);
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to load workspace tree: ${formatError(error)}`);
          setWorkspaceRootPath(currentSession?.directory || "");
        }
      } finally {
        if (!cancelled) {
          setWorkspaceLoading(false);
        }
      }
    };

    void loadTree();
    return () => {
      cancelled = true;
    };
  }, [currentSession?.directory, selectedSessionId, workspaceReloadToken]);

  useEffect(() => {
    if (!selectedFilePath) {
      setSelectedFileContent("");
      setSavedFileContent("");
      return;
    }

    let cancelled = false;

    const loadFile = async () => {
      setFileLoading(true);
      try {
        const response = await apiJson<FileContentResponseRecord>(
          `/file/content?path=${encodeURIComponent(selectedFilePath)}`,
        );
        if (cancelled) return;
        setSelectedFileContent(response.content ?? "");
        setSavedFileContent(response.content ?? "");
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to read file: ${formatError(error)}`);
        }
      } finally {
        if (!cancelled) {
          setFileLoading(false);
        }
      }
    };

    void loadFile();
    return () => {
      cancelled = true;
    };
  }, [selectedFilePath]);

  useEffect(() => {
    let active = true;
    let controller: AbortController | null = null;

    const refreshSessions = async () => {
      try {
        const sessionData = await fetchSessions();
        if (!active) return;
        setSessions(sessionData);
        setSelectedSessionId((current) => {
          if (current && sessionData.some((session) => session.id === current)) {
            return current;
          }
          return sessionData[0]?.id ?? null;
        });
      } catch (error) {
        if (active) {
          setBanner(`Failed to refresh sessions: ${formatError(error)}`);
        }
      }
    };

    const reloadProvidersAndModes = async () => {
      try {
        const [providersData, modeData, connectSchema] = await Promise.all([
          apiJson<ConfigProvidersResponseRecord>("/config/providers"),
          apiJson<ExecutionMode[]>("/mode"),
          apiJson<ProviderConnectSchemaResponseRecord>(
            "/provider/connect/schema",
          ),
        ]);
        if (!active) return;
        setProviders(providersData.providers ?? providersData.all ?? []);
        setKnownProviders(connectSchema.providers ?? []);
        setConnectProtocols(connectSchema.protocols ?? []);
        setModes(
          (modeData ?? [])
            .filter((mode) => mode.hidden !== true)
            .filter((mode) => mode.kind !== "agent" || mode.mode !== "subagent"),
        );
      } catch (error) {
        if (active) {
          setBanner(`Failed to refresh config data: ${formatError(error)}`);
        }
      }
    };

    const handleServerEvent = (payload: unknown) => {
      const event = payload as Record<string, unknown>;
      const type = typeof event.type === "string" ? event.type : "";
      const eventSessionId =
        typeof event.sessionID === "string"
          ? event.sessionID
          : typeof event.session_id === "string"
            ? event.session_id
            : undefined;

      if (type === "output_block" && eventSessionId === selectedSessionRef.current) {
        const rawBlock = event.block as OutputBlock | undefined;
        const block = rawBlock
          ? {
              ...rawBlock,
              id:
                typeof rawBlock.id === "string"
                  ? rawBlock.id
                  : typeof event.id === "string"
                    ? event.id
                    : undefined,
            }
          : undefined;
        if (!block) return;
        liveBlocksRef.current = {
          ...liveBlocksRef.current,
          [eventSessionId]: appendLiveBlock(liveBlocksRef.current[eventSessionId] ?? [], block),
        };
        setMessages((current) => applyOutputBlock(current, block, showThinking));
        return;
      }

      if (type === "error" && eventSessionId === selectedSessionRef.current) {
        setMessages((current) =>
          applyOutputBlock(
            current,
            {
              kind: "status",
              tone: "error",
              text: String(event.error ?? "Unknown error"),
            },
            showThinking,
          ),
        );
        setStreaming(false);
        setStatusLine("idle");
        return;
      }

      if (type === "session.updated") {
        void refreshSessions();
        if (eventSessionId === selectedSessionRef.current) {
          setMessageReloadToken((current) => current + 1);
        }
        return;
      }

      if (type === "config.updated") {
        void reloadProvidersAndModes();
        return;
      }

      if (type === "session.status" && eventSessionId === selectedSessionRef.current) {
        const rawStatus = event.status;
        const status =
          typeof rawStatus === "string"
            ? rawStatus
            : rawStatus && typeof rawStatus === "object" && "type" in rawStatus
              ? String((rawStatus as { type?: unknown }).type ?? "")
              : String(rawStatus ?? "");
        if (status === "idle" || status === "complete" || status === "error") {
          setStreaming(false);
          setStatusLine(status || "idle");
        }
        return;
      }

      if (type === "question.created" && eventSessionId === selectedSessionRef.current) {
        setQuestion(questionInteractionFromEvent(event, eventSessionId));
        setQuestionAnswers({});
        setStreaming(false);
        setStatusLine("awaiting_user");
        return;
      }

      if (type === "question.resolved" && eventSessionId === selectedSessionRef.current) {
        setQuestion(null);
        setQuestionAnswers({});
        setQuestionSubmitting(false);
        return;
      }

      if (type === "execution.topology.changed" && eventSessionId === selectedSessionRef.current) {
        void refreshExecutionActivity(eventSessionId);
        return;
      }

      if (type === "permission.requested" && eventSessionId === selectedSessionRef.current) {
        setPermission(permissionInteractionFromEvent(event, eventSessionId));
        return;
      }

      if (type === "permission.resolved") {
        const resolvedPermissionId = String(event.permissionID ?? "");
        setPermission((current) => {
          if (!current) return null;
          if (resolvedPermissionId && current.permission_id !== resolvedPermissionId) {
            return current;
          }
          return null;
        });
        setPermissionSubmitting(false);
      }
    };

    const connect = async () => {
      while (active) {
        controller = new AbortController();
        try {
          const response = await fetch(apiUrl("/event"), {
            headers: { Accept: "text/event-stream" },
            signal: controller.signal,
          });
          if (!response.ok) {
            throw new Error(`${response.status} ${response.statusText}`);
          }
          await parseSSE(response, (_eventName, payload) => handleServerEvent(payload));
        } catch (error) {
          if (!active || controller.signal.aborted) return;
          setStatusLine("reconnecting");
          await new Promise((resolve) => window.setTimeout(resolve, 1500));
        }
      }
    };

    void connect();
    return () => {
      active = false;
      controller?.abort();
    };
  }, [refreshExecutionActivity, showThinking]);

  const createSession = async (options?: {
    directory?: string;
    title?: string;
    projectId?: string;
  }) => {
    const created = await apiJson<SessionRecord>("/session", {
      method: "POST",
      body: JSON.stringify({
        directory: options?.directory,
        title: options?.title,
        project_id: options?.projectId,
      }),
    });
      const normalized = normalizeSessionRecord(created);
    setSessions((current) =>
      normalizeSessionRecords([normalized, ...current.filter((item) => item.id !== normalized.id)]),
    );
    setCurrentWorkspacePath(normalized.directory?.trim() || options?.directory || null);
    selectedSessionRef.current = normalized.id;
    setSelectedSessionId(normalized.id);
    return normalized.id;
  };

  const selectWorkspace = (workspacePath: string) => {
    setCurrentWorkspacePath(workspacePath);
    const workspaceSessions = sessions
      .filter((session) => session.directory?.trim() === workspacePath)
      .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
    const preferred =
      workspaceSessions.find((session) => !session.parent_id) ?? workspaceSessions[0] ?? null;
    if (preferred) {
      setSelectedSessionId(preferred.id);
    }
  };

  const createProject = async (input: { path: string; title?: string }) => {
    const baseRoot = serviceRootPath || workspaceBasePath || workspaceRootPath;
    const targetPath = resolveWorkspacePath(baseRoot, input.path);
    if (!targetPath) {
      setBanner("Project path is required");
      return;
    }

    try {
      const directory = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
        method: "POST",
        body: JSON.stringify({ path: targetPath }),
      });
      const folderName = basenamePath(directory.path);
      await createSession({
        directory: directory.path,
        projectId: folderName,
        title: input.title || `${folderName} workspace`,
      });
      setPendingWorkspaceSelection({ path: directory.path, type: "directory" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created project ${folderName}`);
    } catch (error) {
      setBanner(`Failed to create project: ${formatError(error)}`);
    }
  };

  const deleteSelectedSessions = async (sessionIds: string[]) => {
    const uniqueIds = Array.from(new Set(sessionIds.map((id) => id.trim()).filter(Boolean)));
    if (uniqueIds.length === 0 || deletingSessions) return;

    const sessionById = new Map(sessions.map((session) => [session.id, session]));
    const selectedSet = new Set(uniqueIds);
    const deleteRoots = uniqueIds.filter((sessionId) => {
      let cursor = sessionById.get(sessionId)?.parent_id ?? null;
      while (cursor) {
        if (selectedSet.has(cursor)) return false;
        cursor = sessionById.get(cursor)?.parent_id ?? null;
      }
      return true;
    });

    if (deleteRoots.length === 0) return;

    setDeletingSessions(true);
    setBanner(null);

    try {
      for (const sessionId of deleteRoots) {
        await api(`/session/${sessionId}`, { method: "DELETE" });
      }

      const sessionData = await fetchSessions();
      setSessions(sessionData);

      const currentStillExists =
        selectedSessionId && sessionData.some((session) => session.id === selectedSessionId);
      if (!currentStillExists) {
        const workspacePath = currentWorkspaceSummary?.path ?? currentWorkspacePath;
        const workspaceSessions = sessionData
          .filter((session) => session.directory?.trim() === workspacePath)
          .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
        const fallback =
          workspaceSessions.find((session) => !session.parent_id) ?? workspaceSessions[0] ?? null;
        setSelectedSessionId(fallback?.id ?? null);
      }

      setBanner(`Deleted ${deleteRoots.length} session${deleteRoots.length === 1 ? "" : "s"}.`);
    } catch (error) {
      setBanner(`Failed to delete sessions: ${formatError(error)}`);
    } finally {
      setDeletingSessions(false);
    }
  };

  const submitPrompt = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const content = composer.trim();
    const promptParts = attachments;
    if ((!content && promptParts.length === 0) || streaming) return;

    setBanner(null);

    try {
      const multimodalGate = await multimodalComposer.preflightBeforeSubmit();
      if (multimodalGate.blocked) {
        setBanner(multimodalGate.banner);
        return;
      }
      if (multimodalGate.banner) {
        setBanner(multimodalGate.banner);
      }
    } catch (error) {
      setBanner(`Multimodal preflight unavailable: ${formatError(error)}`);
    }

    let sessionId = selectedSessionRef.current;
    if (!sessionId) {
      try {
        sessionId = await createSession();
      } catch (error) {
        setBanner(`Failed to create session: ${formatError(error)}`);
        return;
      }
    }
    selectedSessionRef.current = sessionId;

    const preview = promptPreviewText(content, promptParts);
    const optimisticMessage = createOptimisticUserFeedMessage(preview);
    optimisticMessagesRef.current = {
      ...optimisticMessagesRef.current,
      [sessionId]: [
        ...(optimisticMessagesRef.current[sessionId] ?? []),
        optimisticMessage,
      ],
    };
    setMessages((current) => [...current, optimisticMessage]);
    setComposer("");
    setAttachments([]);
    setStreaming(true);
    setStatusLine("running");

    try {
      const payload: Record<string, unknown> = {
        message: content || undefined,
      };
      if (selectedModel) payload.model = selectedModel;
      if (promptParts.length > 0) payload.parts = promptParts;
      if (selectedMode) {
        const [kind, id] = selectedMode.split(":", 2);
        if (kind === "agent") payload.agent = id;
        if (kind === "preset" || kind === "profile") payload.scheduler_profile = id;
      }

      const response = await sendPromptRequest(sessionId, payload);
      if (response.status === "awaiting_user") {
        setStreaming(false);
        setStatusLine("awaiting_user");
        if (response.pending_question_id) {
          await loadPendingQuestion(response.pending_question_id, sessionId);
        }
      }
    } catch (error) {
      setMessages((current) =>
        applyOutputBlock(
          current,
          {
            kind: "status",
            tone: "error",
            text: formatError(error),
          },
          showThinking,
        ),
      );
      setBanner(`Prompt failed: ${formatError(error)}`);
      setStreaming(false);
      setStatusLine("idle");
    }

    try {
      const sessionData = await fetchSessions();
      setSessions(sessionData);
    } catch {
      // best effort
    }
  };

  const attachComposerFiles = async (files: File[], failurePrefix: string) => {
    if (!files.length) return;

    const nextParts = await prepareComposerAttachments(files, {
      workspaceBasePath,
      uploadJson: apiJson,
    }).catch((error) => {
      setBanner(`${failurePrefix}: ${formatError(error)}`);
      return [];
    });

    if (!nextParts.length) return;
    setAttachments((current) => {
      setSelectedAttachmentIndex(current.length + nextParts.length - 1);
      return [...current, ...nextParts];
    });
    const uploadedPaths = nextParts
      .map((part) => attachmentWorkspacePath(part))
      .filter((path): path is string => Boolean(path && path.includes("/.rocode/uploads/")));
    if (uploadedPaths.length && !workspaceDirty) {
      setPendingWorkspaceSelection(
        selectedWorkspacePath
          ? { path: selectedWorkspacePath, type: selectedWorkspaceType }
          : workspaceRootPath
            ? { path: workspaceRootPath, type: "directory" }
            : null,
      );
      setWorkspaceReloadToken((current) => current + 1);
    }
    setBanner(
      nextParts.length === 1
        ? `Attached ${attachmentLabel(nextParts[0])}`
        : `Attached ${nextParts.length} items`,
    );
  };

  const handleFileChange = async (event: ChangeEvent<HTMLInputElement>) => {
    await attachComposerFiles(Array.from(event.target.files ?? []), "Attachment failed");
    event.target.value = "";
  };

  const handleComposerPaste = async (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData.files ?? []).filter((file) =>
      file.type.startsWith("image/"),
    );
    if (!files.length) return;
    event.preventDefault();
    await attachComposerFiles(files, "Image paste failed");
  };

  const handleComposerDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setComposerDragActive(false);
    await attachComposerFiles(droppedFiles(event.dataTransfer), "Drop attach failed");
  };

  const submitQuestion = async () => {
    if (!question) return;
    setQuestionSubmitting(true);
    try {
      const answers = question.questions.map((item, index) =>
        normalizedAnswerValues(questionAnswers[index], Boolean(item.multiple)),
      );
      await api(`/question/${question.request_id}/reply`, {
        method: "POST",
        body: JSON.stringify({ answers }),
      });
      setQuestion(null);
      setQuestionAnswers({});
      const sessionId = question.session_id ?? selectedSessionRef.current;
      if (sessionId) {
        const session = await apiJson<SessionRecord>(`/session/${sessionId}`);
        const pending = pendingCommandFromSession(session, question.request_id);
        if (pending) {
          const argumentsText = mergePendingCommandArguments(pending, answers);
          const response = await sendPromptRequest(sessionId, {
            command: pending.command,
            arguments: argumentsText || undefined,
            model: selectedModel || undefined,
          });
          if (response.status === "awaiting_user") {
            setStreaming(false);
            setStatusLine("awaiting_user");
            if (response.pending_question_id) {
              await loadPendingQuestion(response.pending_question_id, sessionId);
            }
          } else {
            setStreaming(true);
            setStatusLine("running");
          }
        }
      }
    } catch (error) {
      setBanner(`Question reply failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  };

  const rejectQuestion = async () => {
    if (!question) return;
    setQuestionSubmitting(true);
    try {
      await api(`/question/${question.request_id}/reject`, { method: "POST" });
      setQuestion(null);
      setQuestionAnswers({});
    } catch (error) {
      setBanner(`Question reject failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  };

  const replyPermission = async (reply: "once" | "always" | "reject") => {
    const currentPermission = permission;
    if (!currentPermission || permissionSubmitting) return;
    setPermissionSubmitting(true);
    try {
      await api(`/permission/${currentPermission.permission_id}/reply`, {
        method: "POST",
        body: JSON.stringify({ reply }),
      });
    } catch (error) {
      setBanner(`Permission reply failed: ${formatError(error)}`);
    } finally {
      setPermission((current) =>
        current?.permission_id === currentPermission.permission_id ? null : current,
      );
      setPermissionSubmitting(false);
    }
  };

  const connectProvider = async () => {
    const providerId = connectProviderId.trim();
    const apiKey = connectApiKey.trim();
    if (!providerId || !apiKey) {
      setBanner("provider_id and api_key are required");
      return;
    }

    const baseUrl = connectBaseUrl.trim();
    const defaultProtocol = connectProtocols[0]?.id || "openai";
    const protocol = connectProtocol.trim() || defaultProtocol;
    const suggestedDraft = connectResolution?.draft ?? null;
    const suggestedBaseUrl = suggestedDraft?.base_url?.trim() ?? "";
    const suggestedProtocol = suggestedDraft?.protocol?.trim() || defaultProtocol;

    setConnectBusy(true);
    try {
      const useKnownQuickConnect =
        suggestedDraft?.mode === "known" &&
        suggestedDraft.provider_id.toLowerCase() === providerId.toLowerCase() &&
        ((baseUrl === suggestedBaseUrl && protocol === suggestedProtocol) || !baseUrl);
      if (!useKnownQuickConnect && !baseUrl) {
        setBanner("Custom or advanced provider connect requires a base URL.");
        return;
      }

      await api("/provider/connect", {
        method: "POST",
        body: JSON.stringify({
          provider_id: providerId,
          api_key: apiKey,
          base_url: useKnownQuickConnect ? undefined : baseUrl,
          protocol: useKnownQuickConnect ? undefined : protocol,
        }),
      });
      setConnectApiKey("");
      setConnectBaseUrl("");
      await reloadCoreSettingsData();
      setBanner(`Connected provider ${providerId}`);
    } catch (error) {
      setBanner(`Provider connect failed: ${formatError(error)}`);
    } finally {
      setConnectBusy(false);
    }
  };

  const lastAssistant = findLastMessage(
    messages,
    (message) => message.kind === "message" && message.role === "assistant",
  );

  const confirmDiscardWorkspaceChanges = (targetLabel: string) => {
    if (!workspaceDirty) {
      return true;
    }

    return window.confirm(
      `Unsaved changes in ${selectedFilePath || "the current file"} will be lost. Continue to ${targetLabel}?`,
    );
  };

  const selectWorkspaceNode = (path: string, typeHint?: "file" | "directory") => {
    const requestedType = typeHint ?? "file";
    if (
      selectedFilePath &&
      workspaceDirty &&
      (path !== selectedWorkspacePath || requestedType !== selectedWorkspaceType) &&
      !confirmDiscardWorkspaceChanges("switch workspace selection")
    ) {
      return false;
    }

    const node = findNodeByPath(fileTree, path);
    if (node) {
      setSelectedWorkspacePath(node.path);
      setSelectedWorkspaceType(node.type);
      setSelectedFilePath(node.type === "file" ? node.path : null);
      setWorkspacePanelTab(node.type === "file" ? "preview" : "files");
      return true;
    }

    setPendingWorkspaceSelection({ path, type: requestedType });
    setWorkspacePanelTab(requestedType === "file" ? "preview" : "files");
    setWorkspaceReloadToken((current) => current + 1);
    return true;
  };

  useEffect(() => {
    if (!selectedSessionId || !workspaceBasePath) return;
    const previewPath = previewPathFromMessageMetadata(messageHistory, workspaceBasePath);
    if (!previewPath) return;

    const signature = `${selectedSessionId}:${previewPath}`;
    if (autoPreviewSignatureRef.current === signature) {
      return;
    }

    if (selectWorkspaceNode(previewPath, "file")) {
      autoPreviewSignatureRef.current = signature;
      setWorkspacePanelTab("preview");
    }
  }, [
    messageHistory,
    selectedSessionId,
    workspaceBasePath,
  ]);

  const locateAttachmentInWorkspace = (attachment: PromptPart) => {
    const path = attachmentWorkspacePath(attachment);
    if (!path) return;
    selectWorkspaceNode(path, attachment.type === "file" && attachment.mime === "application/x-directory" ? "directory" : "file");
    schedulerNavigation.restoreActiveStage();
    setBanner(`Located ${attachmentLabel(attachment)} in workspace`);
  };

  const removeAttachmentAt = (index: number) => {
    setAttachments((current) => current.filter((_, itemIndex) => itemIndex !== index));
    setSelectedAttachmentIndex((current) => {
      if (current === null) return null;
      if (current === index) return null;
      if (current > index) return current - 1;
      return current;
    });
  };

  const saveSelectedFile = async () => {
    if (!selectedFilePath || fileSaving) return;
    setFileSaving(true);
    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: selectedFilePath,
          content: selectedFileContent,
        }),
      });
      setSavedFileContent(selectedFileContent);
      setBanner(`Saved ${selectedFilePath}`);
    } catch (error) {
      setBanner(`Failed to save file: ${formatError(error)}`);
    } finally {
      setFileSaving(false);
    }
  };

  const createWorkspaceDirectory = async () => {
    const requestedPath = window.prompt("New folder path", "notes");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a folder and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("Directory path is required");
      return;
    }

    try {
      const response = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
        method: "POST",
        body: JSON.stringify({
          path: targetPath,
        }),
      });
      setPendingWorkspaceSelection({ path: response.path, type: "directory" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created directory ${response.path}`);
    } catch (error) {
      setBanner(`Failed to create directory: ${formatError(error)}`);
    }
  };

  const createWorkspaceFile = async () => {
    const requestedPath = window.prompt("New file path", "notes.md");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a file and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("File path is required");
      return;
    }

    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: targetPath,
          content: "",
          create_parents: true,
        }),
      });
      setPendingWorkspaceSelection({ path: targetPath, type: "file" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created ${targetPath}`);
    } catch (error) {
      setBanner(`Failed to create file: ${formatError(error)}`);
    }
  };

  const deleteSelectedWorkspaceNode = async () => {
    if (!selectedWorkspacePath || fileDeleting) return;
    if (selectedWorkspaceIsRoot) {
      setBanner("Refusing to delete the workspace root directory");
      return;
    }
    if (!confirmDiscardWorkspaceChanges("delete the selected workspace node")) {
      return;
    }
    if (!window.confirm(`Delete ${selectedWorkspacePath}?`)) return;

    setFileDeleting(true);
    try {
      await api("/file", {
        method: "DELETE",
        body: JSON.stringify({
          path: selectedWorkspacePath,
          recursive: selectedWorkspaceType === "directory",
        }),
      });
      const nextPath =
        selectedWorkspaceType === "file"
          ? parentDirectory(selectedWorkspacePath) || workspaceBasePath
          : parentDirectory(selectedWorkspacePath) || workspaceBasePath;
      setPendingWorkspaceSelection(nextPath ? { path: nextPath, type: "directory" } : null);
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Deleted ${selectedWorkspacePath}`);
    } catch (error) {
      setBanner(`Failed to delete selection: ${formatError(error)}`);
    } finally {
      setFileDeleting(false);
    }
  };

  const downloadSelectedFile = () => {
    if (!selectedFilePath) return;
    window.location.assign(apiUrl(`/file/download?path=${encodeURIComponent(selectedFilePath)}`));
  };

  const insertWorkspaceReference = () => {
    if (!selectedWorkspaceReference) return;
    setComposer((current) => appendReferenceToken(current, selectedWorkspaceReference));
    setBanner(`Inserted @${selectedWorkspaceReference}`);
  };

  const attachSelectedWorkspaceNode = () => {
    if (!selectedWorkspacePath) return;

    const nextAttachment: PromptPart = {
      type: "file",
      url: fileUrlFromPath(selectedWorkspacePath),
      filename: selectedWorkspaceReference || selectedWorkspaceFilename || "attachment",
      mime: guessWorkspaceMime(selectedWorkspacePath, selectedWorkspaceType),
    };

    setAttachments((current) => {
      if (current.some((part) => part.type === "file" && part.url === nextAttachment.url)) {
        return current;
      }
      setSelectedAttachmentIndex(current.length);
      return [...current, nextAttachment];
    });
    setBanner(
      selectedWorkspaceType === "directory"
        ? `Attached directory ${selectedWorkspaceReference || selectedWorkspacePath}`
        : `Attached file ${selectedWorkspaceReference || selectedWorkspacePath}`,
    );
  };

  const uploadWorkspaceFiles = async (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    if (!files.length || fileUploading) return;

    if (!confirmDiscardWorkspaceChanges("upload files and refresh workspace")) {
      event.target.value = "";
      return;
    }

    setFileUploading(true);
    try {
      const payloadFiles = await Promise.all(
        files.map(
          (file) =>
            new Promise<{ name: string; content: string; mime?: string }>((resolve, reject) => {
              const reader = new FileReader();
              reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
              reader.onload = () =>
                resolve({
                  name: file.name,
                  content: String(reader.result ?? ""),
                  mime: file.type || undefined,
                });
              reader.readAsDataURL(file);
            }),
        ),
      );

      const response = await apiJson<UploadFilesResponseRecord>("/file/upload", {
        method: "POST",
        body: JSON.stringify({
          path: workspaceTargetDirectory || workspaceBasePath || undefined,
          files: payloadFiles,
        }),
      });

      if (response.files[0]?.path) {
        setPendingWorkspaceSelection({ path: response.files[0].path, type: "file" });
      }
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(
        response.files.length === 1
          ? `Uploaded ${response.files[0]?.name ?? "1 file"}`
          : `Uploaded ${response.files.length} files`,
      );
    } catch (error) {
      setBanner(`Failed to upload files: ${formatError(error)}`);
    } finally {
      event.target.value = "";
      setFileUploading(false);
    }
  };

  return (
    <div className="roc-app-shell flex h-dvh flex-col overflow-hidden bg-background text-foreground font-sans">
      <div className="flex flex-1 overflow-hidden">
        {leftSidebarOpen && (
          <>
            <div className="shrink-0 overflow-hidden border-r border-border/50 bg-sidebar" style={{ width: leftResize.width }}>
              <SessionSidebar
                workspaces={workspaceSummaries}
                currentWorkspacePath={currentWorkspaceSummary?.path ?? null}
                currentWorkspaceLabel={currentWorkspaceSummary?.label ?? null}
                currentWorkspaceRootPath={resolvedWorkspaceRootPath || currentWorkspaceSummary?.path || null}
                currentWorkspaceMode={resolvedWorkspaceMode}
                sessionTree={sessionTree}
                selectedSessionId={selectedSessionId}
                deletingSessions={deletingSessions}
                onCreateProject={(input) => {
                  void createProject(input);
                }}
                onCreateSession={() => {
                  void createSession({
                    directory: (currentWorkspaceSummary?.path ?? serviceRootPath) || undefined,
                  });
                }}
                onDeleteSessions={(sessionIds) => {
                  void deleteSelectedSessions(sessionIds);
                }}
                onSelectWorkspace={selectWorkspace}
                onSelectSession={selectSession}
                onHideSidebar={() => setLeftSidebarOpen(false)}
              />
            </div>
            <div className={leftResize.handleClassName} onMouseDown={leftResize.handleMouseDown} />
          </>
        )}

        <div className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
          {!leftSidebarOpen ? (
            <div className="absolute left-4 top-3 z-20 md:left-5">
              <button
                onClick={() => setLeftSidebarOpen(true)}
                className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
                title="Show sidebar"
              >
                <PanelLeftIcon className="size-4" />
              </button>
            </div>
          ) : null}
          <div className="absolute right-4 top-3 z-20 flex items-center gap-1.5 md:right-5">
            {!rightSidebarOpen && selectedWorkspaceFilename ? (
              <button
                onClick={() => setRightSidebarOpen(true)}
                className="hidden items-center gap-1.5 rounded-full border border-border/55 bg-background/78 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground md:flex"
                title="Show workspace"
              >
                <span className="truncate max-w-[10rem]">{selectedWorkspaceFilename}</span>
              </button>
            ) : null}
            <button
              onClick={() => setRightSidebarOpen((value) => !value)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={rightSidebarOpen ? "Hide workspace" : "Show workspace"}
            >
              <FolderTreeIcon className={cn("size-4", rightSidebarOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setTerminalOpen((value) => !value)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={terminalOpen ? "Hide terminal" : "Show terminal"}
            >
              <TerminalSquareIcon className={cn("size-4", terminalOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setSettingsOpen(true)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title="Settings"
            >
              <SettingsIcon className="size-4" />
            </button>
          </div>
          {banner ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div className="roc-banner flex items-start gap-3" data-tone="warning">
                <div className="roc-status-orb mt-0.5 shrink-0" data-tone="loading">
                  <AlertTriangleIcon className="size-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="roc-section-label">Attention</div>
                  <p className="mt-1 text-sm leading-6 text-current/92">{banner}</p>
                </div>
                <button
                  type="button"
                  className="roc-banner-dismiss shrink-0"
                  aria-label="Dismiss status message"
                  onClick={() => setBanner(null)}
                >
                  <XIcon className="size-4" />
                </button>
              </div>
            </div>
          ) : null}

          {selectedMessageIds.size > 0 ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div className="roc-panel flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <span className="text-sm text-muted-foreground">
                  {selectedMessageIds.size} message{selectedMessageIds.size === 1 ? "" : "s"} selected
                </span>
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => void copySelectedMessageLink()}
                  >
                    Copy selected link
                  </button>
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => void copySelectedMessagesMarkdown()}
                  >
                    Copy Markdown
                  </button>
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => setSelectedMessageIds(new Set())}
                  >
                    Clear
                  </button>
                </div>
              </div>
            </div>
          ) : null}

          <ConversationFeedPanel
            sessionId={selectedSessionId}
            feedRef={feedRef}
            historyLoading={historyLoading}
            messages={messages}
            highlightedFeedId={conversationJump.highlightedFeedId}
            highlightedMessageIds={routeHighlightIds}
            activeStageId={schedulerNavigation.previewStageId ?? schedulerNavigation.activeStageId}
            activeToolCallId={schedulerNavigation.activeToolCallId}
            selectedMessageIds={selectedMessageIds}
            streaming={streaming}
            onCopyMessageLink={copyMessageLink}
            onToggleMessageSelected={toggleMessageSelected}
            onNavigateStage={schedulerNavigation.navigateToStage}
            onNavigateChildSession={schedulerNavigation.navigateToChildSession}
          />

          <div className="shrink-0 px-4 pb-5 pt-2 md:px-5">
            <ComposerSection
              composer={composer}
              composerDragActive={composerDragActive}
              streaming={streaming}
              multimodalHints={multimodalComposer.hints}
              allowAudioInput={multimodalComposer.policy?.allow_audio_input ?? true}
              allowImageInput={multimodalComposer.policy?.allow_image_input ?? true}
              allowFileInput={multimodalComposer.policy?.allow_file_input ?? true}
              modeOptions={settingsModeOptions}
              selectedMode={selectedMode}
              onModeChange={setSelectedMode}
              providers={providers}
              selectedModel={selectedModel}
              onModelChange={setSelectedModel}
              references={composerReferences}
              attachments={attachments}
              selectedAttachmentIndex={selectedAttachmentIndex}
              selectedAttachment={selectedAttachment}
              selectedWorkspacePath={selectedWorkspacePath}
              workspaceRootPath={workspaceBasePath || workspaceRootPath}
              contextTokensUsed={composerContextTokens}
              contextTokensLimit={activeProviderModel?.context_window ?? null}
              lastTurnInputTokens={lastAssistantTurnTokens?.input ?? null}
              lastTurnOutputTokens={lastAssistantTurnTokens?.output ?? null}
              inputPricePerMillion={activeProviderModel?.cost_per_million_input ?? null}
              outputPricePerMillion={activeProviderModel?.cost_per_million_output ?? null}
              activeStageId={schedulerNavigation.activeStageId}
              provenance={schedulerNavigation.currentBreadcrumbProvenance}
              onPreviewStage={schedulerNavigation.previewStage}
              onSubmit={submitPrompt}
              onRemoveReference={(reference) => setComposer((current) => removePromptReference(current, reference))}
              onRemoveAttachment={removeAttachmentAt}
              onSelectAttachment={(index, attachment) => {
                setSelectedAttachmentIndex(index);
                locateAttachmentInWorkspace(attachment as PromptPart);
              }}
              onLocateAttachment={(attachment) => locateAttachmentInWorkspace(attachment as PromptPart)}
              onNavigateStage={schedulerNavigation.navigateToStage}
              onNavigateProvenanceSession={schedulerNavigation.navigateToProvenanceSession}
              onNavigateProvenanceStage={schedulerNavigation.navigateToProvenanceStage}
              onNavigateProvenanceToolCall={schedulerNavigation.navigateToProvenanceToolCall}
              onDragEnter={(event) => {
                if (event.dataTransfer.types.includes("Files")) {
                  setComposerDragActive(true);
                }
              }}
              onDragOver={(event) => {
                if (!event.dataTransfer.types.includes("Files")) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "copy";
                setComposerDragActive(true);
              }}
              onDragLeave={(event) => {
                if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                setComposerDragActive(false);
              }}
              onDrop={(event) => void handleComposerDrop(event)}
              onFileChange={(event) => void handleFileChange(event)}
              onPaste={(event) => void handleComposerPaste(event)}
              onComposerChange={setComposer}
            />
          </div>

          {terminalOpen ? (
            <div className="shrink-0 px-4 pb-5 md:px-5">
              <div className="w-full overflow-hidden rounded-2xl border border-border/35 bg-sidebar shadow-sm">
                <div
                  className={terminalResize.handleClassName}
                  onMouseDown={terminalResize.handleMouseDown}
                  title="Resize terminal"
                />
                <div className="min-h-0 overflow-hidden" style={{ height: terminalResize.height }}>
                  <DeferredTerminalPanel
                    expanded={terminalOpen}
                    onExpand={() => setTerminalOpen(true)}
                    terminal={terminalSessions}
                  />
                </div>
              </div>
            </div>
          ) : null}
        </div>

        {rightSidebarOpen && (
          <>
            <div className={rightResize.handleClassName} onMouseDown={rightResize.handleMouseDown} />
            <div className="shrink-0 overflow-hidden border-l border-border/50 bg-sidebar" style={{ width: effectiveRightPanelWidth }}>
            <WorkspacePanel
              apiJson={apiJson}
              activeTab={workspacePanelTab}
              workspaceLoading={workspaceLoading}
              fileTree={fileTree}
              workspaceRootPath={workspaceRootPath || resolvedWorkspaceRootPath}
              workspaceRootLabel={workspaceRootPath || resolvedWorkspaceRootPath || currentSession?.directory || "project"}
              selectedWorkspacePath={selectedWorkspacePath}
              selectedWorkspaceType={selectedWorkspaceType}
              workspaceLinkLabel={workspaceLinkLabel}
              workspaceLinkStageId={workspaceLinkStageId}
              selectedFilePath={selectedFilePath}
              selectedFileContent={selectedFileContent}
              fileLoading={fileLoading}
              fileSaving={fileSaving}
              fileDeleting={fileDeleting}
              fileUploading={fileUploading}
              workspaceDirty={workspaceDirty}
              selectedWorkspaceIsRoot={selectedWorkspaceIsRoot}
              selectedWorkspaceReference={selectedWorkspaceReference}
              lastAssistant={lastAssistant}
              activeStageId={schedulerNavigation.activeStageId}
              previewStageId={schedulerNavigation.previewStageId}
              executionActivity={executionActivity}
              conversationJump={conversationJump}
              schedulerNavigation={schedulerNavigation}
              onCreateWorkspaceFile={createWorkspaceFile}
              onCreateWorkspaceDirectory={createWorkspaceDirectory}
              onUploadWorkspaceFiles={uploadWorkspaceFiles}
              onSelectWorkspaceNode={selectWorkspaceNode}
              onActiveTabChange={setWorkspacePanelTab}
              onWorkspaceContentChange={setSelectedFileContent}
              onInsertWorkspaceReference={insertWorkspaceReference}
              onAttachSelectedWorkspaceNode={attachSelectedWorkspaceNode}
              onDownloadSelectedFile={downloadSelectedFile}
              onDeleteSelectedWorkspaceNode={deleteSelectedWorkspaceNode}
              onSaveSelectedFile={saveSelectedFile}
            />
          </div>
          </>
        )}
      </div>

      {settingsOpen ? (
        <Suspense
          fallback={
            <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-start justify-end">
              <section className="h-full w-full max-w-md bg-card border-l border-border overflow-y-auto p-6 flex flex-col gap-4">
                <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground py-12">
                  <h3 className="text-sm">Loading settings...</h3>
                  <p className="text-xs">Please wait</p>
                </div>
              </section>
            </div>
          }
        >
          <SettingsDrawer
            onClose={() => setSettingsOpen(false)}
            theme={theme}
            themes={THEMES}
            onThemeChange={(nextTheme) => setTheme(nextTheme as ThemeId)}
            workspaceMode={resolvedWorkspaceMode}
            workspaceRootPath={resolvedWorkspaceRootPath}
            workspaceConfigDir={workspaceContext?.identity?.config_dir ?? null}
            selectedSessionId={selectedSessionId}
            modeOptions={settingsModeOptions}
            selectedMode={selectedMode}
            onModeChange={setSelectedMode}
            modelOptions={modelOptions}
            selectedModel={selectedModel}
            onModelChange={setSelectedModel}
            showThinking={showThinking}
            onShowThinkingChange={setShowThinking}
            providers={providers}
            knownProviders={knownProviders}
            connectProtocols={connectProtocols}
            connectQuery={connectQuery}
            onConnectQueryChange={setConnectQuery}
            connectResolution={connectResolution}
            connectResolveBusy={connectResolveBusy}
            connectResolveError={connectResolveError}
            connectProviderId={connectProviderId}
            onConnectProviderIdChange={setConnectProviderId}
            connectProtocol={connectProtocol}
            onConnectProtocolChange={setConnectProtocol}
            connectApiKey={connectApiKey}
            onConnectApiKeyChange={setConnectApiKey}
            connectBaseUrl={connectBaseUrl}
            onConnectBaseUrlChange={setConnectBaseUrl}
            connectBusy={connectBusy}
            onConnectProvider={connectProvider}
            api={api}
            apiJson={apiJson}
            onBanner={setBanner}
            onReloadCoreData={reloadCoreSettingsData}
          />
        </Suspense>
      ) : null}

      <InteractionOverlays
        question={question}
        permission={permission}
        questionAnswers={questionAnswers}
        questionSubmitting={questionSubmitting}
        permissionSubmitting={permissionSubmitting}
        onQuestionAnswerChange={(index, value) =>
          setQuestionAnswers((current) => ({ ...current, [index]: value }))
        }
        onRejectQuestion={rejectQuestion}
        onSubmitQuestion={submitQuestion}
        onReplyPermission={replyPermission}
      />
    </div>
  );
}
