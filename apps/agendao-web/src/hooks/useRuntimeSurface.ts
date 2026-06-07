import { useEffect, useMemo } from "react";
import { useAgendaoStore } from "../store";

interface UseRuntimeSurfaceOptions {
  selectedSessionId: string | null;
  sessionIds: string[];
}

export function useRuntimeSurface({
  selectedSessionId,
  sessionIds,
}: UseRuntimeSurfaceOptions) {
  const appendRuntimeSurfaceBlock = useAgendaoStore((s) => s.appendRuntimeSurfaceBlock);
  const setRuntimeSurfaceBanner = useAgendaoStore((s) => s.setRuntimeSurfaceBanner);
  const currentRuntimeSurfaceFor = useAgendaoStore((s) => s.currentRuntimeSurfaceFor);
  const hasRuntimeSurfaceFor = useAgendaoStore((s) => s.hasRuntimeSurfaceFor);
  const clearRuntimeSurfaceForMissingSessions = useAgendaoStore((s) => s.clearRuntimeSurfaceForMissingSessions);

  const currentRuntimeSurface = useMemo(
    () => currentRuntimeSurfaceFor(selectedSessionId),
    [currentRuntimeSurfaceFor, selectedSessionId],
  );

  const hasCurrentRuntimeSurface = hasRuntimeSurfaceFor(selectedSessionId);

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
