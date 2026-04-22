import type { FeedMessage } from "./history";

export interface FileTreeNodeRecord {
  name: string;
  path: string;
  type: "file" | "directory";
  size?: number | null;
  modified?: number | null;
  children?: FileTreeNodeRecord[];
}

export interface FileContentResponseRecord {
  path: string;
  content: string;
}

export interface PathsResponseRecord {
  cwd: string;
  home: string;
  config: string;
  data: string;
}

export interface WorkspaceIdentityRecord {
  requested_dir: string;
  workspace_root: string;
  config_dir?: string | null;
  workspace_key: string;
}

export interface WorkspaceContextRecord {
  identity: WorkspaceIdentityRecord;
  mode: "shared" | "isolated";
  config: Record<string, unknown>;
  recent_models?: Array<{ provider: string; model: string }>;
}

export interface UploadedFileRecord {
  name: string;
  path: string;
  size: number;
  mime?: string;
}

export interface UploadFilesResponseRecord {
  files: UploadedFileRecord[];
}

export interface DirectoryCreateResponseRecord {
  path: string;
}

export type WorkspaceFeedMessage = Pick<FeedMessage, "title" | "text">;

export function workspaceRootFromContext(context: WorkspaceContextRecord | null): string {
  return context?.identity?.workspace_root?.trim() || "";
}

export function workspaceModeFromContext(
  context: WorkspaceContextRecord | null,
): "shared" | "isolated" | null {
  return context?.mode ?? null;
}
