import { useCallback, useEffect, useRef } from "react";
import { normalizeSessionRecord, normalizeSessionRecords } from "../lib/sidebar";
import {
  OPTIMISTIC_SESSION_ID_PREFIX,
  isOptimisticSessionId,
  type SessionListResponseRecord,
  type SessionRecord,
} from "../lib/session";
import {
  readWebSessionRoute,
  writeWebSessionRoute,
} from "../lib/webSessionUrl";
import { useAgendaoStore } from "../store";
import { useExternalAdapterProvisioning } from "./useExternalAdapterProvisioning";
import { useSessionRegistry } from "./useSessionRegistry";

interface UseSessionCoordinatorOptions {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  currentWorkspacePath: string | null;
  currentWorkspaceSummaryPath: string | null;
  formatError: (error: unknown) => string;
  selectedSessionId: string | null;
  serviceRootPath: string;
  workspaceContextRootPath: string | null;
}

export function useSessionCoordinator({
  api,
  apiJson,
  currentWorkspacePath,
  currentWorkspaceSummaryPath,
  formatError,
  selectedSessionId,
  serviceRootPath,
  workspaceContextRootPath,
}: UseSessionCoordinatorOptions) {
  const sessions = useAgendaoStore((s) => s.sessions);
  const setSessions = useAgendaoStore((s) => s.setSessions);
  const setCurrentWorkspacePath = useAgendaoStore((s) => s.setCurrentWorkspacePath);
  const setSelectedSessionId = useAgendaoStore((s) => s.setSelectedSessionId);
  const deletingSessions = useAgendaoStore((s) => s.deletingSessions);
  const setDeletingSessions = useAgendaoStore((s) => s.setDeletingSessions);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const routeInitializedRef = useRef(false);
  const routeSyncSourceRef = useRef<"app" | "browser">("app");

  const fetchSessions = useCallback(async (): Promise<SessionRecord[]> => {
    const sessionData = await apiJson<SessionListResponseRecord>("/session?limit=500");
    return normalizeSessionRecords(sessionData?.items ?? []);
  }, [apiJson]);

  const refreshSessions = useCallback(async () => {
    const sessionData = await fetchSessions();
    setSessions(sessionData);
    return sessionData;
  }, [fetchSessions, setSessions]);

  const onSessionReady = useCallback(
    (session: SessionRecord, directory: string, replace: boolean) => {
      setSessions((current) =>
        normalizeSessionRecords([session, ...current.filter((item) => item.id !== session.id)]),
      );
      setCurrentWorkspacePath((current) => directory || current);
      writeWebSessionRoute(
        { sessionId: session.id, messageId: null, highlightIds: [], externalProvisioning: null },
        { replace },
      );
    },
    [setCurrentWorkspacePath, setSessions],
  );

  const provisionExternalAdapterSession = useExternalAdapterProvisioning({
    apiJson,
    onSessionReady,
  });

  const { clearPendingSessionRefresh, scheduleSessionRefresh } = useSessionRegistry({
    fetchSessions,
    formatError,
    setBanner,
    setSelectedSessionId,
    setSessions,
  });

  const createSession = useCallback(
    async (options?: { directory?: string; title?: string; projectId?: string }) => {
      const requestedDirectory =
        options?.directory?.trim() ||
        currentWorkspaceSummaryPath ||
        currentWorkspacePath ||
        workspaceContextRootPath ||
        serviceRootPath ||
        undefined;
      const previousSelectedSessionId = selectedSessionId;
      const optimisticId =
        `${OPTIMISTIC_SESSION_ID_PREFIX}${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
      const optimisticSession = normalizeSessionRecord({
        id: optimisticId,
        title: options?.title?.trim() || "New session",
        directory: requestedDirectory,
        project_id: options?.projectId,
        updated: Date.now(),
      } as SessionRecord);
      setSessions((current) =>
        normalizeSessionRecords([
          optimisticSession,
          ...current.filter((item) => item.id !== optimisticId),
        ]),
      );
      setCurrentWorkspacePath(requestedDirectory || null);
      setSelectedSessionId(optimisticId);

      try {
      const created = await apiJson<SessionRecord>("/session", {
        method: "POST",
        body: JSON.stringify({
          directory: requestedDirectory,
          title: options?.title,
          project_id: options?.projectId,
        }),
      });
      const normalized = normalizeSessionRecord(created);
      setSessions((current) =>
        normalizeSessionRecords([
          normalized,
          ...current.filter((item) => item.id !== normalized.id && item.id !== optimisticId),
        ]),
      );
      setCurrentWorkspacePath(normalized.directory?.trim() || requestedDirectory || null);
      setSelectedSessionId(normalized.id);
      return normalized.id;
      } catch (error) {
        setSessions((current) => current.filter((item) => item.id !== optimisticId));
        setSelectedSessionId(previousSelectedSessionId ?? null);
        throw error;
      }
    },
    [
      apiJson,
      currentWorkspacePath,
      currentWorkspaceSummaryPath,
      selectedSessionId,
      serviceRootPath,
      setCurrentWorkspacePath,
      setSelectedSessionId,
      setSessions,
      workspaceContextRootPath,
    ],
  );

  const forkSelectedSession = useCallback(async () => {
    if (!selectedSessionId) return;
    try {
      const forked = normalizeSessionRecord(
        await apiJson<SessionRecord>(`/session/${selectedSessionId}/fork`, {
          method: "POST",
          body: JSON.stringify({ message_id: null }),
        }),
      );
      setSessions((current) =>
        normalizeSessionRecords([forked, ...current.filter((item) => item.id !== forked.id)]),
      );
      setCurrentWorkspacePath(forked.directory?.trim() || currentWorkspacePath || null);
      setSelectedSessionId(forked.id);
      setBanner(`Forked session ${forked.title}`);
    } catch (error) {
      setBanner(`Failed to fork session: ${formatError(error)}`);
    }
  }, [
    apiJson,
    currentWorkspacePath,
    formatError,
    selectedSessionId,
    setBanner,
    setCurrentWorkspacePath,
    setSelectedSessionId,
    setSessions,
  ]);

  const forkSessionFromMessage = useCallback(
    async (messageId: string) => {
      const targetSessionId = selectedSessionId?.trim();
      const targetMessageId = messageId.trim();
      if (!targetSessionId || !targetMessageId) {
        throw new Error("A session and message are required to fork from a prompt.");
      }

      const forked = normalizeSessionRecord(
        await apiJson<SessionRecord>(`/session/${targetSessionId}/fork`, {
          method: "POST",
          body: JSON.stringify({ message_id: targetMessageId }),
        }),
      );
      setSessions((current) =>
        normalizeSessionRecords([forked, ...current.filter((item) => item.id !== forked.id)]),
      );
      setCurrentWorkspacePath(forked.directory?.trim() || currentWorkspacePath || null);
      setSelectedSessionId(forked.id);
      return forked;
    },
    [
      apiJson,
      currentWorkspacePath,
      selectedSessionId,
      setCurrentWorkspacePath,
      setSelectedSessionId,
      setSessions,
    ],
  );

  const selectWorkspace = useCallback(
    (workspacePath: string) => {
      setCurrentWorkspacePath(workspacePath);
      const workspaceSessions = sessions
        .filter((session) => session.directory?.trim() === workspacePath)
        .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
      const preferred =
        workspaceSessions.find((session) => !session.parent_id) ?? workspaceSessions[0] ?? null;
      if (preferred) {
        setSelectedSessionId(preferred.id);
      }
    },
    [sessions, setCurrentWorkspacePath, setSelectedSessionId],
  );

  const deleteSelectedSessions = useCallback(
    async (sessionIds: string[]) => {
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

        const sessionData = await refreshSessions();

        const currentStillExists =
          selectedSessionId && sessionData.some((session) => session.id === selectedSessionId);
        if (!currentStillExists) {
          const workspacePath = currentWorkspaceSummaryPath ?? currentWorkspacePath;
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
    },
    [
      api,
      currentWorkspacePath,
      currentWorkspaceSummaryPath,
      deletingSessions,
      formatError,
      refreshSessions,
      selectedSessionId,
      sessions,
      setBanner,
      setDeletingSessions,
      setSelectedSessionId,
    ],
  );

  const selectSession = useCallback(
    (sessionId: string | null) => {
      routeSyncSourceRef.current = "app";
      setSelectedSessionId(sessionId);
    },
    [routeSyncSourceRef, setSelectedSessionId],
  );

  useEffect(() => {
    if (!selectedSessionId) return;
    if (isOptimisticSessionId(selectedSessionId)) return;
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
    if (route.sessionId === selectedSessionId && (route.messageId || route.highlightIds.length > 0)) {
      routeInitializedRef.current = true;
      return;
    }
    routeInitializedRef.current = true;
    writeWebSessionRoute({ sessionId: selectedSessionId, messageId: null, highlightIds: [] });
  }, [routeInitializedRef, routeSyncSourceRef, selectedSessionId]);

  useEffect(() => {
    let active = true;

    const handlePopState = () => {
      const route = readWebSessionRoute();
      if (!route.sessionId && route.externalProvisioning) {
        void (async () => {
          try {
            const sessionId = await provisionExternalAdapterSession(route.externalProvisioning!, {
              replace: true,
            });
            if (!active) return;
            routeSyncSourceRef.current = "browser";
            setSelectedSessionId(sessionId);
          } catch (error) {
            if (active) {
              setBanner(`Failed to provision external adapter session: ${formatError(error)}`);
            }
          }
        })();
        return;
      }
      routeSyncSourceRef.current = "browser";
      setSelectedSessionId(route.sessionId);
    };

    window.addEventListener("popstate", handlePopState);
    return () => {
      active = false;
      window.removeEventListener("popstate", handlePopState);
    };
  }, [formatError, provisionExternalAdapterSession, routeSyncSourceRef, setBanner, setSelectedSessionId]);

  return {
    clearPendingSessionRefresh,
    createSession,
    deleteSelectedSessions,
    forkSelectedSession,
    forkSessionFromMessage,
    provisionExternalAdapterSession,
    refreshSessions,
    scheduleSessionRefresh,
    selectSession,
    selectWorkspace,
  };
}
