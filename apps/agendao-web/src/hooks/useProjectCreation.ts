import { useCallback } from "react";
import { formatError } from "../lib/api";
import { resolveWorkspacePath } from "../lib/composerContext";
import { basenamePath } from "../lib/sidebar";
import type { DirectoryCreateResponseRecord } from "../lib/workspace";
import { useAgendaoStore } from "../store";

interface UseProjectCreationOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  serviceRootPath: string;
  workspaceBasePath: string;
  createSession: (options?: { directory?: string; title?: string; projectId?: string }) => Promise<string>;
  reloadWorkspaceWithSelection: (path: string | null, type?: "file" | "directory") => void;
}

export function useProjectCreation({
  apiJson,
  serviceRootPath,
  workspaceBasePath,
  createSession,
  reloadWorkspaceWithSelection,
}: UseProjectCreationOptions) {
  const setBanner = useAgendaoStore((s) => s.setBanner);

  return useCallback(
    async (input: { path: string; title?: string }) => {
      const baseRoot = serviceRootPath || workspaceBasePath;
      const targetPath = resolveWorkspacePath(baseRoot, input.path);
      if (!targetPath) {
        setBanner("Project path is required");
        return;
      }

      try {
        const directory = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
          method: "POST",
          body: JSON.stringify({ path: targetPath }),
        });
        const folderName = basenamePath(directory.path);
        await createSession({
          directory: directory.path,
          projectId: folderName,
          title: input.title || `${folderName} workspace`,
        });
        reloadWorkspaceWithSelection(directory.path, "directory");
        setBanner(`Created project ${folderName}`);
      } catch (error) {
        setBanner(`Failed to create project: ${formatError(error)}`);
      }
    },
    [
      apiJson,
      createSession,
      reloadWorkspaceWithSelection,
      serviceRootPath,
      setBanner,
      workspaceBasePath,
    ],
  );
}
