import { resolveSetState, type AgendaoState, type PromptPart, type StoreGet, type StoreSet } from "./types";

export function createComposerSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "composer"
  | "attachments"
  | "selectedAttachmentIndex"
  | "composerDragActive"
  | "setComposer"
  | "setAttachments"
  | "removeAttachmentAt"
  | "selectAttachment"
  | "setComposerDragActive"
  | "clearComposer"
> {
  return {
    composer: "",
    attachments: [] as PromptPart[],
    selectedAttachmentIndex: null,
    composerDragActive: false,

    setComposer: (composer) => set({ composer: resolveSetState(composer, get().composer) }),
    setAttachments: (attachments) =>
      set({ attachments: resolveSetState(attachments, get().attachments) }),
    removeAttachmentAt: (index) =>
      set((state) => {
        const next = state.attachments.filter((_, i) => i !== index);
        let nextIndex = state.selectedAttachmentIndex;
        if (nextIndex === index) nextIndex = null;
        else if (nextIndex !== null && nextIndex > index) nextIndex -= 1;
        return { attachments: next, selectedAttachmentIndex: nextIndex };
      }),
    selectAttachment: (index) =>
      set({
        selectedAttachmentIndex: resolveSetState(index, get().selectedAttachmentIndex),
      }),
    setComposerDragActive: (composerDragActive) =>
      set({
        composerDragActive: resolveSetState(composerDragActive, get().composerDragActive),
      }),
    clearComposer: () =>
      set({ composer: "", attachments: [], selectedAttachmentIndex: null }),
  };
}
