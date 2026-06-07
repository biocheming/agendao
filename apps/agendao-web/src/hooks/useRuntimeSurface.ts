import { useEffect, useMemo } from "react";
import { useAgendaoStore } from "../store";

export function useRuntimeSurface() {
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const sessions = useAgendaoStore((s) => s.sessions);
  const appendRuntimeSurfaceBlock = useAgendaoStore((s) => s.appendRuntimeSurfaceBlock);
  const setRuntimeSurfaceBanner = useAgendaoStore((s) => s.setRuntimeSurfaceBanner);
  const clearRuntimeSurfaceForMissingSessions = useAgendaoStore((s) => s.clearRuntimeSurfaceForMissingSessions);
  const currentRuntimeSurface = useAgendaoStore((s) =>
    s.currentRuntimeSurfaceFor(selectedSessionId),
  );
  const hasCurrentRuntimeSurface = useAgendaoStore((s) =>
    s.hasRuntimeSurfaceFor(selectedSessionId),
  );
  const sessionIds = useMemo(() => sessions.map((session) => session.id), [sessions]);

  useEffect(() => {
    clearRuntimeSurfaceForMissingSessions(sessionIds);
  }, [clearRuntimeSurfaceForMissingSessions, sessionIds]);

  return {
    appendRuntimeSurfaceBlock,
    currentRuntimeSurface,
    hasCurrentRuntimeSurface,
    setRuntimeSurfaceBanner,
  };
}
