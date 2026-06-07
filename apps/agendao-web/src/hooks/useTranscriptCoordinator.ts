import { useCallback, useEffect, useMemo, type RefObject } from "react";
import { buildWebSessionUrl, readWebSessionRoute } from "../lib/webSessionUrl";
import type { FeedMessage, MessageRecord, OutputBlock } from "../lib/history";
import {
  feedToolCallId as feedToolCallIdFromMessage,
  isToolOutputBlock,
} from "../lib/history";
import {
  questionInteractionFromInfo,
  type QuestionInfoResponseRecord,
} from "../lib/interaction";
import { runtimeSurfaceDebugDetail } from "../lib/display";
import { isOptimisticSessionId } from "../lib/session";
import { useAgendaoStore } from "../store";
import { useConversationJump } from "./useConversationJump";
import { useServerEventStream } from "./useServerEventStream";
import { useTranscriptFeedState } from "./useTranscriptFeedState";

interface UseTranscriptCoordinatorOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  applyLiveExecutionOutputBlock: (block: OutputBlock, sessionId: string) => void;
  applySchedulerStageOutputBlock: (block: OutputBlock, sessionId: string) => void;
  clearPendingSessionRefresh: () => void;
  feedRef: RefObject<HTMLDivElement | null>;
  formatError: (error: unknown) => string;
  maxPendingOutputBlocks: number;
  onConfigUpdated: () => void;
  refreshExecutionActivity: (sessionId: string) => void | Promise<void>;
  scheduleSessionRefresh: () => void;
}

export function useTranscriptCoordinator({
  apiJson,
  applyLiveExecutionOutputBlock,
  applySchedulerStageOutputBlock,
  clearPendingSessionRefresh,
  feedRef,
  formatError,
  maxPendingOutputBlocks,
  onConfigUpdated,
  refreshExecutionActivity,
  scheduleSessionRefresh,
}: UseTranscriptCoordinatorOptions) {
  const sessions = useAgendaoStore((s) => s.sessions);
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const selectedMessageIds = useAgendaoStore((s) => s.selectedMessageIds);
  const setSelectedMessageIds = useAgendaoStore((s) => s.setSelectedMessageIds);
  const setHistoryLoading = useAgendaoStore((s) => s.setHistoryLoading);
  const setQuestion = useAgendaoStore((s) => s.setQuestion);
  const setQuestionAnswers = useAgendaoStore((s) => s.setQuestionAnswers);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const sessionBreadcrumbs = useAgendaoStore((s) => s.sessionBreadcrumbs);
  const currentBreadcrumbProvenanceFor = useAgendaoStore((s) => s.currentBreadcrumbProvenanceFor);
  const streaming = useAgendaoStore((s) => s.streaming);
  const showThinking = useAgendaoStore((s) => s.showThinking);

  const {
    clearPendingOutputBlockFlush,
    clearTranscriptFeed,
    flushPendingOutputBlocks,
    liveBlocksRef,
    messageHistory,
    messages,
    optimisticMessagesRef,
    pendingOutputBlocksRef,
    queueVisibleLiveSnapshot,
    rebuildFeedFromHistory,
    setMessages,
  } = useTranscriptFeedState({
    maxPendingOutputBlocks,
    sessionIds: sessions.map((session) => session.id),
    showThinking,
  });

  const conversationJump = useConversationJump({
    messages,
    feedRef,
    onMiss: (message) => setBanner(message),
  });

  const routeHighlightIds = useMemo(() => {
    const route = readWebSessionRoute();
    return route.sessionId === selectedSessionId ? new Set(route.highlightIds) : new Set<string>();
  }, [selectedSessionId]);

  const loadPendingQuestion = useCallback(
    async (requestId: string, sessionId?: string | null) => {
      const questions = await apiJson<QuestionInfoResponseRecord[]>("/question");
      const pending = (questions ?? []).find((candidate) => candidate.id === requestId);
      if (!pending) return;
      const interaction = questionInteractionFromInfo(pending);
      if (sessionId && interaction.session_id && interaction.session_id !== sessionId) {
        return;
      }
      setQuestion(interaction);
      setQuestionAnswers({});
    },
    [apiJson, setQuestion, setQuestionAnswers],
  );

  const copyMessageLink = useCallback(
    async (message: FeedMessage) => {
      if (!selectedSessionId || !message.anchorId) return;
      const relative = buildWebSessionUrl({
        sessionId: selectedSessionId,
        messageId: message.anchorId,
        highlightIds: [],
      });
      const url = new URL(relative, window.location.origin).toString();
      await navigator.clipboard.writeText(url);
      setBanner("Copied message link");
    },
    [selectedSessionId, setBanner],
  );

  const toggleMessageSelected = useCallback(
    (message: FeedMessage) => {
      if (!message.anchorId) return;
      setSelectedMessageIds((current) => {
        const next = new Set(current);
        if (next.has(message.anchorId!)) next.delete(message.anchorId!);
        else next.add(message.anchorId!);
        return next;
      });
    },
    [setSelectedMessageIds],
  );

  const copySelectedMessageLink = useCallback(async () => {
    if (!selectedSessionId || selectedMessageIds.size === 0) return;
    const highlightIds = Array.from(selectedMessageIds);
    const relative = buildWebSessionUrl({
      sessionId: selectedSessionId,
      messageId: highlightIds[0] ?? null,
      highlightIds,
    });
    await navigator.clipboard.writeText(new URL(relative, window.location.origin).toString());
    setBanner(`Copied link for ${highlightIds.length} selected message${highlightIds.length === 1 ? "" : "s"}`);
  }, [selectedMessageIds, selectedSessionId, setBanner]);

  const copySelectedMessagesMarkdown = useCallback(async () => {
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
  }, [messages, selectedMessageIds, setBanner]);

  useEffect(() => {
    const route = readWebSessionRoute();
    const messageId = route.messageId || route.highlightIds[0] || null;
    if (!messageId || route.sessionId !== selectedSessionId) return;
    conversationJump.jumpOrQueueConversationTarget({ messageId, label: messageId });
  }, [conversationJump, messages.length, selectedSessionId]);

  useEffect(() => {
    clearTranscriptFeed();
    setSelectedMessageIds(new Set());
  }, [clearTranscriptFeed, selectedSessionId, setSelectedMessageIds]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const selectedLiveBlocks = selectedSessionId ? (liveBlocksRef.current[selectedSessionId] ?? []) : [];
    const pendingVisible = selectedSessionId ? (pendingOutputBlocksRef.current[selectedSessionId] ?? []) : [];
    (
      window as Window & {
        __agendaoWebDebug?: {
          selectedSessionId: string | null;
          showThinking: boolean;
          breadcrumbs: Array<{
            sessionId: string;
            title: string;
            viaLabel?: string | null;
            viaStageId?: string | null;
            viaToolCallId?: string | null;
          }>;
          breadcrumbProvenance: {
            sourceSessionId: string;
            sourceSessionTitle: string;
            label?: string | null;
            stageId?: string | null;
            toolCallId?: string | null;
          } | null;
          messages: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string }>;
          liveBlocks: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string; detail?: string; part_key?: string; part_kind?: string }>;
          pendingVisible: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string; detail?: string; part_key?: string; part_kind?: string }>;
          injectRuntimeSurface?: (payload: {
            banner?: string | null;
            queueItems?: Array<Record<string, unknown>>;
            sessionEvents?: Array<Record<string, unknown>>;
            inspectItems?: Array<Record<string, unknown>>;
          }) => boolean;
        };
      }
    ).__agendaoWebDebug = {
      selectedSessionId,
      showThinking,
      breadcrumbs: sessionBreadcrumbs.map((crumb) => ({
        sessionId: crumb.sessionId,
        title: crumb.title,
        viaLabel: crumb.viaLabel ?? null,
        viaStageId: crumb.viaStageId ?? null,
        viaToolCallId: crumb.viaToolCallId ?? null,
      })),
      breadcrumbProvenance: currentBreadcrumbProvenanceFor(selectedSessionId),
      messages: messages.map((message) => ({
        kind: message.kind,
        id: message.id,
        tool_call_id: feedToolCallIdFromMessage(message),
        text: message.text?.slice(0, 160),
      })),
      liveBlocks: selectedLiveBlocks.map((block) => ({
        kind: block.kind,
        id: block.id,
        tool_call_id: isToolOutputBlock(block) ? block.tool_call_id : undefined,
        text: block.text?.slice(0, 160),
        detail: runtimeSurfaceDebugDetail(block)?.slice(0, 160),
        part_key: block.live_identity?.part_key,
        part_kind: block.live_identity?.part_kind,
      })),
      pendingVisible: pendingVisible.map((block) => ({
        kind: block.kind,
        id: block.id,
        tool_call_id: isToolOutputBlock(block) ? block.tool_call_id : undefined,
        text: block.text?.slice(0, 160),
        detail: runtimeSurfaceDebugDetail(block)?.slice(0, 160),
        part_key: block.live_identity?.part_key,
        part_kind: block.live_identity?.part_kind,
      })),
      injectRuntimeSurface: ({ banner = null, queueItems = [], sessionEvents = [], inspectItems = [] }) => {
        if (!selectedSessionId) return false;
        const store = useAgendaoStore.getState();
        store.setRuntimeSurfaceBanner(selectedSessionId, banner);
        queueItems.forEach((block) =>
          store.appendRuntimeSurfaceBlock(
            selectedSessionId,
            "queueItems",
            block as never,
            20,
          ),
        );
        sessionEvents.forEach((block) =>
          store.appendRuntimeSurfaceBlock(
            selectedSessionId,
            "sessionEvents",
            block as never,
            50,
          ),
        );
        inspectItems.forEach((block) =>
          store.appendRuntimeSurfaceBlock(
            selectedSessionId,
            "inspectItems",
            block as never,
            10,
          ),
        );
        return true;
      },
    };
  }, [
    currentBreadcrumbProvenanceFor,
    liveBlocksRef,
    messages,
    pendingOutputBlocksRef,
    selectedSessionId,
    sessionBreadcrumbs,
    showThinking,
  ]);

  useEffect(() => {
    if (!selectedSessionId) {
      setBanner(null);
      return;
    }
    if (isOptimisticSessionId(selectedSessionId)) {
      setHistoryLoading(false);
      return;
    }

    let cancelled = false;

    const loadHistory = async () => {
      setHistoryLoading(true);
      try {
        const history = await apiJson<MessageRecord[]>(`/session/${selectedSessionId}/message`);
        if (cancelled) return;
        rebuildFeedFromHistory({
          history,
          sessionId: selectedSessionId,
          streaming,
        });
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
  }, [
    apiJson,
    formatError,
    rebuildFeedFromHistory,
    selectedSessionId,
    setBanner,
    setHistoryLoading,
    streaming,
  ]);

  useServerEventStream({
    applyLiveExecutionOutputBlock,
    applySchedulerStageOutputBlock,
    clearPendingOutputBlockFlush,
    clearPendingSessionRefresh,
    flushPendingOutputBlocks,
    onConfigUpdated,
    queueVisibleLiveSnapshot,
    refreshExecutionActivity,
    scheduleSessionRefresh,
  });

  return {
    conversationJump,
    copyMessageLink,
    copySelectedMessageLink,
    copySelectedMessagesMarkdown,
    loadPendingQuestion,
    messageHistory,
    messages,
    optimisticMessagesRef,
    routeHighlightIds,
    setMessages,
    toggleMessageSelected,
  };
}
