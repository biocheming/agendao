import { useCallback, useEffect, useMemo, useState } from "react";
import type { AuxiliaryOutputBlock } from "../lib/history";

export type SessionRuntimeSurface = {
  banner: string | null;
  sessionEvents: AuxiliaryOutputBlock[];
  inspectItems: AuxiliaryOutputBlock[];
  queueItems: AuxiliaryOutputBlock[];
};

type RuntimeSurfaceMap = Record<string, SessionRuntimeSurface>;
type RuntimeSurfaceKey = "sessionEvents" | "inspectItems" | "queueItems";

const MAX_RUNTIME_SURFACE_SESSIONS = 12;

function createEmptyRuntimeSurface(): SessionRuntimeSurface {
  return {
    banner: null,
    sessionEvents: [],
    inspectItems: [],
    queueItems: [],
  };
}

interface UseRuntimeSurfaceOptions {
  selectedSessionId: string | null;
  sessionIds: string[];
}

export function useRuntimeSurface({
  selectedSessionId,
  sessionIds,
}: UseRuntimeSurfaceOptions) {
  const [runtimeSurfaceBySession, setRuntimeSurfaceBySession] = useState<RuntimeSurfaceMap>({});

  const updateRuntimeSurface = useCallback(
    (sessionId: string, updater: (current: SessionRuntimeSurface) => SessionRuntimeSurface) => {
      setRuntimeSurfaceBySession((prev) => {
        const current = prev[sessionId] ?? createEmptyRuntimeSurface();
        const next = updater(current);
        if (next === current) return prev;
        return { ...prev, [sessionId]: next };
      });
    },
    [],
  );

  const appendRuntimeSurfaceBlock = useCallback(
    (sessionId: string, key: RuntimeSurfaceKey, block: AuxiliaryOutputBlock, limit: number) => {
      updateRuntimeSurface(sessionId, (current) => ({
        ...current,
        [key]: [...current[key].slice(-(limit - 1)), block],
      }));
    },
    [updateRuntimeSurface],
  );

  const setRuntimeSurfaceBanner = useCallback(
    (sessionId: string, nextBanner: string | null) => {
      updateRuntimeSurface(sessionId, (current) =>
        current.banner === nextBanner ? current : { ...current, banner: nextBanner },
      );
    },
    [updateRuntimeSurface],
  );

  const currentRuntimeSurface = useMemo(
    () =>
      selectedSessionId
        ? (runtimeSurfaceBySession[selectedSessionId] ?? createEmptyRuntimeSurface())
        : createEmptyRuntimeSurface(),
    [runtimeSurfaceBySession, selectedSessionId],
  );

  const hasCurrentRuntimeSurface =
    Boolean(currentRuntimeSurface.banner) ||
    currentRuntimeSurface.sessionEvents.length > 0 ||
    currentRuntimeSurface.inspectItems.length > 0 ||
    currentRuntimeSurface.queueItems.length > 0;

  useEffect(() => {
    const validSessionIds = new Set(sessionIds);
    setRuntimeSurfaceBySession((prev) => {
      const entries = Object.entries(prev).filter(([id]) => validSessionIds.has(id));
      const next =
        entries.length > MAX_RUNTIME_SURFACE_SESSIONS
          ? entries.slice(-MAX_RUNTIME_SURFACE_SESSIONS)
          : entries;
      if (next.length === Object.keys(prev).length) return prev;
      return Object.fromEntries(next);
    });
  }, [sessionIds]);

  return {
    appendRuntimeSurfaceBlock,
    currentRuntimeSurface,
    hasCurrentRuntimeSurface,
    setRuntimeSurfaceBanner,
  };
}
