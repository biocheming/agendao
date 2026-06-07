import { useCallback, useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import type { SessionRecord } from "../lib/session";

interface UseSessionRegistryOptions {
  fetchSessions: () => Promise<SessionRecord[]>;
  formatError: (error: unknown) => string;
  setBanner: Dispatch<SetStateAction<string | null>>;
  setSelectedSessionId: Dispatch<SetStateAction<string | null>>;
  setSessions: Dispatch<SetStateAction<SessionRecord[]>>;
}

export function useSessionRegistry({
  fetchSessions,
  formatError,
  setBanner,
  setSelectedSessionId,
  setSessions,
}: UseSessionRegistryOptions) {
  const pendingSessionRefreshTimerRef = useRef<number | null>(null);

  const clearPendingSessionRefresh = useCallback(() => {
    if (pendingSessionRefreshTimerRef.current !== null) {
      window.clearTimeout(pendingSessionRefreshTimerRef.current);
      pendingSessionRefreshTimerRef.current = null;
    }
  }, []);

  const refreshSessionsFromServer = useCallback(async () => {
    try {
      const sessionData = await fetchSessions();
      setSessions(sessionData);
      setSelectedSessionId((current) => {
        if (current && sessionData.some((session) => session.id === current)) {
          return current;
        }
        return sessionData[0]?.id ?? null;
      });
    } catch (error) {
      setBanner(`Failed to refresh sessions: ${formatError(error)}`);
    }
  }, [fetchSessions, formatError, setBanner, setSelectedSessionId, setSessions]);

  const scheduleSessionRefresh = useCallback(() => {
    if (pendingSessionRefreshTimerRef.current !== null) {
      return;
    }
    pendingSessionRefreshTimerRef.current = window.setTimeout(() => {
      pendingSessionRefreshTimerRef.current = null;
      void refreshSessionsFromServer();
    }, 120);
  }, [refreshSessionsFromServer]);

  useEffect(() => clearPendingSessionRefresh, [clearPendingSessionRefresh]);

  return {
    clearPendingSessionRefresh,
    scheduleSessionRefresh,
  };
}
