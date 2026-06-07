import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

export function createUiSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "route"
  | "leftSidebarOpen"
  | "rightSidebarOpen"
  | "terminalOpen"
  | "banner"
  | "setRoute"
  | "setLeftSidebarOpen"
  | "setRightSidebarOpen"
  | "setTerminalOpen"
  | "setBanner"
  | "clearBanner"
> {
  return {
    route: "workbench",
    leftSidebarOpen: true,
    rightSidebarOpen: true,
    terminalOpen: false,
    banner: null,

    setRoute: (route) =>
      set({ route: resolveSetState(route, get().route) }),
    setLeftSidebarOpen: (leftSidebarOpen) =>
      set({ leftSidebarOpen: resolveSetState(leftSidebarOpen, get().leftSidebarOpen) }),
    setRightSidebarOpen: (rightSidebarOpen) =>
      set({ rightSidebarOpen: resolveSetState(rightSidebarOpen, get().rightSidebarOpen) }),
    setTerminalOpen: (terminalOpen) =>
      set({ terminalOpen: resolveSetState(terminalOpen, get().terminalOpen) }),
    setBanner: (banner) => set({ banner: resolveSetState(banner, get().banner) }),
    clearBanner: () => set({ banner: null }),
  };
}
