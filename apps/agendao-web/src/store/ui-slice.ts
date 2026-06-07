import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

export function createUiSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "leftSidebarOpen"
  | "rightSidebarOpen"
  | "terminalOpen"
  | "settingsOpen"
  | "banner"
  | "setLeftSidebarOpen"
  | "setRightSidebarOpen"
  | "setTerminalOpen"
  | "setSettingsOpen"
  | "setBanner"
  | "clearBanner"
> {
  return {
    leftSidebarOpen: true,
    rightSidebarOpen: true,
    terminalOpen: false,
    settingsOpen: false,
    banner: null,

    setLeftSidebarOpen: (leftSidebarOpen) =>
      set({ leftSidebarOpen: resolveSetState(leftSidebarOpen, get().leftSidebarOpen) }),
    setRightSidebarOpen: (rightSidebarOpen) =>
      set({ rightSidebarOpen: resolveSetState(rightSidebarOpen, get().rightSidebarOpen) }),
    setTerminalOpen: (terminalOpen) =>
      set({ terminalOpen: resolveSetState(terminalOpen, get().terminalOpen) }),
    setSettingsOpen: (settingsOpen) =>
      set({ settingsOpen: resolveSetState(settingsOpen, get().settingsOpen) }),
    setBanner: (banner) => set({ banner: resolveSetState(banner, get().banner) }),
    clearBanner: () => set({ banner: null }),
  };
}
