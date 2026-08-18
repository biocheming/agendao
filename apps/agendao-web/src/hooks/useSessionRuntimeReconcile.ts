import { useCallback, useEffect } from "react";
import { permissionInteractionFromEvent } from "../lib/interaction";
import { useAgendaoStore } from "../store";

interface UseSessionRuntimeReconcileOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  loadPendingQuestion: (requestId: string, sessionId?: string | null) => Promise<void>;
}

interface RuntimeSnapshotRecord {
  run_status?: string;
  pending_question?: { request_id?: string } | null;
  pending_permission?: { permission_id?: string } | null;
}

type PermissionListItem = Record<string, unknown> & {
  id?: string;
  session_id?: string | null;
};

/**
 * SSE is a pure broadcast: events emitted while the stream is down (or while
 * another session was selected) are gone forever. `GET /session/{id}/runtime`
 * plus the pending permission/question listings are the authoritative state,
 * so every session switch, stream reconnect, and window refocus re-derives
 * the local view from them.
 */
export function useSessionRuntimeReconcile({
  apiJson,
  loadPendingQuestion,
}: UseSessionRuntimeReconcileOptions) {
  const reconcileSessionRuntime = useCallback(
    async (sessionId: string) => {
      if (!sessionId) return;
      try {
        const runtime = await apiJson<RuntimeSnapshotRecord>(
          `/session/${encodeURIComponent(sessionId)}/runtime`,
        );
        const store = useAgendaoStore.getState();
        store.applySessionRunStatus(sessionId, String(runtime.run_status ?? "idle"));

        // Task governance snapshot rides the same reconciliation so ledger
        // state converges after switches, reconnects, and tab refocus —
        // one reconcile entry, no second mechanism.
        try {
          const ledger = await apiJson<Record<string, unknown>>(
            `/session/${encodeURIComponent(sessionId)}/task-ledger`,
          );
          useAgendaoStore
            .getState()
            .setTaskLedger(sessionId, ledger as never);
        } catch {
          // Ledger unreachable; the event stream will correct the view.
        }

        const pendingPermissionId = runtime.pending_permission?.permission_id;
        if (pendingPermissionId) {
          try {
            const permissions = await apiJson<PermissionListItem[]>("/permission");
            const match = (permissions ?? []).find(
              (candidate) => candidate.id === pendingPermissionId,
            );
            useAgendaoStore.getState().setSessionPermission(
              sessionId,
              match
                ? permissionInteractionFromEvent(
                    { permissionID: pendingPermissionId, info: match },
                    sessionId,
                  )
                : null,
            );
          } catch {
            // Listing failed; keep whatever the event stream already delivered.
          }
        } else {
          useAgendaoStore.getState().setSessionPermission(sessionId, null);
        }

        const pendingQuestionId = runtime.pending_question?.request_id;
        if (pendingQuestionId) {
          await loadPendingQuestion(pendingQuestionId, sessionId);
        } else {
          useAgendaoStore.getState().setSessionQuestion(sessionId, null);
        }
      } catch {
        // Session disappeared or the request failed; the next event or
        // reconcile will correct the view.
      }
    },
    [apiJson, loadPendingQuestion],
  );

  const reconcileKnownStreams = useCallback(() => {
    const { selectedSessionId, runtimeViews } = useAgendaoStore.getState();
    const targetIds = new Set<string>();
    if (selectedSessionId) targetIds.add(selectedSessionId);
    for (const [sessionId, view] of Object.entries(runtimeViews)) {
      if (view.streaming || view.question || view.permission) {
        targetIds.add(sessionId);
      }
    }
    for (const sessionId of targetIds) {
      void reconcileSessionRuntime(sessionId);
    }
  }, [reconcileSessionRuntime]);

  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);

  // Reconcile whenever the selected session changes so the header, composer,
  // and overlays reflect that session's real runtime state.
  useEffect(() => {
    if (!selectedSessionId || selectedSessionId.startsWith("opt-")) return;
    void reconcileSessionRuntime(selectedSessionId);
  }, [reconcileSessionRuntime, selectedSessionId]);

  // Refresh on tab refocus: another client (TUI, CLI, second tab) may have
  // driven the session while this tab was hidden.
  useEffect(() => {
    const handleRefocus = () => {
      if (document.visibilityState === "visible") {
        reconcileKnownStreams();
      }
    };
    document.addEventListener("visibilitychange", handleRefocus);
    window.addEventListener("focus", handleRefocus);
    return () => {
      document.removeEventListener("visibilitychange", handleRefocus);
      window.removeEventListener("focus", handleRefocus);
    };
  }, [reconcileKnownStreams]);

  return { reconcileSessionRuntime, reconcileKnownStreams };
}
