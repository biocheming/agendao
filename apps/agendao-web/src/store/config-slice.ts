import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet, type SetStateFn } from "./types";

export function createConfigSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "providers"
  | "knownProviders"
  | "connectProtocols"
  | "modes"
  | "workspaceContext"
  | "selectedModel"
  | "selectedMode"
  | "showThinking"
  | "serviceRootPath"
  | "theme"
  | "setProviders"
  | "setKnownProviders"
  | "setConnectProtocols"
  | "setModes"
  | "setWorkspaceContext"
  | "setSelectedModel"
  | "setSelectedMode"
  | "setShowThinking"
  | "setServiceRootPath"
  | "setTheme"
> {
  return {
    providers: [],
    knownProviders: [],
    connectProtocols: [],
    modes: [],
    workspaceContext: null,
    selectedModel: "",
    selectedMode: "",
    showThinking: true,
    serviceRootPath: "",
    theme: "daylight",

    setProviders: (providers: SetStateFn<AgendaoState["providers"]>) =>
      set({ providers: resolveSetState(providers, get().providers) }),
    setKnownProviders: (knownProviders: SetStateFn<AgendaoState["knownProviders"]>) =>
      set({ knownProviders: resolveSetState(knownProviders, get().knownProviders) }),
    setConnectProtocols: (connectProtocols: SetStateFn<AgendaoState["connectProtocols"]>) =>
      set({ connectProtocols: resolveSetState(connectProtocols, get().connectProtocols) }),
    setModes: (modes: SetStateFn<AgendaoState["modes"]>) =>
      set({ modes: resolveSetState(modes, get().modes) }),
    setWorkspaceContext: (workspaceContext: SetStateFn<AgendaoState["workspaceContext"]>) =>
      set({ workspaceContext: resolveSetState(workspaceContext, get().workspaceContext) }),
    setSelectedModel: (selectedModel: SetStateFn<string>) =>
      set({ selectedModel: resolveSetState(selectedModel, get().selectedModel) }),
    setSelectedMode: (selectedMode: SetStateFn<string>) =>
      set({ selectedMode: resolveSetState(selectedMode, get().selectedMode) }),
    setShowThinking: (showThinking: SetStateFn<boolean>) =>
      set({ showThinking: resolveSetState(showThinking, get().showThinking) }),
    setServiceRootPath: (serviceRootPath: SetStateFn<string>) =>
      set({ serviceRootPath: resolveSetState(serviceRootPath, get().serviceRootPath) }),
    setTheme: (theme: SetStateFn<AgendaoState["theme"]>) =>
      set({ theme: resolveSetState(theme, get().theme) }),
  };
}
