import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

export function createTranscriptFeedSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "messages"
  | "messageHistory"
  | "selectedMessageIds"
  | "historyLoading"
  | "setMessages"
  | "setMessageHistory"
  | "setSelectedMessageIds"
  | "clearTranscriptFeed"
  | "toggleMessageSelected"
  | "clearSelectedMessages"
  | "setHistoryLoading"
> {
  return {
    messages: [],
    messageHistory: [],
    selectedMessageIds: new Set<string>(),
    historyLoading: false,

    setMessages: (messages) => set({ messages: resolveSetState(messages, get().messages) }),
    setMessageHistory: (messageHistory) =>
      set({ messageHistory: resolveSetState(messageHistory, get().messageHistory) }),
    setSelectedMessageIds: (selectedMessageIds) =>
      set({
        selectedMessageIds: resolveSetState(selectedMessageIds, get().selectedMessageIds),
      }),
    clearTranscriptFeed: () =>
      set({ messages: [], messageHistory: [], selectedMessageIds: new Set() }),
    toggleMessageSelected: (message) =>
      set((state) => {
        if (!message.anchorId) return {};
        const next = new Set(state.selectedMessageIds);
        if (next.has(message.anchorId)) next.delete(message.anchorId);
        else next.add(message.anchorId);
        return { selectedMessageIds: next };
      }),
    clearSelectedMessages: () => set({ selectedMessageIds: new Set() }),
    setHistoryLoading: (historyLoading) =>
      set({ historyLoading: resolveSetState(historyLoading, get().historyLoading) }),
  };
}
