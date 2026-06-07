import { useEffect, useRef } from "react";
import { apiUrl, parseSSE } from "../lib/api";
import type { OutputBlock } from "../lib/history";
import { applyOutputBlock, shouldQueueLiveTranscriptBlock } from "../lib/liveTranscriptState";
import {
  permissionInteractionFromEvent,
  questionInteractionFromEvent,
} from "../lib/interaction";
import { useAgendaoStore } from "../store";

interface UseServerEventStreamOptions {
  applyLiveExecutionOutputBlock: (block: OutputBlock, sessionId: string) => void;
  applySchedulerStageOutputBlock: (block: OutputBlock, sessionId: string) => void;
  clearPendingOutputBlockFlush: () => void;
  clearPendingSessionRefresh: () => void;
  flushPendingOutputBlocks: () => void;
  onConfigUpdated: () => void;
  queueVisibleLiveSnapshot: (sessionId: string, block: OutputBlock) => void;
  refreshExecutionActivity: (sessionId: string) => void | Promise<void>;
  scheduleSessionRefresh: () => void;
}

function outputBlockFromEvent(event: Record<string, unknown>): OutputBlock | undefined {
  const rawBlock = event.block as OutputBlock | undefined;
  const rawLiveIdentity = event.live_identity as Record<string, unknown> | undefined;
  const liveIdentity: OutputBlock["live_identity"] = rawLiveIdentity?.message_id
    ? (rawLiveIdentity as unknown as OutputBlock["live_identity"])
    : undefined;
  if (!rawBlock) return undefined;
  return {
    ...rawBlock,
    id:
      typeof rawBlock.id === "string"
        ? rawBlock.id
        : typeof event.id === "string"
          ? event.id
          : undefined,
    live_identity: liveIdentity ?? rawBlock.live_identity,
  };
}

function eventSessionIdFromPayload(event: Record<string, unknown>): string | undefined {
  return typeof event.sessionID === "string"
    ? event.sessionID
    : typeof event.session_id === "string"
      ? event.session_id
      : undefined;
}

export function useServerEventStream({
  applyLiveExecutionOutputBlock,
  applySchedulerStageOutputBlock,
  clearPendingOutputBlockFlush,
  clearPendingSessionRefresh,
  flushPendingOutputBlocks,
  onConfigUpdated,
  queueVisibleLiveSnapshot,
  refreshExecutionActivity,
  scheduleSessionRefresh,
}: UseServerEventStreamOptions) {
  const appendRuntimeSurfaceBlock = useAgendaoStore((s) => s.appendRuntimeSurfaceBlock);
  const setMessages = useAgendaoStore((s) => s.setMessages);
  const setRuntimeSurfaceBanner = useAgendaoStore((s) => s.setRuntimeSurfaceBanner);
  const showThinking = useAgendaoStore((s) => s.showThinking);
  const showThinkingRef = useRef(showThinking);

  useEffect(() => {
    showThinkingRef.current = showThinking;
  }, [showThinking]);

  useEffect(() => {
    let active = true;
    let controller: AbortController | null = null;

    const handleServerEvent = (payload: unknown) => {
      const store = useAgendaoStore.getState();
      const event = payload as Record<string, unknown>;
      const type = typeof event.type === "string" ? event.type : "";
      const eventSessionId = eventSessionIdFromPayload(event);
      const selectedSessionId = store.selectedSessionId;

      if (type === "output_block" && eventSessionId === selectedSessionId) {
        const block = outputBlockFromEvent(event);
        if (!block) return;
        if (block.kind === "scheduler_stage") {
          applySchedulerStageOutputBlock(block, eventSessionId);
          if (shouldQueueLiveTranscriptBlock(block)) {
            queueVisibleLiveSnapshot(eventSessionId, block);
          }
          return;
        }
        if (block.kind === "tool") {
          applyLiveExecutionOutputBlock(block, eventSessionId);
        }
        if (block.kind === "session_event") {
          appendRuntimeSurfaceBlock(eventSessionId, "sessionEvents", block, 50);
          return;
        }
        if (block.kind === "status") {
          setRuntimeSurfaceBanner(eventSessionId, block.text?.trim() || null);
          return;
        }
        if (block.kind === "queue_item") {
          appendRuntimeSurfaceBlock(eventSessionId, "queueItems", block, 20);
          return;
        }
        if (block.kind === "inspect") {
          appendRuntimeSurfaceBlock(eventSessionId, "inspectItems", block, 10);
          return;
        }
        if (shouldQueueLiveTranscriptBlock(block)) {
          queueVisibleLiveSnapshot(eventSessionId, block);
        }
        return;
      }

      if (type === "error" && eventSessionId === selectedSessionId) {
        flushPendingOutputBlocks();
        store.setLatestRuntimeError(String(event.error ?? "Unknown error"));
        setMessages((current) =>
          applyOutputBlock(
            current,
            {
              kind: "status",
              tone: "error",
              text: String(event.error ?? "Unknown error"),
            },
            showThinkingRef.current,
          ),
        );
        store.setStreaming(false);
        store.setStatusLine("error");
        return;
      }

      if (type === "session.updated") {
        const source = typeof event.source === "string" ? event.source : "";
        if (source !== "topology") {
          scheduleSessionRefresh();
        }
        return;
      }

      if (type === "config.updated") {
        onConfigUpdated();
        return;
      }

      if (type === "session.status" && eventSessionId === selectedSessionId) {
        flushPendingOutputBlocks();
        const rawStatus = event.status;
        const statusCandidate =
          typeof rawStatus === "string"
            ? rawStatus
            : rawStatus && typeof rawStatus === "object" && "type" in rawStatus
              ? String((rawStatus as { type?: unknown }).type ?? "")
              : String(rawStatus ?? "");
        const status = statusCandidate === "retry" ? "retrying" : statusCandidate;
        if (status === "idle" || status === "complete" || status === "error") {
          store.setStreaming(false);
          store.setStatusLine(status || "idle");
          if (status !== "error") {
            store.setLatestRuntimeError(null);
          }
        } else if (status === "compacting" || status === "retrying") {
          store.setStreaming(true);
          store.setStatusLine(status);
          store.setLatestRuntimeError(null);
        }
        return;
      }

      if (type === "question.created" && eventSessionId === selectedSessionId) {
        flushPendingOutputBlocks();
        store.setQuestion(questionInteractionFromEvent(event, eventSessionId));
        store.setQuestionAnswers({});
        store.setStreaming(false);
        store.setStatusLine("awaiting_user");
        store.setLatestRuntimeError(null);
        return;
      }

      if (type === "question.resolved" && eventSessionId === selectedSessionId) {
        store.setQuestion(null);
        store.setQuestionAnswers({});
        store.setQuestionSubmitting(false);
        store.setLatestRuntimeError(null);
        store.setStreaming(true);
        store.setStatusLine("running");
        return;
      }

      if (type === "execution.topology.changed" && eventSessionId === selectedSessionId) {
        void refreshExecutionActivity(eventSessionId);
        return;
      }

      if (type === "permission.requested" && eventSessionId === selectedSessionId) {
        store.setPermission(permissionInteractionFromEvent(event, eventSessionId));
        store.setPermissionSubmitting(false);
        store.setPermissionSubmitError(null);
        store.setPermissionSubmitStartedAt(null);
        store.setPermissionSubmitCompletedAt(null);
        store.setLatestRuntimeError(null);
        store.setStreaming(false);
        store.setStatusLine("awaiting_user");
        return;
      }

      if (type === "permission.resolved") {
        const resolvedPermissionId = String(event.permissionID ?? "");
        let resolvedCurrentPermission = false;
        store.setPermission((current) => {
          if (!current) return null;
          if (resolvedPermissionId && current.permission_id !== resolvedPermissionId) {
            return current;
          }
          resolvedCurrentPermission = true;
          return null;
        });
        if (resolvedCurrentPermission || !resolvedPermissionId) {
          store.setPermissionSubmitting(false);
          store.setPermissionSubmitError(null);
          store.setPermissionSubmitCompletedAt(new Date().toISOString());
          store.setLatestRuntimeError(null);
          store.setStreaming(true);
          store.setStatusLine("running");
        }
      }
    };

    const connect = async () => {
      while (active) {
        controller = new AbortController();
        try {
          const response = await fetch(apiUrl("/event?tier=web"), {
            headers: { Accept: "text/event-stream" },
            signal: controller.signal,
          });
          if (!response.ok) {
            throw new Error(`${response.status} ${response.statusText}`);
          }
          await parseSSE(response, (_eventName, payload) => handleServerEvent(payload));
        } catch {
          if (!active || controller.signal.aborted) return;
          useAgendaoStore.getState().setStatusLine("reconnecting");
          await new Promise((resolve) => window.setTimeout(resolve, 1500));
        }
      }
    };

    void connect();
    return () => {
      active = false;
      controller?.abort();
      clearPendingOutputBlockFlush();
      clearPendingSessionRefresh();
    };
  }, [
    applyLiveExecutionOutputBlock,
    applySchedulerStageOutputBlock,
    clearPendingOutputBlockFlush,
    clearPendingSessionRefresh,
    flushPendingOutputBlocks,
    onConfigUpdated,
    queueVisibleLiveSnapshot,
    refreshExecutionActivity,
    scheduleSessionRefresh,
    appendRuntimeSurfaceBlock,
    setMessages,
    setRuntimeSurfaceBanner,
  ]);
}
