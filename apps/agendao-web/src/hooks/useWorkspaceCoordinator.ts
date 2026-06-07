import { type ChangeEvent, useCallback, useEffect, useMemo, useRef } from "react";
import { apiUrl } from "../lib/api";
import { previewPathFromMessageMetadata } from "../lib/display";
import type { MessageRecord } from "../lib/history";
import type { PromptPart } from "../lib/display";
import {
  attachmentLabel,
  attachmentWorkspacePath,
  appendReferenceToken,
  fileUrlFromPath,
  findNodeByPath,
  guessWorkspaceMime,
  parentDirectory,
  resolveWorkspacePath,
  toWorkspaceReferencePath,
} from "../lib/composerContext";
import type {
  DirectoryCreateResponseRecord,
  FileContentResponseRecord,
  FileTreeNodeRecord,
  UploadFilesResponseRecord,
  WorkspaceContextRecord,
} from "../lib/workspace";
import { workspaceRootFromContext } from "../lib/workspace";
import { isOptimisticSessionId } from "../lib/session";
import { useAgendaoStore } from "../store";

interface UseWorkspaceCoordinatorOptions {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  currentSessionDirectory: string | null | undefined;
  currentWorkspaceSummaryPath: string | null;
  formatError: (error: unknown) => string;
  messageHistory: MessageRecord[];
  selectedSessionId: string | null;
  serviceRootPath: string;
  workspaceContext: WorkspaceContextRecord | null;
}

export function useWorkspaceCoordinator({
  api,
  apiJson,
  currentSessionDirectory,
  currentWorkspaceSummaryPath,
  formatError,
  messageHistory,
  selectedSessionId,
  serviceRootPath,
  workspaceContext,
}: UseWorkspaceCoordinatorOptions) {
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const setComposer = useAgendaoStore((s) => s.setComposer);
  const setAttachments = useAgendaoStore((s) => s.setAttachments);
  const setSelectedAttachmentIndex = useAgendaoStore((s) => s.selectAttachment);
  const fileTree = useAgendaoStore((s) => s.fileTree);
  const setFileTree = useAgendaoStore((s) => s.setFileTree);
  const workspaceRootPath = useAgendaoStore((s) => s.workspaceRootPath);
  const setWorkspaceRootPath = useAgendaoStore((s) => s.setWorkspaceRootPath);
  const setWorkspaceLoading = useAgendaoStore((s) => s.setWorkspaceLoading);
  const setWorkspaceNodeLoading = useAgendaoStore((s) => s.setWorkspaceNodeLoading);
  const selectedFilePath = useAgendaoStore((s) => s.selectedFilePath);
  const selectedFileContent = useAgendaoStore((s) => s.selectedFileContent);
  const setSelectedFileContent = useAgendaoStore((s) => s.setSelectedFileContent);
  const savedFileContent = useAgendaoStore((s) => s.savedFileContent);
  const setSavedFileContent = useAgendaoStore((s) => s.setSavedFileContent);
  const setFileLoading = useAgendaoStore((s) => s.setFileLoading);
  const fileSaving = useAgendaoStore((s) => s.fileSaving);
  const setFileSaving = useAgendaoStore((s) => s.setFileSaving);
  const fileDeleting = useAgendaoStore((s) => s.fileDeleting);
  const setFileDeleting = useAgendaoStore((s) => s.setFileDeleting);
  const fileUploading = useAgendaoStore((s) => s.fileUploading);
  const setFileUploading = useAgendaoStore((s) => s.setFileUploading);
  const setWorkspacePanelTab = useAgendaoStore((s) => s.setWorkspacePanelTab);
  const selectedWorkspacePath = useAgendaoStore((s) => s.selectedWorkspacePath);
  const setSelectedWorkspacePath = useAgendaoStore((s) => s.setSelectedWorkspacePath);
  const selectedWorkspaceType = useAgendaoStore((s) => s.selectedWorkspaceType);
  const setSelectedWorkspaceType = useAgendaoStore((s) => s.setSelectedWorkspaceType);
  const pendingWorkspaceSelection = useAgendaoStore((s) => s.pendingWorkspaceSelection);
  const setPendingWorkspaceSelection = useAgendaoStore((s) => s.setPendingWorkspaceSelection);
  const workspaceReloadToken = useAgendaoStore((s) => s.workspaceReloadToken);
  const triggerWorkspaceReload = useAgendaoStore((s) => s.triggerWorkspaceReload);
  const setSelectedFilePath = useAgendaoStore((s) => s.setSelectedFilePath);
  const setWorkspaceDirty = useAgendaoStore((s) => s.setWorkspaceDirty);
  const autoPreviewSignatureRef = useRef("");

  const mergeTreeNode = useCallback(
    (
      tree: FileTreeNodeRecord | null,
      path: string,
      updater: (node: FileTreeNodeRecord) => FileTreeNodeRecord,
    ): FileTreeNodeRecord | null => {
      if (!tree) return null;
      if (tree.path === path) {
        return updater(tree);
      }
      const children = tree.children ?? [];
      let changed = false;
      const nextChildren = children.map((child) => {
        const nextChild = mergeTreeNode(child, path, updater);
        if (nextChild !== child) changed = true;
        return nextChild ?? child;
      });
      return changed ? { ...tree, children: nextChildren } : tree;
    },
    [],
  );

  const ensureWorkspaceNodeLoaded = useCallback(
    async (path: string) => {
      if (!path) return;
      const state = useAgendaoStore.getState();
      const existing = findNodeByPath(state.fileTree, path);
      if (!existing || existing.type !== "directory") return;
      if (existing.childrenLoaded) return;

      setWorkspaceNodeLoading(path, true);
      try {
        const loaded = await apiJson<FileTreeNodeRecord>(
          `/file/tree?path=${encodeURIComponent(path)}&depth=1`,
        );
        setFileTree((current) =>
          mergeTreeNode(current, path, (node) => ({
            ...node,
            children: loaded.children ?? [],
            hasChildren: loaded.hasChildren,
            childrenLoaded: true,
          })),
        );
      } catch (error) {
        setBanner(`Failed to expand workspace folder: ${formatError(error)}`);
      } finally {
        setWorkspaceNodeLoading(path, false);
      }
    },
    [apiJson, formatError, mergeTreeNode, setBanner, setFileTree, setWorkspaceNodeLoading],
  );

  const workspaceDirty = Boolean(selectedFilePath) && selectedFileContent !== savedFileContent;
  const workspaceBasePath =
    currentSessionDirectory?.trim() ||
    currentWorkspaceSummaryPath ||
    workspaceRootFromContext(workspaceContext) ||
    workspaceRootPath ||
    serviceRootPath ||
    "";
  const workspaceTargetDirectory =
    selectedWorkspaceType === "directory" && selectedWorkspacePath
      ? selectedWorkspacePath
      : selectedFilePath
        ? parentDirectory(selectedFilePath) || workspaceBasePath
        : workspaceBasePath;
  const selectedWorkspaceReference = selectedWorkspacePath
    ? toWorkspaceReferencePath(selectedWorkspacePath, workspaceBasePath || workspaceRootPath)
    : null;
  const selectedWorkspaceFilename = selectedWorkspacePath
    ? selectedWorkspacePath.split("/").filter(Boolean).pop() || selectedWorkspacePath
    : null;
  const selectedWorkspaceIsRoot =
    Boolean(selectedWorkspacePath) &&
    selectedWorkspaceType === "directory" &&
    selectedWorkspacePath === (workspaceRootPath || workspaceBasePath);

  useEffect(() => {
    setWorkspaceDirty(workspaceDirty);
  }, [setWorkspaceDirty, workspaceDirty]);

  const confirmDiscardWorkspaceChanges = useCallback(
    (targetLabel: string) => {
      if (!workspaceDirty) {
        return true;
      }
      return window.confirm(
        `Unsaved changes in ${selectedFilePath || "the current file"} will be lost. Continue to ${targetLabel}?`,
      );
    },
    [selectedFilePath, workspaceDirty],
  );

  const reloadWorkspacePreservingSelection = useCallback(() => {
    setPendingWorkspaceSelection(
      selectedWorkspacePath
        ? { path: selectedWorkspacePath, type: selectedWorkspaceType }
        : workspaceRootPath
          ? { path: workspaceRootPath, type: "directory" }
          : null,
    );
    triggerWorkspaceReload();
  }, [
    selectedWorkspacePath,
    selectedWorkspaceType,
    setPendingWorkspaceSelection,
    triggerWorkspaceReload,
    workspaceRootPath,
  ]);

  const reloadWorkspaceWithSelection = useCallback(
    (path: string | null, type: "file" | "directory" = "directory") => {
      setPendingWorkspaceSelection(path ? { path, type } : null);
      triggerWorkspaceReload();
    },
    [setPendingWorkspaceSelection, triggerWorkspaceReload],
  );

  const selectWorkspaceNode = useCallback(
    (path: string, typeHint?: "file" | "directory") => {
      const requestedType = typeHint ?? "file";
      if (
        selectedFilePath &&
        workspaceDirty &&
        (path !== selectedWorkspacePath || requestedType !== selectedWorkspaceType) &&
        !confirmDiscardWorkspaceChanges("switch workspace selection")
      ) {
        return false;
      }

      const node = findNodeByPath(fileTree, path);
      if (node) {
        setSelectedWorkspacePath(node.path);
        setSelectedWorkspaceType(node.type);
        setSelectedFilePath(node.type === "file" ? node.path : null);
        setWorkspacePanelTab(node.type === "file" ? "preview" : "files");
        if (node.type === "directory") {
          void ensureWorkspaceNodeLoaded(node.path);
        }
        return true;
      }

      setPendingWorkspaceSelection({ path, type: requestedType });
      setWorkspacePanelTab(requestedType === "file" ? "preview" : "files");
      triggerWorkspaceReload();
      return true;
    },
    [
      confirmDiscardWorkspaceChanges,
      fileTree,
      selectedFilePath,
      selectedWorkspacePath,
      selectedWorkspaceType,
      setPendingWorkspaceSelection,
      setSelectedFilePath,
      setSelectedWorkspacePath,
      setSelectedWorkspaceType,
      setWorkspacePanelTab,
      triggerWorkspaceReload,
      workspaceDirty,
      ensureWorkspaceNodeLoaded,
    ],
  );

  const locateAttachmentInWorkspace = useCallback(
    (attachment: PromptPart) => {
      const path = attachmentWorkspacePath(attachment);
      if (!path) return;
      const selected = selectWorkspaceNode(
        path,
        attachment.type === "file" && attachment.mime === "application/x-directory"
          ? "directory"
          : "file",
      );
      if (selected) {
        setWorkspacePanelTab("files");
      }
      setBanner(`Located ${attachmentLabel(attachment)} in workspace`);
    },
    [selectWorkspaceNode, setBanner, setWorkspacePanelTab],
  );

  const insertWorkspaceReference = useCallback(() => {
    if (!selectedWorkspaceReference) return;
    setComposer((current) => appendReferenceToken(current, selectedWorkspaceReference));
    setBanner(`Inserted @${selectedWorkspaceReference}`);
  }, [selectedWorkspaceReference, setBanner, setComposer]);

  const attachSelectedWorkspaceNode = useCallback(() => {
    if (!selectedWorkspacePath) return;

    const nextAttachment: PromptPart = {
      type: "file",
      url: fileUrlFromPath(selectedWorkspacePath),
      filename: selectedWorkspaceReference || selectedWorkspaceFilename || "attachment",
      mime: guessWorkspaceMime(selectedWorkspacePath, selectedWorkspaceType),
    };
    const currentAttachments = useAgendaoStore.getState().attachments;
    const existingIndex = currentAttachments.findIndex(
      (part) => part.type === "file" && part.url === nextAttachment.url,
    );
    if (existingIndex >= 0) {
      setSelectedAttachmentIndex(existingIndex);
      return;
    }

    const nextIndex = currentAttachments.length;
    setAttachments([...currentAttachments, nextAttachment]);
    setSelectedAttachmentIndex(nextIndex);
    setBanner(
      selectedWorkspaceType === "directory"
        ? `Attached directory ${selectedWorkspaceReference || selectedWorkspacePath}`
        : `Attached file ${selectedWorkspaceReference || selectedWorkspacePath}`,
    );
  }, [
    selectedWorkspaceFilename,
    selectedWorkspacePath,
    selectedWorkspaceReference,
    selectedWorkspaceType,
    setAttachments,
    setBanner,
    setSelectedAttachmentIndex,
  ]);

  const saveSelectedFile = useCallback(async () => {
    if (!selectedFilePath || fileSaving) return;
    setFileSaving(true);
    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: selectedFilePath,
          content: selectedFileContent,
        }),
      });
      setSavedFileContent(selectedFileContent);
      setBanner(`Saved ${selectedFilePath}`);
    } catch (error) {
      setBanner(`Failed to save file: ${formatError(error)}`);
    } finally {
      setFileSaving(false);
    }
  }, [
    api,
    fileSaving,
    formatError,
    selectedFileContent,
    selectedFilePath,
    setBanner,
    setFileSaving,
    setSavedFileContent,
  ]);

  const createWorkspaceDirectory = useCallback(async () => {
    const requestedPath = window.prompt("New folder path", "notes");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a folder and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("Directory path is required");
      return;
    }

    try {
      const response = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
        method: "POST",
        body: JSON.stringify({ path: targetPath }),
      });
      setPendingWorkspaceSelection({ path: response.path, type: "directory" });
      triggerWorkspaceReload();
      setBanner(`Created directory ${response.path}`);
    } catch (error) {
      setBanner(`Failed to create directory: ${formatError(error)}`);
    }
  }, [
    apiJson,
    confirmDiscardWorkspaceChanges,
    formatError,
    setBanner,
    setPendingWorkspaceSelection,
    triggerWorkspaceReload,
    workspaceBasePath,
    workspaceTargetDirectory,
  ]);

  const createWorkspaceFile = useCallback(async () => {
    const requestedPath = window.prompt("New file path", "notes.md");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a file and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("File path is required");
      return;
    }

    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: targetPath,
          content: "",
          create_parents: true,
        }),
      });
      setPendingWorkspaceSelection({ path: targetPath, type: "file" });
      triggerWorkspaceReload();
      setBanner(`Created ${targetPath}`);
    } catch (error) {
      setBanner(`Failed to create file: ${formatError(error)}`);
    }
  }, [
    api,
    confirmDiscardWorkspaceChanges,
    formatError,
    setBanner,
    setPendingWorkspaceSelection,
    triggerWorkspaceReload,
    workspaceBasePath,
    workspaceTargetDirectory,
  ]);

  const deleteSelectedWorkspaceNode = useCallback(async () => {
    if (!selectedWorkspacePath || fileDeleting) return;
    if (selectedWorkspaceIsRoot) {
      setBanner("Refusing to delete the workspace root directory");
      return;
    }
    if (!confirmDiscardWorkspaceChanges("delete the selected workspace node")) {
      return;
    }
    if (!window.confirm(`Delete ${selectedWorkspacePath}?`)) return;

    setFileDeleting(true);
    try {
      await api("/file", {
        method: "DELETE",
        body: JSON.stringify({
          path: selectedWorkspacePath,
          recursive: selectedWorkspaceType === "directory",
        }),
      });
      const nextPath = parentDirectory(selectedWorkspacePath) || workspaceBasePath;
      setPendingWorkspaceSelection(nextPath ? { path: nextPath, type: "directory" } : null);
      triggerWorkspaceReload();
      setBanner(`Deleted ${selectedWorkspacePath}`);
    } catch (error) {
      setBanner(`Failed to delete selection: ${formatError(error)}`);
    } finally {
      setFileDeleting(false);
    }
  }, [
    api,
    confirmDiscardWorkspaceChanges,
    fileDeleting,
    formatError,
    selectedWorkspaceIsRoot,
    selectedWorkspacePath,
    selectedWorkspaceType,
    setBanner,
    setFileDeleting,
    setPendingWorkspaceSelection,
    triggerWorkspaceReload,
    workspaceBasePath,
  ]);

  const downloadSelectedFile = useCallback(() => {
    if (!selectedFilePath) return;
    window.location.assign(apiUrl(`/file/download?path=${encodeURIComponent(selectedFilePath)}`));
  }, [selectedFilePath]);

  const uploadWorkspaceFiles = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(event.target.files ?? []);
      if (!files.length || fileUploading) return;

      if (!confirmDiscardWorkspaceChanges("upload files and refresh workspace")) {
        event.target.value = "";
        return;
      }

      setFileUploading(true);
      try {
        const payloadFiles = await Promise.all(
          files.map(
            (file) =>
              new Promise<{ name: string; content: string; mime?: string }>((resolve, reject) => {
                const reader = new FileReader();
                reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
                reader.onload = () =>
                  resolve({
                    name: file.name,
                    content: String(reader.result ?? ""),
                    mime: file.type || undefined,
                  });
                reader.readAsDataURL(file);
              }),
          ),
        );

        const response = await apiJson<UploadFilesResponseRecord>("/file/upload", {
          method: "POST",
          body: JSON.stringify({
            path: workspaceTargetDirectory || workspaceBasePath || undefined,
            files: payloadFiles,
          }),
        });

        if (response.files[0]?.path) {
          setPendingWorkspaceSelection({ path: response.files[0].path, type: "file" });
        }
        triggerWorkspaceReload();
        setBanner(
          response.files.length === 1
            ? `Uploaded ${response.files[0]?.name ?? "1 file"}`
            : `Uploaded ${response.files.length} files`,
        );
      } catch (error) {
        setBanner(`Failed to upload files: ${formatError(error)}`);
      } finally {
        event.target.value = "";
        setFileUploading(false);
      }
    },
    [
      apiJson,
      confirmDiscardWorkspaceChanges,
      fileUploading,
      formatError,
      setBanner,
      setFileUploading,
      setPendingWorkspaceSelection,
      triggerWorkspaceReload,
      workspaceBasePath,
      workspaceTargetDirectory,
    ],
  );

  useEffect(() => {
    if (selectedSessionId && isOptimisticSessionId(selectedSessionId)) {
      setWorkspaceLoading(false);
      return;
    }
    let cancelled = false;
    let timer: number | null = null;

    const loadTree = async () => {
      setWorkspaceLoading(true);
      setSelectedWorkspacePath(null);
      setSelectedWorkspaceType("directory");
      setSelectedFilePath(null);
      setSelectedFileContent("");
      setSavedFileContent("");

      try {
        const requestedPath = currentSessionDirectory?.trim() ?? "";
        const query = requestedPath ? `?path=${encodeURIComponent(requestedPath)}&depth=1` : "?depth=1";
        let tree: FileTreeNodeRecord;
        try {
          tree = await apiJson<FileTreeNodeRecord>(`/file/tree${query}`);
        } catch (error) {
          const message = formatError(error);
          if (
            requestedPath &&
            message.includes("Access denied: path escapes project directory")
          ) {
            tree = await apiJson<FileTreeNodeRecord>("/file/tree?depth=1");
            setBanner(
              "Session workspace is outside the current project. Showing the current project root instead.",
            );
          } else {
            throw error;
          }
        }
        if (cancelled) return;
        setFileTree(tree);
        setWorkspaceRootPath(tree.path);
        const preferredNode = pendingWorkspaceSelection
          ? findNodeByPath(tree, pendingWorkspaceSelection.path)
          : null;
        const nextNode = preferredNode ?? tree;

        setSelectedWorkspacePath(nextNode?.path ?? null);
        setSelectedWorkspaceType(nextNode?.type ?? "directory");
        setSelectedFilePath(nextNode?.type === "file" ? nextNode.path : null);
        setPendingWorkspaceSelection(null);
        if (nextNode?.type === "directory" && nextNode.path) {
          void ensureWorkspaceNodeLoaded(nextNode.path);
        }
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to load workspace tree: ${formatError(error)}`);
          setWorkspaceRootPath(currentSessionDirectory || "");
        }
      } finally {
        if (!cancelled) {
          setWorkspaceLoading(false);
        }
      }
    };

    timer = window.setTimeout(() => {
      void loadTree();
    }, 140);
    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [
    apiJson,
    currentSessionDirectory,
    formatError,
    pendingWorkspaceSelection,
    selectedSessionId,
    setBanner,
    setFileTree,
    setSavedFileContent,
    setSelectedFileContent,
    setSelectedFilePath,
    setSelectedWorkspacePath,
    setSelectedWorkspaceType,
    setWorkspaceLoading,
    setWorkspaceRootPath,
    setPendingWorkspaceSelection,
    workspaceReloadToken,
    ensureWorkspaceNodeLoaded,
  ]);

  useEffect(() => {
    if (!selectedFilePath) {
      setSelectedFileContent("");
      setSavedFileContent("");
      return;
    }

    let cancelled = false;

    const loadFile = async () => {
      setFileLoading(true);
      try {
        const response = await apiJson<FileContentResponseRecord>(
          `/file/content?path=${encodeURIComponent(selectedFilePath)}`,
        );
        if (cancelled) return;
        setSelectedFileContent(response.content ?? "");
        setSavedFileContent(response.content ?? "");
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to read file: ${formatError(error)}`);
        }
      } finally {
        if (!cancelled) {
          setFileLoading(false);
        }
      }
    };

    void loadFile();
    return () => {
      cancelled = true;
    };
  }, [
    apiJson,
    formatError,
    selectedFilePath,
    setBanner,
    setFileLoading,
    setSavedFileContent,
    setSelectedFileContent,
  ]);

  useEffect(() => {
    if (!selectedSessionId || !workspaceBasePath) return;
    const previewPath = previewPathFromMessageMetadata(messageHistory, workspaceBasePath);
    if (!previewPath) return;

    const signature = `${selectedSessionId}:${previewPath}`;
    if (autoPreviewSignatureRef.current === signature) {
      return;
    }

    if (selectWorkspaceNode(previewPath, "file")) {
      autoPreviewSignatureRef.current = signature;
      setWorkspacePanelTab("preview");
    }
  }, [
    messageHistory,
    selectWorkspaceNode,
    selectedSessionId,
    setWorkspacePanelTab,
    workspaceBasePath,
  ]);

  return useMemo(
    () => ({
      attachSelectedWorkspaceNode,
      createWorkspaceDirectory,
      createWorkspaceFile,
      deleteSelectedWorkspaceNode,
      downloadSelectedFile,
      insertWorkspaceReference,
      locateAttachmentInWorkspace,
      ensureWorkspaceNodeLoaded,
      reloadWorkspacePreservingSelection,
      reloadWorkspaceWithSelection,
      saveSelectedFile,
      selectWorkspaceNode,
      selectedWorkspaceFilename,
      selectedWorkspaceIsRoot,
      selectedWorkspaceReference,
      uploadWorkspaceFiles,
      workspaceBasePath,
      workspaceDirty,
      workspaceTargetDirectory,
    }),
    [
      attachSelectedWorkspaceNode,
      createWorkspaceDirectory,
      createWorkspaceFile,
      deleteSelectedWorkspaceNode,
      downloadSelectedFile,
      insertWorkspaceReference,
      locateAttachmentInWorkspace,
      reloadWorkspacePreservingSelection,
      reloadWorkspaceWithSelection,
      saveSelectedFile,
      selectWorkspaceNode,
      selectedWorkspaceFilename,
      selectedWorkspaceIsRoot,
      selectedWorkspaceReference,
      uploadWorkspaceFiles,
      workspaceBasePath,
      workspaceDirty,
      workspaceTargetDirectory,
      ensureWorkspaceNodeLoaded,
    ],
  );
}
