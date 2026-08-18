import type { AgendaoState, StoreGet, StoreSet } from "./types";

export function createTaskLedgerSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<AgendaoState, "taskLedgers" | "setTaskLedger" | "clearTaskLedger"> {
  return {
    taskLedgers: {},
    setTaskLedger: (sessionId, ledger) => {
      const next =
        typeof ledger === "function"
          ? ledger(get().taskLedgers[sessionId] ?? null)
          : ledger;
      if (!next) {
        const { [sessionId]: _removed, ...remaining } = get().taskLedgers;
        set({ taskLedgers: remaining });
        return;
      }
      const current = get().taskLedgers[sessionId];
      // Replacement events can arrive out of order across reconnects;
      // never let an older revision overwrite a newer one.
      if (current && current.revision > next.revision) {
        return;
      }
      set({
        taskLedgers: { ...get().taskLedgers, [sessionId]: next },
      });
    },
    clearTaskLedger: (sessionId) => {
      const { [sessionId]: _removed, ...remaining } = get().taskLedgers;
      set({ taskLedgers: remaining });
    },
  };
}
