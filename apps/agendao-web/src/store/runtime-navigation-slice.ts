import type { BreadcrumbProvenance } from "../hooks/useSchedulerNavigation";
import { resolveSetState, type AgendaoState, type SessionRuntimeSurface, type StoreGet, type StoreSet } from "./types";

const MAX_RUNTIME_SURFACE_SESSIONS = 12;

const EMPTY_RUNTIME_SURFACE: SessionRuntimeSurface = {
  banner: null,
  sessionEvents: [],
  inspectItems: [],
  queueItems: [],
};

function createEmptyRuntimeSurface(): SessionRuntimeSurface {
  return {
    ...EMPTY_RUNTIME_SURFACE,
    sessionEvents: [],
    inspectItems: [],
    queueItems: [],
  };
}

export function createRuntimeNavigationSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "runtimeSurfaceBySession"
  | "activeStageContext"
  | "previewStageId"
  | "sessionBreadcrumbs"
  | "setRuntimeSurfaceBySession"
  | "setActiveStageContext"
  | "setPreviewStageId"
  | "setSessionBreadcrumbs"
  | "appendRuntimeSurfaceBlock"
  | "setRuntimeSurfaceBanner"
  | "clearRuntimeSurfaceForMissingSessions"
  | "currentRuntimeSurfaceFor"
  | "hasRuntimeSurfaceFor"
  | "currentBreadcrumbProvenanceFor"
> {
  return {
    runtimeSurfaceBySession: {},
    activeStageContext: null,
    previewStageId: null,
    sessionBreadcrumbs: [],

    setRuntimeSurfaceBySession: (runtimeSurfaceBySession) =>
      set({
        runtimeSurfaceBySession: resolveSetState(
          runtimeSurfaceBySession,
          get().runtimeSurfaceBySession,
        ),
      }),
    setActiveStageContext: (activeStageContext) =>
      set({
        activeStageContext: resolveSetState(activeStageContext, get().activeStageContext),
      }),
    setPreviewStageId: (previewStageId) =>
      set({ previewStageId: resolveSetState(previewStageId, get().previewStageId) }),
    setSessionBreadcrumbs: (sessionBreadcrumbs) =>
      set({
        sessionBreadcrumbs: resolveSetState(sessionBreadcrumbs, get().sessionBreadcrumbs),
      }),

    appendRuntimeSurfaceBlock: (sessionId, key, block, limit) =>
      set((state) => {
        const current = state.runtimeSurfaceBySession[sessionId] ?? createEmptyRuntimeSurface();
        const next = {
          ...current,
          [key]: [...current[key].slice(-(limit - 1)), block],
        };
        return {
          runtimeSurfaceBySession: {
            ...state.runtimeSurfaceBySession,
            [sessionId]: next,
          },
        };
      }),

    setRuntimeSurfaceBanner: (sessionId, banner) =>
      set((state) => {
        const current = state.runtimeSurfaceBySession[sessionId] ?? createEmptyRuntimeSurface();
        if (current.banner === banner) return {};
        return {
          runtimeSurfaceBySession: {
            ...state.runtimeSurfaceBySession,
            [sessionId]: { ...current, banner },
          },
        };
      }),

    clearRuntimeSurfaceForMissingSessions: (sessionIds) =>
      set((state) => {
        const validSessionIds = new Set(sessionIds);
        const entries = Object.entries(state.runtimeSurfaceBySession).filter(([id]) =>
          validSessionIds.has(id),
        );
        const next =
          entries.length > MAX_RUNTIME_SURFACE_SESSIONS
            ? entries.slice(-MAX_RUNTIME_SURFACE_SESSIONS)
            : entries;
        const currentEntries = Object.entries(state.runtimeSurfaceBySession);
        const unchanged =
          next.length === currentEntries.length &&
          next.every(([id, value], index) => {
            const [currentId, currentValue] = currentEntries[index] ?? [];
            return currentId === id && currentValue === value;
          });
        if (unchanged) {
          return {};
        }
        return {
          runtimeSurfaceBySession: Object.fromEntries(next),
        };
      }),

    currentRuntimeSurfaceFor: (sessionId) =>
      sessionId ? (get().runtimeSurfaceBySession[sessionId] ?? EMPTY_RUNTIME_SURFACE) : EMPTY_RUNTIME_SURFACE,

    hasRuntimeSurfaceFor: (sessionId) => {
      const current = get().currentRuntimeSurfaceFor(sessionId);
      return (
        Boolean(current.banner) ||
        current.sessionEvents.length > 0 ||
        current.inspectItems.length > 0 ||
        current.queueItems.length > 0
      );
    },

    currentBreadcrumbProvenanceFor: (selectedSessionId) => {
      const breadcrumbs = get().sessionBreadcrumbs;
      if (!selectedSessionId || breadcrumbs.length < 2) return null;
      const selectedIndex = breadcrumbs.findIndex((crumb) => crumb.sessionId === selectedSessionId);
      if (selectedIndex <= 0) return null;
      const sourceCrumb = breadcrumbs[selectedIndex - 1];
      return {
        sourceSessionId: sourceCrumb.sessionId,
        sourceSessionTitle: sourceCrumb.title,
        label: sourceCrumb.viaLabel ?? null,
        stageId: sourceCrumb.viaStageId ?? null,
        toolCallId: sourceCrumb.viaToolCallId ?? null,
      } satisfies BreadcrumbProvenance;
    },
  };
}
