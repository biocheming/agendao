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
  clearPendingOutputBlockFlush: () => void;
  clearPendingSessionRefresh: () => void;
  flushPendingOutputBlocks: () => void;
  onConfigUpdated: () => void;
  /** Called after every successful (re)connection so missed events can be reconciled. */
  onStreamReconnected: () => void;
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
  clearPendingOutputBlockFlush,
  clearPendingSessionRefresh,
  flushPendingOutputBlocks,
  onConfigUpdated,
  onStreamReconnected,
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

      // Feed/transcript blocks only render for the selected session, but
      // runtime state below is tracked for EVERY session so switching (or a
      // reconnect) never resurrects another session's stale state.
      if (type === "output_block" && eventSessionId === selectedSessionId) {
        const block = outputBlockFromEvent(event);
        if (!block) return;
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
      if (type === "session.runtime.replaced" && eventSessionId) {
        if (eventSessionId === selectedSessionId) {
          flushPendingOutputBlocks();
        }
        scheduleSessionRefresh();
        const runtime = event.runtime as Record<string, unknown> | undefined;
        const rawStatus = typeof runtime?.run_status === "string" ? runtime.run_status : "";
        if (rawStatus) {
          store.applySessionRunStatus(eventSessionId, rawStatus);
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
      if (type === "question.upsert" && eventSessionId) {
        if (eventSessionId === selectedSessionId) {
          flushPendingOutputBlocks();
          store.setQuestionAnswers({});
        }
        const question = event.question as Parameters<
          typeof questionInteractionFromInfo
        >[0];
        store.setSessionQuestion(eventSessionId, questionInteractionFromInfo(question));
        store.setQuestionSubmitting(false);
        store.applySessionRunStatus(eventSessionId, "waiting_on_user");
        return;
      }

      if (type === "question.removed" && eventSessionId) {
        store.setSessionQuestion(eventSessionId, null);
        if (eventSessionId === selectedSessionId) {
          store.setQuestionAnswers({});
          store.setQuestionSubmitting(false);
        }
        store.applySessionRunStatus(eventSessionId, "running");
        return;
      }

      // Canonical web-tier contract: the server projects PermissionRequested
      // into `permission.upsert` (payload `permission` is PermissionRequestInfo,
      // same shape as the legacy `info`), so both paths converge here.
      if (type === "permission.upsert" && eventSessionId) {
        const info = (event.permission ?? {}) as Record<string, unknown>;
        store.setSessionPermission(
          eventSessionId,
          permissionInteractionFromEvent({ permissionID: info.id, info }, eventSessionId),
        );
        store.setPermissionSubmitting(false);
        store.setPermissionSubmitError(null);
        store.setPermissionSubmitStartedAt(null);
        store.setPermissionSubmitCompletedAt(null);
        store.applySessionRunStatus(eventSessionId, "waiting_on_user");
        return;
      }

      if (type === "permission.resolved" || type === "permission.removed") {
        const resolvedPermissionId = String(event.permissionID ?? "");
        const targetSessionId = eventSessionId;
        if (!targetSessionId) return;
        let resolvedCurrentPermission = false;
        store.setSessionPermission(targetSessionId, (current) => {
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
          store.applySessionRunStatus(targetSessionId, "running");
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
          // Events emitted while disconnected are not replayed; re-derive
          // the affected sessions from the authoritative runtime state.
          onStreamReconnected();
          await parseSSE(response, (_eventName, payload) => handleServerEvent(payload));
        } catch {
          if (!active || controller.signal.aborted) return;
          const store = useAgendaoStore.getState();
          if (store.selectedSessionId) {
            store.setSessionStatusLine(store.selectedSessionId, "reconnecting");
          }
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
    clearPendingOutputBlockFlush,
    clearPendingSessionRefresh,
    flushPendingOutputBlocks,
    onConfigUpdated,
    onStreamReconnected,
    queueVisibleLiveSnapshot,
    refreshExecutionActivity,
    scheduleSessionRefresh,
    appendRuntimeSurfaceBlock,
    setRuntimeSurfaceBanner,
  ]);
}
