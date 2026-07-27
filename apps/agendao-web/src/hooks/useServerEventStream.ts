import { useEffect } from "react";
import { apiUrl, parseSSE, serverPasswordAuthHeaders } from "../lib/api";
import type { OutputBlock } from "../lib/history";
import { shouldQueueLiveTranscriptBlock } from "../lib/liveTranscriptState";
import {
  permissionInteractionFromEvent,
  questionInteractionFromInfo,
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
  const setRuntimeSurfaceBanner = useAgendaoStore((s) => s.setRuntimeSurfaceBanner);

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

      if (type === "config.updated") {
        onConfigUpdated();
        return;
      }

      // Canonical web-tier status event: the server projects every
      // ServerEvent::SessionStatus into `session.runtime.replaced` on the
      // frontend bus (see crates/agendao-server/src/session_runtime/
      // frontend_projection.rs). It is also the sidebar-refresh trigger —
      // the legacy `session.updated` vocabulary never reaches this channel.
      if (type === "session.runtime.replaced" && eventSessionId === selectedSessionId) {
        flushPendingOutputBlocks();
        scheduleSessionRefresh();
        const runtime = event.runtime as Record<string, unknown> | undefined;
        const rawStatus = typeof runtime?.run_status === "string" ? runtime.run_status : "";
        if (rawStatus === "idle") {
          store.setStreaming(false);
          store.setStatusLine("idle");
          store.setLatestRuntimeError(null);
        } else if (rawStatus === "waiting_on_user") {
          store.setStreaming(false);
          store.setStatusLine("awaiting_user");
          store.setLatestRuntimeError(null);
        } else if (rawStatus === "running" || rawStatus === "waiting_on_tool") {
          store.setStreaming(true);
          store.setStatusLine("running");
          store.setLatestRuntimeError(null);
        } else if (rawStatus === "compacting") {
          store.setStreaming(true);
          store.setStatusLine("compacting");
          store.setLatestRuntimeError(null);
        }
        return;
      }

      // Topology/stage snapshots replace the legacy `execution.topology.changed`
      // vocabulary; refresh the activity panel when a topology is present.
      if (type === "session.projection.replaced" && eventSessionId === selectedSessionId) {
        if (event.topology) {
          void refreshExecutionActivity(eventSessionId);
        }
        return;
      }

      // Canonical web-tier question events (payload `question` is QuestionInfo).
      if (type === "question.upsert" && eventSessionId === selectedSessionId) {
        flushPendingOutputBlocks();
        const question = event.question as Parameters<typeof questionInteractionFromInfo>[0];
        store.setQuestion(questionInteractionFromInfo(question));
        store.setQuestionAnswers({});
        store.setQuestionSubmitting(false);
        store.setStreaming(false);
        store.setStatusLine("awaiting_user");
        store.setLatestRuntimeError(null);
        return;
      }

      if (type === "question.removed" && eventSessionId === selectedSessionId) {
        store.setQuestion(null);
        store.setQuestionAnswers({});
        store.setQuestionSubmitting(false);
        store.setLatestRuntimeError(null);
        store.setStreaming(true);
        store.setStatusLine("running");
        return;
      }

      // Canonical web-tier contract: the server projects PermissionRequested
      // into `permission.upsert` (payload `permission` is PermissionRequestInfo,
      // same shape as the legacy `info`), so both paths converge here.
      if (type === "permission.upsert" && eventSessionId === selectedSessionId) {
        const info = (event.permission ?? {}) as Record<string, unknown>;
        store.setPermission(
          permissionInteractionFromEvent({ permissionID: info.id, info }, eventSessionId),
        );
        store.setPermissionSubmitting(false);
        store.setPermissionSubmitError(null);
        store.setPermissionSubmitStartedAt(null);
        store.setPermissionSubmitCompletedAt(null);
        store.setLatestRuntimeError(null);
        store.setStreaming(false);
        store.setStatusLine("awaiting_user");
        return;
      }

      if (type === "permission.resolved" || type === "permission.removed") {
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
            headers: { Accept: "text/event-stream", ...serverPasswordAuthHeaders() },
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
    setRuntimeSurfaceBanner,
  ]);
}
