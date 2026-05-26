import { buildMultimodalHistoryBlocks } from "./multimodal";
import type { FeedMessage, MessagePartRecord, MessageRecord, OutputBlock, OutputField, OutputPreview } from "./history";
import {
  ASSISTANT_REASONING_MAIN_PART_KEY,
  ASSISTANT_TEXT_MAIN_PART_KEY,
  outputBlockLiveSlotKey,
  toolIdFromPartKey,
} from "./liveIdentity";

let feedSequence = 0;

function nextFeedId() {
  feedSequence += 1;
  return `feed-${feedSequence}`;
}

export function resetLiveTranscriptFeedSequence() {
  feedSequence = 0;
}

function stableToolCallIdFromIdentity(block: OutputBlock): string | undefined {
  // Phase W2: return the raw call_id for external linking (activity panel,
  // conversation jump, highlighting). Internal live-cache dedup uses
  // toolSlotKey() which adds part_kind prefix so running and result
  // for the same tool do not collide.
  const wireLegacyBlockId = block.live_identity?.legacy_block_id?.trim();
  if (wireLegacyBlockId) return wireLegacyBlockId;
  return toolIdFromPartKey(block.live_identity?.part_key) ?? undefined;
}

function compatibilityToolCallId(block: OutputBlock): string | undefined {
  const explicit = block.tool_call_id?.trim();
  if (explicit) return explicit;
  const raw = block.id?.trim();
  return raw || undefined;
}

function toolTranscriptEntryId(block: OutputBlock): string | undefined {
  return outputBlockLiveSlotKey(block) ?? (block.id?.trim() || undefined);
}

// Phase W2: internal dedup key for tool live cache slots.
// Prefixes with part_kind so tool_call and tool_result for the same tool
// get distinct slots, while the visible transcript id remains the raw
// call_id for activity-panel / conversation-jump compatibility.
function toolSlotKey(block: OutputBlock): string | undefined {
  const entryId = toolTranscriptEntryId(block) ?? stableToolCallIdFromIdentity(block) ?? compatibilityToolCallId(block);
  if (!entryId) return undefined;
  const partKind = block.live_identity?.part_kind;
  const prefix =
    partKind === "tool_call"
      ? "running"
      : partKind === "tool_result"
        ? "done"
        : block.phase === "start" || block.phase === "running"
          ? "running"
          : "done";
  return prefix ? `${prefix}/${entryId}` : entryId;
}

function hasLiveIdentity(block: OutputBlock): boolean {
  return Boolean(block.live_identity?.message_id?.trim());
}

function isTranscriptBearingIdentity(block: OutputBlock): boolean {
  const kind = block.live_identity?.part_kind;
  return kind === "assistant_text"
    || kind === "assistant_reasoning"
    || kind === "tool_call"
    || kind === "tool_result";
}

type LiveTranscriptRoute =
  | "compatibility"
  | "transcript"
  | "non_transcript_live";

function isAuxiliaryTranscriptExcludedBlock(block: OutputBlock): boolean {
  return block.kind === "status" || block.kind === "queue_item";
}

// Phase W1: only "tool" remains on the compatibility insertion path.
// "session_event" and "inspect" must go to their dedicated surfaces (activity
// panel, debug panel), not the conversation feed. "status" is handled
// separately in applyOutputBlock.
function shouldInsertByCompatibilityPresentation(block: OutputBlock): boolean {
  return block.kind === "tool";
}

function liveTranscriptRoute(block: OutputBlock): LiveTranscriptRoute {
  if (!hasLiveIdentity(block)) {
    return "compatibility";
  }
  return isTranscriptBearingIdentity(block) ? "transcript" : "non_transcript_live";
}

export function shouldQueueLiveTranscriptBlock(block: OutputBlock): boolean {
  if (isAuxiliaryTranscriptExcludedBlock(block)) {
    return false;
  }
  if (block.kind === "session_event" || block.kind === "inspect") {
    return false;
  }
  if (liveTranscriptRoute(block) === "non_transcript_live") {
    return false;
  }
  // Scheduler stage live output already has a dedicated activity/progress
  // surface. Until it carries an explicit transcript identity, keep it out of
  // the visible transcript feed so Web does not treat progress snapshots as
  // durable conversation entries.
  if (block.kind === "scheduler_stage") {
    return false;
  }
  // Tool progress without a stable tool-call identity belongs to the
  // execution/progress surface, not the durable transcript feed.
  if (block.kind === "tool" && liveTranscriptRoute(block) === "transcript" && !normalizeStreamingBlockId(block)) {
    return false;
  }
  return true;
}

function joinedFieldText(fields?: OutputField[]): string | null {
  if (!fields?.length) return null;
  const text = fields
    .map((field) => `${field.label ?? "Field"}: ${String(field.value ?? "")}`)
    .join("\n")
    .trim();
  return text || null;
}

function prefersDisplayContractText(block: OutputBlock): boolean {
  switch (block.kind) {
    case "message":
    case "reasoning":
    case "status":
    case "queue_item":
      return false;
    default:
      return true;
  }
}

export function normalizeBlockText(block: OutputBlock): string {
  const rawText = block.text?.trim() ? block.text : null;
  const displaySummary = block.display?.summary?.trim() ? block.display.summary : null;
  const displayFields = joinedFieldText(block.display?.fields);
  const displayPreview = block.display?.preview?.text?.trim() ? block.display.preview.text : null;
  const summary = block.summary?.trim() ? block.summary : null;
  const compatibilityFields = joinedFieldText(block.fields);
  const body = block.body?.trim() ? block.body : null;
  const detail = block.detail?.trim() ? block.detail : null;
  const preview = block.preview?.trim() ? block.preview : null;

  const contractFirst = [
    displaySummary,
    summary,
    displayFields,
    compatibilityFields,
    displayPreview,
    body,
    detail,
    rawText,
    preview,
  ];
  const rawFirst = [
    rawText,
    displaySummary,
    summary,
    body,
    displayPreview,
    displayFields,
    compatibilityFields,
    detail,
    preview,
  ];

  const candidates = prefersDisplayContractText(block) ? contractFirst : rawFirst;
  return candidates.find((value): value is string => Boolean(value)) ?? "";
}

function toFeedMessage(block: OutputBlock): FeedMessage {
  // Web Phase 2: streaming text blocks anchor on slotKey() so selection,
  // copy-link, and conversation jump resolve to the specific part rather
  // than sharing message_id across all parts in the same message.
  const anchorId = isStreamingTextBlock(block)
    ? (slotKey(block) ?? block.id)
    : block.id;
  return {
    ...block,
    feedId: nextFeedId(),
    anchorId,
    text: normalizeBlockText(block),
  };
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

function insertFeedMessageByPresentation(
  messages: FeedMessage[],
  incoming: FeedMessage,
): FeedMessage[] {
  return [...messages, incoming];
}

function orderedMessageParts(parts: MessagePartRecord[] = []): MessagePartRecord[] {
  return parts;
}

function shouldRenderHistoryPart(message: MessageRecord, part: MessagePartRecord): boolean {
  if (part.ignored) {
    return false;
  }

  if (part.type === "reasoning") {
    return true;
  }

  const keepSyntheticText = message.mode === "compaction";
  if (part.type === "text" && part.synthetic && !keepSyntheticText) {
    return false;
  }

  return true;
}

function orderRelatedFeedMessages(messages: FeedMessage[]): FeedMessage[] {
  return messages;
}

export function createOptimisticUserFeedMessage(text: string): FeedMessage {
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

function historyTextBlockId(messageId: string, kind: "message" | "reasoning"): string {
  return `${messageId}:${kind}`;
}

function historyToolBlockId(messageId: string, partId: string): string {
  return `${messageId}:${partId}:tool`;
}

function historyToolPartKind(partType: string): "tool_call" | "tool_result" | undefined {
  if (partType === "tool_call") return "tool_call";
  if (partType === "tool_result") return "tool_result";
  return undefined;
}

function historyMainStreamingBlockId(block: OutputBlock): string | undefined {
  const messageId = block.live_identity?.message_id?.trim();
  const partKey = block.live_identity?.part_key?.trim();
  if (!messageId || !partKey) return undefined;
  if (block.kind === "message" && partKey === ASSISTANT_TEXT_MAIN_PART_KEY) {
    return historyTextBlockId(messageId, "message");
  }
  if (block.kind === "reasoning" && partKey === ASSISTANT_REASONING_MAIN_PART_KEY) {
    return historyTextBlockId(messageId, "reasoning");
  }
  return undefined;
}

// Web Phase 2: per-part-key slot identity for live cache dedup.
// Visible feed uses message_id (history-compatible); live cache uses
// message_id:part_key so multiple reasoning parts within the same
// message do not collide.
function slotKey(block: OutputBlock): string | undefined {
  if (block.kind !== "message" && block.kind !== "reasoning") return undefined;
  return outputBlockLiveSlotKey(block);
}

export function normalizeStreamingBlockId(block: OutputBlock): string | undefined {
  if (liveTranscriptRoute(block) === "non_transcript_live") {
    return undefined;
  }

  // Web Phase 2: visible feed identity stays at message_id level for
  // history rebuild compatibility (persisted history anchors use
  // {messageId}:message / {messageId}:reasoning). Live cache dedup
  // uses slotKey() for per-part-key isolation — see appendLiveBlock.
  const identityId = block.live_identity?.message_id?.trim();
  if (identityId && (block.kind === "message" || block.kind === "reasoning")) {
    return identityId;
  }

  if (block.kind === "tool") {
    const toolId = stableToolCallIdFromIdentity(block);
    if (toolId) return toolId;
    // Identity-bearing tool blocks must not fall back to raw event IDs.
    // Without a canonical tool_call/tool_result identity, Web would create
    // visible transcript entries that cannot be reconciled safely.
    if (hasLiveIdentity(block)) return undefined;
  }

  const raw = typeof block.id === "string" ? block.id.trim() : "";
  if (!raw) return undefined;
  if (
    liveTranscriptRoute(block) === "compatibility"
    && (block.kind === "message" || block.kind === "reasoning")
  ) {
    return raw;
  }
  if (block.kind === "message" || block.kind === "reasoning") {
    return undefined;
  }
  if (block.kind !== "message" && block.kind !== "reasoning") {
    return raw;
  }
  return undefined;
}

export function normalizeOutputBlock(block: OutputBlock): OutputBlock {
  const id = normalizeStreamingBlockId(block);
  const toolCallId =
    block.kind === "tool"
      ? (block.tool_call_id?.trim() || stableToolCallIdFromIdentity(block) || id)
      : undefined;
  const sameId = id === block.id;
  const sameToolCallId =
    block.kind !== "tool" || toolCallId === undefined || toolCallId === block.tool_call_id;
  if (sameId && sameToolCallId) {
    return block;
  }
  return {
    ...block,
    id,
    ...(block.kind === "tool" && toolCallId ? { tool_call_id: toolCallId } : {}),
  };
}

function reconcileStreamingText(authoritativeText: string, liveText: string): string {
  if (!liveText) return authoritativeText;
  if (!authoritativeText) return liveText;
  if (liveText === authoritativeText) return authoritativeText;
  if (liveText.startsWith(authoritativeText)) return liveText;
  if (authoritativeText.startsWith(liveText)) return authoritativeText;
  return authoritativeText.length >= liveText.length ? authoritativeText : liveText;
}

function reconcileBlockString(
  previousValue: string | null | undefined,
  incomingValue: string | null | undefined,
): string | undefined {
  const previousText = previousValue?.trim() ? previousValue : "";
  const incomingText = incomingValue?.trim() ? incomingValue : "";
  if (!previousText) return incomingText || undefined;
  if (!incomingText) return previousText;
  return reconcileStreamingText(previousText, incomingText);
}

function reconcileToolPreview(
  previousPreview: OutputPreview | null | undefined,
  incomingPreview: OutputPreview | null | undefined,
) {
  if (!previousPreview) return incomingPreview ?? null;
  if (!incomingPreview) return previousPreview;
  return {
    ...previousPreview,
    ...incomingPreview,
    text: reconcileBlockString(previousPreview.text, incomingPreview.text),
    kind: incomingPreview.kind ?? previousPreview.kind,
    truncated: incomingPreview.truncated ?? previousPreview.truncated,
  };
}

function toolSnapshot(block: OutputBlock, previous?: OutputBlock): OutputBlock {
  const previousDisplay = previous?.display ?? null;
  const incomingDisplay = block.display ?? null;

  return {
    ...previous,
    ...block,
    text: reconcileBlockString(previous?.text, block.text),
    summary: reconcileBlockString(previous?.summary, block.summary),
    detail: reconcileBlockString(previous?.detail, block.detail),
    preview: reconcileBlockString(previous?.preview, block.preview),
    body: reconcileBlockString(previous?.body, block.body),
    title: block.title ?? previous?.title,
    name: block.name ?? previous?.name,
    fields: block.fields?.length ? block.fields : previous?.fields,
    structured: block.structured ?? previous?.structured,
    display: previousDisplay || incomingDisplay
      ? {
          ...(previousDisplay ?? {}),
          ...(incomingDisplay ?? {}),
          header: incomingDisplay?.header ?? previousDisplay?.header,
          summary: reconcileBlockString(previousDisplay?.summary, incomingDisplay?.summary),
          fields: incomingDisplay?.fields?.length ? incomingDisplay.fields : previousDisplay?.fields,
          preview: reconcileToolPreview(previousDisplay?.preview ?? null, incomingDisplay?.preview ?? null),
        }
      : null,
  };
}

function accumulateLiveStreamingText(previousText: string, incomingText: string): string {
  if (!incomingText) return previousText;
  if (!previousText) return incomingText;
  if (incomingText === previousText) return previousText;
  if (incomingText.startsWith(previousText)) return incomingText;
  if (previousText.startsWith(incomingText)) return previousText;
  if (previousText.endsWith(incomingText)) return previousText;
  return `${previousText}${incomingText}`;
}

function hasVisibleTextPayload(block: OutputBlock): boolean {
  return normalizeBlockText(block).trim().length > 0;
}

function isStandalonePunctuationSnapshot(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  return !/[\p{L}\p{N}]/u.test(trimmed);
}

function matchesStreamingFeedMessage(candidate: FeedMessage, block: OutputBlock): boolean {
  if (!isStreamingTextBlock(block) || candidate.kind !== block.kind) {
    return false;
  }

  const candidateSlotKey = slotKey(candidate);
  const blockSlotKey = slotKey(block);
  if (candidateSlotKey || blockSlotKey) {
    if (candidateSlotKey && blockSlotKey) {
      return candidateSlotKey === blockSlotKey;
    }

    const candidateId = candidate.id?.trim();
    const historyMainId = historyMainStreamingBlockId(block);
    if (!candidateSlotKey && blockSlotKey && candidateId && historyMainId) {
      return candidateId === historyMainId;
    }

    return false;
  }

  const candidateId = candidate.id?.trim();
  const blockId = block.id?.trim();
  if (candidateId && blockId && candidateId === blockId) {
    return true;
  }

  return false;
}

function upsertFeedMessage(
  messages: FeedMessage[],
  block: OutputBlock,
  overrides: Partial<FeedMessage> = {},
): FeedMessage[] {
  const normalizedBlock = normalizeOutputBlock(block);
  const route = liveTranscriptRoute(normalizedBlock);
  if (route === "non_transcript_live") {
    return messages;
  }
  if (!normalizedBlock.id) {
    if (route === "transcript") {
      return messages;
    }
    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage(normalizedBlock),
      ...overrides,
    });
  }

  // Web Phase 2: streaming text blocks match by slotKey(). Tool blocks
  // match by toolSlotKey() so running and result for the same call_id
  // get distinct visible entries instead of overwriting each other.
  const matchBySlot = isStreamingTextBlock(normalizedBlock)
    ? slotKey(normalizedBlock)
    : normalizedBlock.kind === "tool"
      ? toolSlotKey(normalizedBlock)
      : undefined;
  const index = messages.findIndex((message) => {
    if (message.kind !== normalizedBlock.kind) return false;
    if (matchBySlot) {
      if (isStreamingTextBlock(normalizedBlock)) {
        return matchesStreamingFeedMessage(message, normalizedBlock);
      }
      if (normalizedBlock.kind === "tool") return toolSlotKey(message) === matchBySlot;
    }
    return message.id === normalizedBlock.id;
  });
  if (index < 0) {
    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage(normalizedBlock),
      ...overrides,
    });
  }

  const next = [...messages];
  const nextText = isStreamingTextBlock(normalizedBlock)
    ? reconcileStreamingText(next[index].text ?? "", normalizeBlockText(normalizedBlock))
    : (overrides.text ?? normalizeBlockText(normalizedBlock));
  if (normalizedBlock.kind === "tool") {
    const mergedToolBlock = toolSnapshot(
      {
        ...normalizedBlock,
        ...overrides,
      },
      next[index],
    );
    next[index] = {
      ...next[index],
      ...mergedToolBlock,
      text: overrides.text ?? normalizeBlockText(mergedToolBlock),
      feedId: next[index].feedId,
      anchorId: next[index].anchorId ?? mergedToolBlock.id,
    };
    return next;
  }
  next[index] = {
    ...next[index],
    ...normalizedBlock,
    ...overrides,
    text: nextText,
    feedId: next[index].feedId,
    // Web Phase 2: streaming text blocks use slotKey() for anchor so
    // selection/copy-link/jump resolve to the specific part, not just
    // the message. Multi-part reasoning within the same message gets
    // distinct anchors instead of sharing message_id.
    anchorId: isStreamingTextBlock(normalizedBlock)
      ? (next[index].anchorId ?? slotKey(normalizedBlock) ?? normalizedBlock.id)
      : (next[index].anchorId ?? normalizedBlock.id),
  };
  return next;
}

function appendStreamingDelta(
  messages: FeedMessage[],
  block: OutputBlock,
): FeedMessage[] {
  const normalizedBlock = normalizeOutputBlock(block);
  if (liveTranscriptRoute(normalizedBlock) === "non_transcript_live") {
    return messages;
  }
  const incomingText = normalizedBlock.text ?? "";
  if (normalizedBlock.id) {
    const index = messages.findIndex(
      (message) => message.kind === normalizedBlock.kind && message.id === normalizedBlock.id,
    );
    if (index >= 0) {
      const next = [...messages];
      const candidate = next[index];
      next[index] = {
        ...candidate,
        ...normalizedBlock,
        text: `${candidate.text}${incomingText}`,
        feedId: candidate.feedId,
        anchorId: candidate.anchorId ?? normalizedBlock.id,
      };
      return next;
    }

    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage({ ...normalizedBlock, text: incomingText }),
      text: incomingText,
    });
  }

  return messages;
}

export function applyOutputBlock(
  messages: FeedMessage[],
  block: OutputBlock,
  showThinking: boolean,
): FeedMessage[] {
  const normalizedBlock = normalizeOutputBlock(block);
  if (isAuxiliaryTranscriptExcludedBlock(normalizedBlock)) {
    return messages;
  }
  const route = liveTranscriptRoute(normalizedBlock);
  if (route === "non_transcript_live") {
    return messages;
  }
  const phase = normalizedBlock.phase === "snapshot" ? "full" : normalizedBlock.phase;
  if (normalizedBlock.kind === "reasoning" && !showThinking) {
    return messages;
  }
  if (normalizedBlock.kind === "status" && normalizedBlock.silent) {
    return messages;
  }

  if (normalizedBlock.kind === "message") {
    if (phase === "start") {
      return messages;
    }
    if (phase === "delta" && route === "compatibility") {
      return appendStreamingDelta(messages, normalizedBlock);
    }
    // Web Phase 2: Deltas no longer rewrite the visible feed per-token.
    // The coalescer ensures every streaming text sequence ends with a
    // full or end block carrying complete accumulated text. Delta-only
    // blocks silently update the live cache (via appendLiveBlock) but
    // do not touch the visible message feed.
    if (phase === "delta") {
      return messages;
    }
    if (phase === "end") {
      // Web Phase 1: End finalizes the block. If the end payload carries
      // accumulated text (coalescer path), upsert it. Otherwise the last
      // full already placed the content and this is a no-op.
      if (hasVisibleTextPayload(normalizedBlock)) {
        return upsertFeedMessage(messages, normalizedBlock);
      }
      return messages;
    }
    if (phase === "full") {
      if (!hasVisibleTextPayload(normalizedBlock)) {
        return messages;
      }
      return upsertFeedMessage(messages, normalizedBlock);
    }
    return messages;
  }

  if (normalizedBlock.kind === "reasoning") {
    if (phase === "start") {
      return messages;
    }
    if (phase === "delta" && route === "compatibility") {
      return appendStreamingDelta(messages, normalizedBlock);
    }
    if (phase === "delta") {
      return messages;
    }
    if (phase === "end") {
      if (hasVisibleTextPayload(normalizedBlock)) {
        return upsertFeedMessage(messages, normalizedBlock);
      }
      return messages;
    }
    if (phase === "full") {
      if (!hasVisibleTextPayload(normalizedBlock)) {
        return messages;
      }
      return upsertFeedMessage(messages, normalizedBlock);
    }
    return messages;
  }

  // Phase W1: status / session_event / inspect / queue_item must never
  // enter the conversation feed. They belong to auxiliary surfaces
  // (banner, run-tail, activity panel, debug panel).
  if (
    normalizedBlock.kind === "status" ||
    normalizedBlock.kind === "session_event" ||
    normalizedBlock.kind === "queue_item" ||
    normalizedBlock.kind === "inspect"
  ) {
    return messages;
  }

  if (normalizedBlock.kind === "tool") {
    // Identity-bearing tool blocks must carry a canonical stable tool ID
    // before they are allowed into the visible transcript feed.
    if (route === "transcript" && !normalizeStreamingBlockId(normalizedBlock)) {
      return messages;
    }
    // Tool boundary shells without any visible detail do not materialize
    // conversation entries. Only tool_call/tool_result snapshots with
    // visible summary/detail content reach the transcript feed.
    if (!hasVisibleTextPayload(normalizedBlock)) {
      return messages;
    }
    if (normalizedBlock.id) {
      return upsertFeedMessage(messages, normalizedBlock, {
        text: normalizeBlockText(normalizedBlock),
      });
    }
    if (!shouldInsertByCompatibilityPresentation(normalizedBlock)) {
      return messages;
    }
    return insertFeedMessageByPresentation(messages, toFeedMessage(normalizedBlock));
  }

  if (normalizedBlock.id) {
    return upsertFeedMessage(messages, normalizedBlock, {
      text: normalizeBlockText(normalizedBlock),
    });
  }

  if (!shouldInsertByCompatibilityPresentation(normalizedBlock)) {
    return messages;
  }

  return insertFeedMessageByPresentation(messages, toFeedMessage(normalizedBlock));
}

export function buildFeedFromHistory(history: MessageRecord[], showThinking: boolean): FeedMessage[] {
  resetLiveTranscriptFeedSequence();
  let messages: FeedMessage[] = [];

  for (const message of history || []) {
    let startedReasoning = false;
    let startedText = false;

    for (const part of orderedMessageParts(message.parts)) {
      if (!shouldRenderHistoryPart(message, part)) {
        continue;
      }
      if (part.output_block) {
        const partKind = historyToolPartKind(part.type);
        const historyOutputBlock =
          part.output_block.kind === "tool"
            ? {
                ...part.output_block,
                id: historyToolBlockId(message.id, part.id),
                metadata: partKind
                  ? {
                      ...(part.output_block.metadata ?? {}),
                      rocode_web_history_part_kind: partKind,
                    }
                  : part.output_block.metadata,
                tool_call_id:
                  part.output_block.tool_call_id?.trim()
                  || part.output_block.id?.trim()
                  || undefined,
              }
            : part.output_block;
        messages = applyOutputBlock(messages, historyOutputBlock, showThinking);
        continue;
      }

      if (part.type === "reasoning" && part.text) {
        const blockId = historyTextBlockId(message.id, "reasoning");
        if (!startedReasoning) {
          messages = applyOutputBlock(
            messages,
            {
              id: blockId,
              kind: "reasoning",
              phase: "start",
              role: message.role,
              metadata: message.metadata,
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
            // Phase 2: synthetic "full" so history rebuild produces visible
            // blocks (delta is a silent no-op in the visible feed).
            phase: "full",
            role: message.role,
            text: part.text,
          },
          showThinking,
        );
        continue;
      }

      if (part.type === "text" && part.text) {
        const blockId = historyTextBlockId(message.id, "message");
        if (!startedText) {
          messages = applyOutputBlock(
            messages,
            {
              id: blockId,
              kind: "message",
              phase: "start",
              role: message.role,
              metadata: message.metadata,
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
            phase: "full",
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
          id: historyTextBlockId(message.id, "reasoning"),
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
          id: historyTextBlockId(message.id, "message"),
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

export function estimateContextTokensFromHistory(history: MessageRecord[]): number | null {
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
  if (liveTranscriptRoute(block) === "non_transcript_live") {
    return false;
  }
  return Boolean(normalizeStreamingBlockId(block));
}

function retainedLiveMatchKey(block: OutputBlock): string | undefined {
  if (isStreamingTextBlock(block)) return slotKey(block);
  if (block.kind === "tool") return toolSlotKey(block);
  return block.id;
}

function findRetainedLiveBlockIndex(blocks: OutputBlock[], block: OutputBlock): number {
  const normalizedBlock = normalizeOutputBlock(block);
  const matchKey = retainedLiveMatchKey(normalizedBlock);
  return blocks.findIndex((candidate) => {
    if (candidate.kind !== normalizedBlock.kind) return false;
    if (!matchKey) return candidate.id === normalizedBlock.id;
    return retainedLiveMatchKey(candidate) === matchKey;
  });
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
  const currentText = normalizeBlockText(block);
  return {
    ...previous,
    ...block,
    text: accumulateLiveStreamingText(previous?.text ?? "", currentText),
  };
}

export function appendLiveBlock(blocks: OutputBlock[], block: OutputBlock): OutputBlock[] {
  const normalizedBlock = normalizeOutputBlock(block);
  if (!shouldQueueLiveTranscriptBlock(normalizedBlock)) {
    return blocks;
  }
  if (!shouldRetainLiveBlock(normalizedBlock)) {
    return blocks;
  }

  const next = blocks.slice();
  const existingIndex = findRetainedLiveBlockIndex(next, normalizedBlock);
  if (isStreamingTextBlock(normalizedBlock) && normalizedBlock.phase === "start") {
    return next;
  }
  if (normalizedBlock.phase === "end") {
    // Web Phase 1: End is finalize, not prune.
    // Streaming text blocks (message/reasoning) carry their accumulated
    // content from prior delta/full updates in the live cache. The End
    // signal means "this part is complete" — keep the accumulated text
    // and explicitly mark the retained block as settled so downstream
    // consumers can distinguish live from finalized content.
    if (isStreamingTextBlock(normalizedBlock)) {
      if (existingIndex >= 0) {
        const finalized = hasVisibleTextPayload(normalizedBlock)
          ? liveTextSnapshot(normalizedBlock, next[existingIndex])
          : { ...next[existingIndex], phase: "end" as const };
        next[existingIndex] = finalized;
      } else if (hasVisibleTextPayload(normalizedBlock)) {
        next.push({
          ...normalizedBlock,
          phase: "end",
          text: normalizeBlockText(normalizedBlock),
        });
      }
      return next;
    }

    const retained = toolSnapshot(
      {
        ...normalizedBlock,
        text: normalizeBlockText(normalizedBlock),
      },
      existingIndex >= 0 ? next[existingIndex] : undefined,
    );
    if (existingIndex >= 0) {
      next[existingIndex] = retained;
      return next;
    }
    next.push(retained);
    return next;
  }

  const previous = existingIndex >= 0 ? next[existingIndex] : undefined;
  if (isStreamingTextBlock(normalizedBlock) && !hasVisibleTextPayload(normalizedBlock)) {
    return next;
  }
  if (
    isStreamingTextBlock(normalizedBlock)
    && normalizedBlock.phase !== "end"
    && !previous?.text?.trim()
    && isStandalonePunctuationSnapshot(normalizeBlockText(normalizedBlock))
  ) {
    return next;
  }
  const retained = isStreamingTextBlock(normalizedBlock)
    ? liveTextSnapshot(normalizedBlock, previous)
    : normalizedBlock.kind === "tool"
      ? toolSnapshot(normalizedBlock, previous)
      : normalizedBlock;
  if (existingIndex >= 0) {
    next[existingIndex] = retained;
    return next;
  }
  next.push(retained);
  return next;
}

export function visibleSnapshotFromLiveBlocks(
  blocks: OutputBlock[],
  block: OutputBlock,
): OutputBlock | undefined {
  const normalizedBlock = normalizeOutputBlock(block);
  const retainedIndex = findRetainedLiveBlockIndex(blocks, normalizedBlock);
  if (retainedIndex < 0) {
    return undefined;
  }

  const retained = blocks[retainedIndex];
  if (isStreamingTextBlock(retained)) {
    const text = retained.text ?? "";
    if (!text.trim()) {
      return undefined;
    }
    return {
      ...retained,
      // Web should render the current accumulated snapshot for a live text part,
      // not the raw incoming delta/end shell.
      phase: retained.phase === "end" ? "end" : "full",
      text,
    };
  }

  return retained;
}

function mergeLiveTextBlock(messages: FeedMessage[], block: OutputBlock, showThinking: boolean): FeedMessage[] {
  const normalizedBlock = normalizeOutputBlock(block);
  if (normalizedBlock.kind === "reasoning" && !showThinking) {
    return messages;
  }
  if (liveTranscriptRoute(normalizedBlock) === "transcript" && !normalizedBlock.id) {
    return messages;
  }

  const blockText = normalizedBlock.text ?? "";
  // Web Phase 2: history rebuild matches streaming text by slotKey()
  // so multi-part reasoning (e.g. reasoning/main, reasoning/branch-a)
  // within the same message do not collide during merge.
  const mergeSlotKey = isStreamingTextBlock(normalizedBlock)
    ? slotKey(normalizedBlock)
    : undefined;
  const matchIndex = mergeSlotKey
    ? messages.findIndex((message) => matchesStreamingFeedMessage(message, normalizedBlock))
    : normalizedBlock.id
      ? messages.findIndex((message) => message.kind === normalizedBlock.kind && message.id === normalizedBlock.id)
      : -1;
  if (matchIndex >= 0) {
    const next = [...messages];
    const candidate = next[matchIndex];
    next[matchIndex] = {
      ...candidate,
      ...normalizedBlock,
      text: reconcileStreamingText(candidate.text ?? "", blockText),
      feedId: candidate.feedId,
      anchorId: isStreamingTextBlock(normalizedBlock)
        ? (candidate.anchorId ?? slotKey(normalizedBlock) ?? normalizedBlock.id)
        : (candidate.anchorId ?? normalizedBlock.id),
    };
    return next;
  }

  return insertFeedMessageByPresentation(messages, {
    ...toFeedMessage(normalizedBlock),
    text: blockText,
  });
}

export function mergeHistoryWithLiveBlocks(
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

export function pruneLiveBlocksCoveredByHistory(
  history: MessageRecord[],
  liveBlocks: OutputBlock[],
): OutputBlock[] {
  if (liveBlocks.length === 0) return liveBlocks;

  const coveredIds = new Set<string>();
  for (const message of history || []) {
    coveredIds.add(message.id);
    for (const part of orderedMessageParts(message.parts)) {
      // LTS-A2: only server-issued output_block.live_identity may define
      // slot ownership for history prune. Web must not invent part_key
      // names for persisted history parts.
      if (part.output_block) {
        const normalized = normalizeOutputBlock(part.output_block);
        if (normalized.id) {
          coveredIds.add(normalized.id);
        }
        const sk = slotKey(normalized);
        if (sk) coveredIds.add(sk);
      }
    }
  }

  return liveBlocks.filter((block) => {
    const normalized = normalizeOutputBlock(block);
    const route = liveTranscriptRoute(normalized);
    if (route !== "transcript") {
      return true;
    }
    // Web Phase 2: streaming text blocks are pruned at slotKey()
    // granularity only — the generic blockId fallback (which would
    // collapse all reasoning parts for the same message_id into one)
    // is intentionally skipped for streaming text.
    if (isStreamingTextBlock(normalized)) {
      const sk = slotKey(normalized);
      return !(sk && coveredIds.has(sk));
    }
    const blockId = normalized.id?.trim();
    if (blockId && coveredIds.has(blockId)) {
      return false;
    }
    return true;
  });
}
