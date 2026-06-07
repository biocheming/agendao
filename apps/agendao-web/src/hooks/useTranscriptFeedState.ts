import {
  startTransition,
  useCallback,
  useEffect,
  useRef,
  type MutableRefObject,
} from "react";
import type {
  FeedMessage,
  MessageRecord,
  OutputBlock,
} from "../lib/history";
import {
  appendLiveBlock,
  applyOutputBlock,
  mergeHistoryWithLiveBlocks,
  pruneLiveBlocksCoveredByHistory,
  setActiveFeedSequence,
  visibleSnapshotFromLiveBlocks,
} from "../lib/liveTranscriptState";
import { useAgendaoStore } from "../store";

type SessionLiveBlockCache = Record<string, OutputBlock[]>;
type SessionOptimisticFeedCache = Record<string, FeedMessage[]>;

const MAX_SESSION_CACHE_ENTRIES = 16;

function createFeedSequence() {
  let seq = 0;
  return {
    nextId() {
      seq += 1;
      return `feed-${seq}`;
    },
    reset() {
      seq = 0;
    },
  };
}

interface UseTranscriptFeedStateOptions {
  maxPendingOutputBlocks: number;
  selectedSessionRef: MutableRefObject<string | null>;
  sessionIds: string[];
  showThinking: boolean;
}

interface RebuildFeedFromHistoryOptions {
  history: MessageRecord[];
  sessionId: string;
  streaming: boolean;
}

export function useTranscriptFeedState({
  maxPendingOutputBlocks,
  selectedSessionRef,
  sessionIds,
  showThinking,
}: UseTranscriptFeedStateOptions) {
  const messages = useAgendaoStore((s) => s.messages);
  const messageHistory = useAgendaoStore((s) => s.messageHistory);
  const setMessages = useAgendaoStore((s) => s.setMessages);
  const setMessageHistory = useAgendaoStore((s) => s.setMessageHistory);
  const liveBlocksRef = useRef<SessionLiveBlockCache>({});
  const optimisticMessagesRef = useRef<SessionOptimisticFeedCache>({});
  const pendingOutputBlocksRef = useRef<Record<string, OutputBlock[]>>({});
  const outputFlushFrameRef = useRef<number | null>(null);
  const showThinkingRef = useRef(showThinking);
  const feedSequenceRef = useRef(createFeedSequence());

  // P2-2: The hook owns the active feed sequence during its lifecycle.
  useEffect(() => {
    return setActiveFeedSequence(feedSequenceRef.current);
  }, []);

  // P2-2: Prune per-session cache entries when sessions are removed,
  // keeping at most MAX_SESSION_CACHE_ENTRIES.
  useEffect(() => {
    const validIds = new Set(sessionIds);
    const prune = (cache: Record<string, unknown>) => {
      const entries = Object.entries(cache).filter(([id]) => validIds.has(id));
      if (entries.length > MAX_SESSION_CACHE_ENTRIES) {
        return Object.fromEntries(entries.slice(-MAX_SESSION_CACHE_ENTRIES));
      }
      if (entries.length < Object.keys(cache).length) {
        return Object.fromEntries(entries);
      }
      return cache;
    };
    liveBlocksRef.current = prune(liveBlocksRef.current) as SessionLiveBlockCache;
    optimisticMessagesRef.current = prune(optimisticMessagesRef.current) as SessionOptimisticFeedCache;
    pendingOutputBlocksRef.current = prune(pendingOutputBlocksRef.current) as Record<string, OutputBlock[]>;
  }, [sessionIds]);

  useEffect(() => {
    showThinkingRef.current = showThinking;
  }, [showThinking]);

  const clearPendingOutputBlockFlush = useCallback(() => {
    if (outputFlushFrameRef.current !== null) {
      window.cancelAnimationFrame(outputFlushFrameRef.current);
      outputFlushFrameRef.current = null;
    }
  }, []);

  const pendingVisibleSnapshotKey = useCallback((block: OutputBlock): string => {
    const messageId = block.live_identity?.message_id?.trim() || "";
    const partKey = block.live_identity?.part_key?.trim() || "";
    if (block.kind === "message" || block.kind === "reasoning") {
      return `${block.kind}:${messageId}:${partKey || block.id || ""}`;
    }
    if (block.kind === "tool") {
      const toolId = block.tool_call_id?.trim() || block.id?.trim() || "";
      return `${block.kind}:${messageId}:${partKey || toolId}:${block.live_identity?.part_kind || block.phase || ""}`;
    }
    return `${block.kind}:${block.id?.trim() || messageId || block.phase || ""}`;
  }, []);

  const flushPendingOutputBlocks = useCallback(() => {
    clearPendingOutputBlockFlush();

    const queuedBySession = pendingOutputBlocksRef.current;
    const sessionIds = Object.keys(queuedBySession);
    if (sessionIds.length === 0) {
      return;
    }
    pendingOutputBlocksRef.current = {};
    const activeSessionId = selectedSessionRef.current;
    const visibleSnapshots = activeSessionId ? (queuedBySession[activeSessionId] ?? []) : [];

    if (visibleSnapshots.length === 0) {
      return;
    }

    startTransition(() => {
      setMessages((current) =>
        visibleSnapshots.reduce(
          (nextMessages, block) => applyOutputBlock(nextMessages, block, showThinkingRef.current),
          current,
        ),
      );
    });
  }, [clearPendingOutputBlockFlush, selectedSessionRef, setMessages]);

  const schedulePendingOutputBlockFlush = useCallback(() => {
    if (outputFlushFrameRef.current !== null) {
      return;
    }
    outputFlushFrameRef.current = window.requestAnimationFrame(() => {
      flushPendingOutputBlocks();
    });
  }, [flushPendingOutputBlocks]);

  const materializePendingOutputBlocksForSession = useCallback((sessionId: string): OutputBlock[] => {
    const nextQueued = { ...pendingOutputBlocksRef.current };
    if (sessionId in nextQueued) {
      delete nextQueued[sessionId];
      pendingOutputBlocksRef.current = nextQueued;
      if (Object.keys(nextQueued).length === 0) {
        clearPendingOutputBlockFlush();
      }
    }
    return liveBlocksRef.current[sessionId] ?? [];
  }, [clearPendingOutputBlockFlush]);

  const queueVisibleLiveSnapshot = useCallback((sessionId: string, block: OutputBlock) => {
    const currentLiveBlocks = liveBlocksRef.current[sessionId] ?? [];
    const nextLiveBlocks = appendLiveBlock(currentLiveBlocks, block);
    liveBlocksRef.current = {
      ...liveBlocksRef.current,
      [sessionId]: nextLiveBlocks,
    };
    if (sessionId !== selectedSessionRef.current) {
      return;
    }
    const visible = visibleSnapshotFromLiveBlocks(nextLiveBlocks, block);
    if (!visible) {
      return;
    }
    const queue = pendingOutputBlocksRef.current[sessionId] ?? [];
    const queueKey = pendingVisibleSnapshotKey(visible);
    const existingIndex = queue.findIndex((candidate) => pendingVisibleSnapshotKey(candidate) === queueKey);
    if (existingIndex >= 0) {
      queue[existingIndex] = visible;
    } else {
      queue.push(visible);
    }
    while (queue.length > maxPendingOutputBlocks) queue.shift();
    pendingOutputBlocksRef.current[sessionId] = queue;
    schedulePendingOutputBlockFlush();
  }, [
    maxPendingOutputBlocks,
    pendingVisibleSnapshotKey,
    schedulePendingOutputBlockFlush,
    selectedSessionRef,
  ]);

  const rebuildFeedFromHistory = useCallback(({
    history,
    sessionId,
    streaming,
  }: RebuildFeedFromHistoryOptions) => {
    setMessageHistory(history);
    const currentLiveBlocks = materializePendingOutputBlocksForSession(sessionId);
    const shouldPruneFromHistory = !streaming;
    const prunedLiveBlocks = shouldPruneFromHistory
      ? pruneLiveBlocksCoveredByHistory(history, currentLiveBlocks)
      : currentLiveBlocks;
    liveBlocksRef.current = {
      ...liveBlocksRef.current,
      [sessionId]: prunedLiveBlocks,
    };
    const mergedHistory = mergeHistoryWithLiveBlocks(
      history,
      prunedLiveBlocks,
      showThinkingRef.current,
    );
    const optimisticMessages = optimisticMessagesRef.current[sessionId] ?? [];
    const merged = mergeOptimisticMessages(mergedHistory, optimisticMessages);
    optimisticMessagesRef.current = {
      ...optimisticMessagesRef.current,
      [sessionId]: merged.remaining,
    };
    setMessages(merged.messages);
  }, [materializePendingOutputBlocksForSession, setMessageHistory, setMessages]);

  const clearTranscriptFeed = useCallback(() => {
    setMessages([]);
    setMessageHistory([]);
  }, [setMessageHistory, setMessages]);

  return {
    clearPendingOutputBlockFlush,
    clearTranscriptFeed,
    flushPendingOutputBlocks,
    liveBlocksRef,
    materializePendingOutputBlocksForSession,
    messageHistory,
    messages,
    optimisticMessagesRef,
    pendingOutputBlocksRef,
    pendingVisibleSnapshotKey,
    queueVisibleLiveSnapshot,
    rebuildFeedFromHistory,
    setMessageHistory,
    setMessages,
  };
}

function mergeOptimisticMessages(
  feed: FeedMessage[],
  optimisticMessages: FeedMessage[],
): { messages: FeedMessage[]; remaining: FeedMessage[] } {
  if (optimisticMessages.length === 0) {
    return { messages: feed, remaining: [] };
  }

  const acknowledged = new Set(
    feed
      .filter((message) => message.kind === "message")
      .map((message) => message.text.trim())
      .filter(Boolean),
  );

  const remaining = optimisticMessages.filter((message) => {
    const text = message.text.trim();
    return !text || !acknowledged.has(text);
  });

  return {
    messages: remaining.length > 0 ? [...feed, ...remaining] : feed,
    remaining,
  };
}
