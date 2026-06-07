import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

export function createWorkspaceSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "fileTree"
  | "workspaceRootPath"
  | "workspaceLoading"
  | "workspaceNodeLoading"
  | "selectedFilePath"
  | "selectedFileContent"
  | "savedFileContent"
  | "fileLoading"
  | "fileSaving"
  | "fileDeleting"
  | "fileUploading"
  | "currentWorkspacePath"
  | "workspacePanelTab"
  | "selectedWorkspacePath"
  | "selectedWorkspaceType"
  | "pendingWorkspaceSelection"
  | "workspaceReloadToken"
  | "workspaceDirty"
  | "setFileTree"
  | "setWorkspaceRootPath"
  | "setWorkspaceLoading"
  | "setWorkspaceNodeLoading"
  | "setSelectedFilePath"
  | "setSelectedFileContent"
  | "setSavedFileContent"
  | "setFileLoading"
  | "setFileSaving"
  | "setFileDeleting"
  | "setFileUploading"
  | "setCurrentWorkspacePath"
  | "setWorkspacePanelTab"
  | "setSelectedWorkspacePath"
  | "setSelectedWorkspaceType"
  | "setPendingWorkspaceSelection"
  | "triggerWorkspaceReload"
  | "setWorkspaceDirty"
> {
  return {
    fileTree: null,
    workspaceRootPath: "",
    workspaceLoading: false,
    workspaceNodeLoading: {},
    selectedFilePath: null,
    selectedFileContent: "",
    savedFileContent: "",
    fileLoading: false,
    fileSaving: false,
    fileDeleting: false,
    fileUploading: false,
    currentWorkspacePath: null,
    workspacePanelTab: "files",
    selectedWorkspacePath: null,
    selectedWorkspaceType: "directory" as const,
    pendingWorkspaceSelection: null,
    workspaceReloadToken: 0,
    workspaceDirty: false,

    setFileTree: (fileTree) => set({ fileTree: resolveSetState(fileTree, get().fileTree) }),
    setWorkspaceRootPath: (workspaceRootPath) =>
      set({
        workspaceRootPath: resolveSetState(workspaceRootPath, get().workspaceRootPath),
      }),
    setWorkspaceLoading: (workspaceLoading) =>
      set({ workspaceLoading: resolveSetState(workspaceLoading, get().workspaceLoading) }),
    setWorkspaceNodeLoading: (path, loading) =>
      set((state) => {
        const next = { ...state.workspaceNodeLoading };
        if (loading) {
          next[path] = true;
        } else {
          delete next[path];
        }
        return { workspaceNodeLoading: next };
      }),
    setSelectedFilePath: (selectedFilePath) =>
      set({ selectedFilePath: resolveSetState(selectedFilePath, get().selectedFilePath) }),
    setSelectedFileContent: (selectedFileContent) =>
      set({
        selectedFileContent: resolveSetState(
          selectedFileContent,
          get().selectedFileContent,
        ),
      }),
    setSavedFileContent: (savedFileContent) =>
      set({ savedFileContent: resolveSetState(savedFileContent, get().savedFileContent) }),
    setFileLoading: (fileLoading) =>
      set({ fileLoading: resolveSetState(fileLoading, get().fileLoading) }),
    setFileSaving: (fileSaving) =>
      set({ fileSaving: resolveSetState(fileSaving, get().fileSaving) }),
    setFileDeleting: (fileDeleting) =>
      set({ fileDeleting: resolveSetState(fileDeleting, get().fileDeleting) }),
    setFileUploading: (fileUploading) =>
      set({ fileUploading: resolveSetState(fileUploading, get().fileUploading) }),
    setCurrentWorkspacePath: (currentWorkspacePath) =>
      set({
        currentWorkspacePath: resolveSetState(
          currentWorkspacePath,
          get().currentWorkspacePath,
        ),
      }),
    setWorkspacePanelTab: (workspacePanelTab) =>
      set({
        workspacePanelTab: resolveSetState(workspacePanelTab, get().workspacePanelTab),
      }),
    setSelectedWorkspacePath: (selectedWorkspacePath) =>
      set({
        selectedWorkspacePath: resolveSetState(
          selectedWorkspacePath,
          get().selectedWorkspacePath,
        ),
      }),
    setSelectedWorkspaceType: (selectedWorkspaceType) =>
      set({
        selectedWorkspaceType: resolveSetState(
          selectedWorkspaceType,
          get().selectedWorkspaceType,
        ),
      }),
    setPendingWorkspaceSelection: (pendingWorkspaceSelection) =>
      set({
        pendingWorkspaceSelection: resolveSetState(
          pendingWorkspaceSelection,
          get().pendingWorkspaceSelection,
        ),
      }),
    triggerWorkspaceReload: () =>
      set((state) => ({ workspaceReloadToken: state.workspaceReloadToken + 1 })),
    setWorkspaceDirty: (workspaceDirty) =>
      set({ workspaceDirty: resolveSetState(workspaceDirty, get().workspaceDirty) }),
  };
}
