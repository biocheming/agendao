import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

export function createSessionListSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "sessions"
  | "selectedSessionId"
  | "deletingSessions"
  | "setSessions"
  | "setSelectedSessionId"
  | "selectSession"
  | "setDeletingSessions"
> {
  const selectSessionId = (id: string | null) => {
    set({ selectedSessionId: id });
    get().syncRuntimeViewForSelection(id);
  };

  return {
    sessions: [],
    selectedSessionId: null,
    deletingSessions: false,

    setSessions: (sessions) => set({ sessions: resolveSetState(sessions, get().sessions) }),
    setSelectedSessionId: (selectedSessionId) =>
      selectSessionId(resolveSetState(selectedSessionId, get().selectedSessionId)),
    selectSession: (id) => selectSessionId(id),
    setDeletingSessions: (deletingSessions) =>
      set({ deletingSessions: resolveSetState(deletingSessions, get().deletingSessions) }),
  };
}
