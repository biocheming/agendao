import { buildMultimodalHistoryBlocks } from "./multimodal";
import type { FeedMessage, MessagePartRecord, MessageRecord, OutputBlock, OutputField } from "./history";

let feedSequence = 0;

function nextFeedId() {
  feedSequence += 1;
  return `feed-${feedSequence}`;
}

export function resetLiveTranscriptFeedSequence() {
  feedSequence = 0;
}

function stableToolCallIdFromIdentity(block: OutputBlock): string | undefined {
  const wireLegacyBlockId = block.live_identity?.legacy_block_id?.trim();
  if (wireLegacyBlockId) return wireLegacyBlockId;

  const partKey = block.live_identity?.part_key?.trim();
  if (!partKey) return undefined;
  if (!(partKey.startsWith("tool_call/") || partKey.startsWith("tool_result/"))) {
    return undefined;
  }
  const slash = partKey.indexOf("/");
  if (slash < 0 || slash === partKey.length - 1) {
    return undefined;
  }
  const candidate = partKey.slice(slash + 1).trim();
  return candidate || undefined;
}

function hasLiveIdentity(block: OutputBlock): boolean {
  return Boolean(block.live_identity?.message_id?.trim());
}

function isTranscriptBearingIdentity(block: OutputBlock): boolean {
  const kind = block.live_identity?.part_kind;
  return kind === "assistant_text"
    || kind === "assistant_reasoning"
    || kind === "tool_result";
}

type LiveTranscriptRoute =
  | "compatibility"
  | "transcript"
  | "non_transcript_live";

function isAuxiliaryTranscriptExcludedBlock(block: OutputBlock): boolean {
  return block.kind === "status" || block.kind === "queue_item";
}

function shouldInsertByCompatibilityPresentation(block: OutputBlock): boolean {
  switch (block.kind) {
    case "tool":
    case "session_event":
    case "inspect":
      return true;
    default:
      return false;
  }
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
  if (liveTranscriptRoute(block) === "non_transcript_live") {
    return false;
  }
  if (block.kind === "tool" && (block.phase === "start" || block.phase === "running")) {
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

function presentationRank(block: OutputBlock): number {
  return typeof block.presentation?.rank === "number"
    ? block.presentation.rank
    : outputBlockSemanticRank(block);
}

function presentationSequence(block: OutputBlock): number {
  return typeof block.presentation?.sequence === "number" ? block.presentation.sequence : 0;
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

// Web Phase 2: per-part-key slot identity for live cache dedup.
// Visible feed uses message_id (history-compatible); live cache uses
// message_id:part_key so multiple reasoning parts within the same
// message do not collide.
function slotKey(block: OutputBlock): string | undefined {
  const messageId = block.live_identity?.message_id?.trim();
  const partKey = block.live_identity?.part_key?.trim();
  if (messageId && partKey && (block.kind === "message" || block.kind === "reasoning")) {
    return `${messageId}:${partKey}`;
  }
  return undefined;
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
  }

  const raw = typeof block.id === "string" ? block.id.trim() : "";
  if (!raw) return undefined;
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
  if (id === block.id) {
    return block;
  }
  return { ...block, id };
}

function reconcileStreamingText(authoritativeText: string, liveText: string): string {
  if (!liveText) return authoritativeText;
  if (!authoritativeText) return liveText;
  if (liveText === authoritativeText) return authoritativeText;
  if (liveText.startsWith(authoritativeText)) return liveText;
  if (authoritativeText.startsWith(liveText)) return authoritativeText;
  return authoritativeText.length >= liveText.length ? authoritativeText : liveText;
}

function hasVisibleTextPayload(block: OutputBlock): boolean {
  return normalizeBlockText(block).trim().length > 0;
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

  // Web Phase 2: streaming text blocks match by slotKey() so distinct
  // part_keys (e.g. reasoning/main vs reasoning/branch-a) get separate
  // visible feed entries instead of overwriting each other.
  const matchBySlot = isStreamingTextBlock(normalizedBlock)
    ? slotKey(normalizedBlock)
    : undefined;
  const index = messages.findIndex((message) => {
    if (message.kind !== normalizedBlock.kind) return false;
    if (matchBySlot) return slotKey(message) === matchBySlot;
    return message.id === normalizedBlock.id;
  });
  if (index < 0) {
    return insertFeedMessageByPresentation(messages, {
      ...toFeedMessage(normalizedBlock),
      ...overrides,
    });
  }

  const next = [...messages];
  next[index] = {
    ...next[index],
    ...normalizedBlock,
    ...overrides,
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
  if (normalizedBlock.kind === "tool" && (normalizedBlock.phase === "start" || normalizedBlock.phase === "running")) {
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

  if (normalizedBlock.id) {
    return upsertFeedMessage(messages, normalizedBlock, {
      text: normalizeBlockText(normalizedBlock),
    });
  }

  if (route === "transcript" && normalizedBlock.kind === "tool") {
    return messages;
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
        messages = applyOutputBlock(messages, part.output_block, showThinking);
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

export function appendLiveBlock(blocks: OutputBlock[], block: OutputBlock): OutputBlock[] {
  const normalizedBlock = normalizeOutputBlock(block);
  if (!shouldQueueLiveTranscriptBlock(normalizedBlock)) {
    return blocks;
  }
  if (!shouldRetainLiveBlock(normalizedBlock)) {
    return blocks;
  }

  const next = blocks.slice();
  // Web Phase 2: streaming text blocks (message/reasoning) dedup by
  // slotKey() = message_id:part_key, so distinct part_keys within the
  // same message do not collide. Non-streaming blocks continue to match
  // by kind + id, which is compatible with history anchors.
  const slotMatchKey = isStreamingTextBlock(normalizedBlock)
    ? slotKey(normalizedBlock)
    : undefined;
  const existingIndex = next.findIndex((candidate) => {
    if (candidate.kind !== normalizedBlock.kind) return false;
    if (slotMatchKey) return slotKey(candidate) === slotMatchKey;
    return candidate.id === normalizedBlock.id;
  });
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

    const retained = {
      ...normalizedBlock,
      text: normalizeBlockText(normalizedBlock),
    };
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
  const retained = isStreamingTextBlock(normalizedBlock)
    ? liveTextSnapshot(normalizedBlock, previous)
    : normalizedBlock;
  if (existingIndex >= 0) {
    next[existingIndex] = retained;
    return next;
  }
  next.push(retained);
  return next;
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
    ? messages.findIndex((message) => message.kind === normalizedBlock.kind && slotKey(message) === mergeSlotKey)
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
      // Web Phase 2: add slotKey from output_block for precise part-level
      // prune. If output_block is not available, only infer the default
      // main slot for persisted assistant text/reasoning. This preserves
      // backward compatibility for the common single-slot case without
      // over-pruning multi-part live branches.
      if (part.output_block) {
        const normalized = normalizeOutputBlock(part.output_block);
        if (normalized.id) {
          coveredIds.add(normalized.id);
        }
        const sk = slotKey(normalized);
        if (sk) coveredIds.add(sk);
      } else if (message.id && (part.type === "text" || part.type === "reasoning")) {
        // Backward compat: history parts without output_block only absorb
        // the canonical main slot for persisted assistant text/reasoning.
        const defaultSk = slotKey({
          kind: part.type === "text" ? "message" : "reasoning",
          live_identity: {
            message_id: message.id,
            part_key: part.type === "text" ? "text/main" : "reasoning/main",
            part_kind: part.type === "text" ? "assistant_text" : "assistant_reasoning",
            phase: "snapshot",
          },
        } as OutputBlock);
        if (defaultSk) coveredIds.add(defaultSk);
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
