"use client";

import type { ChangeEvent } from "react";
import { Suspense, lazy, useRef } from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { FileTreeNodeRecord } from "@/lib/workspace";
import { PanelErrorBoundary } from "./PanelErrorBoundary";
import { WorkspaceTreeNode } from "./WorkspaceTreeNode";
import {
  FolderTreeIcon,
  LightbulbIcon,
  EyeIcon,
  PlusIcon,
  FolderPlusIcon,
  UploadIcon,
} from "lucide-react";
import type { useExecutionActivity } from "../hooks/useExecutionActivity";

const SessionInsightsPanel = lazy(async () => {
  const module = await import("./SessionInsightsPanel");
  return { default: module.SessionInsightsPanel };
});

const FilePreviewPane = lazy(async () => {
  const module = await import("./FilePreviewPane");
  return { default: module.FilePreviewPane };
});

export type WorkspacePanelTab = "files" | "insights" | "preview";

interface WorkspacePanelProps {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  activeTab: WorkspacePanelTab;
  workspaceLoading: boolean;
  fileTree: FileTreeNodeRecord | null;
  workspaceRootPath: string;
  workspaceRootLabel: string;
  selectedWorkspacePath: string | null;
  selectedWorkspaceType: "file" | "directory";
  workspaceLinkLabel: string | null;
  workspaceLinkStageId: string | null;
  selectedFilePath: string | null;
  selectedFileContent: string;
  fileLoading: boolean;
  fileSaving: boolean;
  fileDeleting: boolean;
  fileUploading: boolean;
  workspaceDirty: boolean;
  selectedWorkspaceIsRoot: boolean;
  selectedWorkspaceReference: string | null;
  lastAssistant: { title?: string; text?: string } | null;
  activeStageId: string | null;
  previewStageId: string | null;
  executionActivity: ReturnType<typeof useExecutionActivity>;
  conversationJump: unknown;
  schedulerNavigation: {
    navigateToStage: (stageId: string) => void;
    navigateToChildSession: (
      sessionId: string,
      context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null },
    ) => void | Promise<void>;
    previewStage: (stageId: string | null) => void;
    restoreActiveStage: () => void;
  };
  onCreateWorkspaceFile: () => Promise<void>;
  onCreateWorkspaceDirectory: () => Promise<void>;
  onUploadWorkspaceFiles: (event: ChangeEvent<HTMLInputElement>) => void;
  onSelectWorkspaceNode: (path: string, type: "file" | "directory") => void;
  onWorkspaceContentChange: (content: string) => void;
  onInsertWorkspaceReference: () => void;
  onAttachSelectedWorkspaceNode: () => void;
  onDownloadSelectedFile: () => void;
  onDeleteSelectedWorkspaceNode: () => Promise<void>;
  onSaveSelectedFile: () => Promise<void>;
  onActiveTabChange: (tab: WorkspacePanelTab) => void;
}

export function WorkspacePanel({
  apiJson,
  activeTab,
  workspaceLoading,
  fileTree,
  workspaceRootPath,
  workspaceRootLabel,
  selectedWorkspacePath,
  workspaceLinkLabel,
  workspaceLinkStageId,
  onCreateWorkspaceFile,
  onCreateWorkspaceDirectory,
  onUploadWorkspaceFiles,
  onSelectWorkspaceNode,
  onActiveTabChange,
  schedulerNavigation,
  executionActivity,
}: WorkspacePanelProps) {
  const workspaceUploadInputRef = useRef<HTMLInputElement | null>(null);
  const workspaceRootName =
    workspaceRootLabel.split("/").filter(Boolean).pop() || workspaceRootLabel || "Workspace";

  return (
    <div className="flex flex-col h-full overflow-hidden" data-testid="workspace-panel">
      <div className="flex items-center justify-between border-b border-border shrink-0 px-3 py-2">
        <div className="flex min-w-0 flex-1 items-center">
          <button
            className={cn(
              "inline-flex min-w-0 items-center justify-center gap-1.5 rounded-full px-3 py-1.5 text-[11px] font-medium transition-colors",
              activeTab === "files"
                ? "bg-foreground/8 text-foreground"
                : "text-muted-foreground hover:bg-accent/45 hover:text-foreground"
            )}
            type="button"
            onClick={() => onActiveTabChange("files")}
            title={workspaceRootLabel}
          >
            <FolderTreeIcon className="size-3.25" />
            <span className="truncate">{activeTab === "files" ? workspaceRootName : "Files"}</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-full px-3 py-1.5 text-[11px] font-medium transition-colors",
              activeTab === "insights"
                ? "bg-foreground/8 text-foreground"
                : "text-muted-foreground hover:bg-accent/45 hover:text-foreground"
            )}
            type="button"
            onClick={() => onActiveTabChange("insights")}
          >
            <LightbulbIcon className="size-3.25" />
            <span>Insights</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-full px-3 py-1.5 text-[11px] font-medium transition-colors",
              activeTab === "preview"
                ? "bg-foreground/8 text-foreground"
                : "text-muted-foreground hover:bg-accent/45 hover:text-foreground"
            )}
            type="button"
            onClick={() => onActiveTabChange("preview")}
          >
            <EyeIcon className="size-3.25" />
            <span>Preview</span>
          </button>
        </div>
        <div className="flex items-center gap-0.5 flex-shrink-0">
          <Button
            variant="ghost"
            size="icon"
            className="h-6.5 w-6.5"
            onClick={() => void onCreateWorkspaceFile()}
            title="New file"
          >
            <PlusIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-6.5 w-6.5"
            onClick={() => void onCreateWorkspaceDirectory()}
            title="New folder"
          >
            <FolderPlusIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-6.5 w-6.5"
            onClick={() => workspaceUploadInputRef.current?.click()}
            title="Upload"
          >
            <UploadIcon className="size-3" />
          </Button>
        </div>
      </div>

      {/* File Tree */}
      <div className="flex-1 min-h-0 overflow-auto py-1">
        {activeTab === "insights" ? (
          <PanelErrorBoundary label="Insights">
            <Suspense
              fallback={
                <div className="flex items-center justify-center py-6 text-muted-foreground/60">
                  <span className="text-[10px]">Loading insights...</span>
                </div>
              }
            >
              <div className="p-2">
                <SessionInsightsPanel activity={executionActivity} apiJson={apiJson} />
              </div>
            </Suspense>
          </PanelErrorBoundary>
        ) : null}
        {activeTab === "files"
          ? workspaceLoading
            ? (
              <div className="flex items-center justify-center py-6 text-muted-foreground/60">
                <span className="text-[10px]">Loading...</span>
              </div>
            )
            : fileTree
              ? (
                <WorkspaceTreeNode
                  node={fileTree}
                  selectedPath={selectedWorkspacePath}
                  linkedPath={workspaceLinkLabel ? selectedWorkspacePath : null}
                  linkedLabel={workspaceLinkLabel}
                  linkedStageId={workspaceLinkStageId}
                  onSelectNode={(node) => {
                    onSelectWorkspaceNode(node.path, node.type);
                    onActiveTabChange(node.type === "file" ? "preview" : "files");
                    schedulerNavigation.restoreActiveStage();
                  }}
                  onPreviewStage={schedulerNavigation.previewStage}
                />
              )
              : (
                <div className="text-[10px] text-muted-foreground/50 px-3 py-2">
                  No workspace
                </div>
              )
          : null}
        {activeTab === "preview" ? (
          <PanelErrorBoundary label="Preview">
            <Suspense
              fallback={
                <div className="flex items-center justify-center py-6 text-muted-foreground/60">
                  <span className="text-[10px]">Loading preview...</span>
                </div>
              }
            >
              <FilePreviewPane filePath={selectedWorkspacePath} />
            </Suspense>
          </PanelErrorBoundary>
        ) : null}
      </div>

      {/* Hidden file input */}
      <input
        ref={workspaceUploadInputRef}
        className="hidden"
        type="file"
        multiple
        onChange={onUploadWorkspaceFiles}
      />
    </div>
  );
}
