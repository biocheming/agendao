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
  return {
    sessions: [],
    selectedSessionId: null,
    deletingSessions: false,

    setSessions: (sessions) => set({ sessions: resolveSetState(sessions, get().sessions) }),
    setSelectedSessionId: (selectedSessionId) =>
      set({ selectedSessionId: resolveSetState(selectedSessionId, get().selectedSessionId) }),
    selectSession: (id) => set({ selectedSessionId: id }),
    setDeletingSessions: (deletingSessions) =>
      set({ deletingSessions: resolveSetState(deletingSessions, get().deletingSessions) }),
  };
}
