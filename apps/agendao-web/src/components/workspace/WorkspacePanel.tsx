"use client";

import type { ChangeEvent } from "react";
import { Suspense, lazy, useRef } from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { PanelErrorBoundary } from "./PanelErrorBoundary";
import { WorkspaceTreeNode } from "./WorkspaceTreeNode";
import { useAgendaoStore } from "../../store";
import {
  FolderTreeIcon,
  LightbulbIcon,
  EyeIcon,
  PlusIcon,
  FolderPlusIcon,
  SparklesIcon,
  UploadIcon,
  GitBranchIcon,
} from "lucide-react";
import type { useExecutionActivity } from "../../hooks/useExecutionActivity";

const SessionInsightsPanel = lazy(async () => {
  const module = await import("../session/SessionInsightsPanel");
  return { default: module.SessionInsightsPanel };
});

const FilePreviewPane = lazy(async () => {
  const module = await import("./FilePreviewPane");
  return { default: module.FilePreviewPane };
});

const SkillProposalInbox = lazy(async () => {
  const module = await import("../settings/SkillProposalInbox");
  return { default: module.SkillProposalInbox };
});

const WorktreePanel = lazy(async () => {
  const module = await import("./WorktreePanel");
  return { default: module.WorktreePanel };
});

export type WorkspacePanelTab = "files" | "insights" | "preview" | "proposals" | "worktrees";

interface WorkspacePanelProps {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  workspaceRootLabel: string;
  workspaceLinkLabel: string | null;
  workspaceLinkStageId: string | null;
  executionActivity: ReturnType<typeof useExecutionActivity>;
  schedulerNavigation: {
    navigateToStage: (stageId: string) => void;
    navigateToAttachedSession: (
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
  onExpandWorkspaceNode: (path: string) => void | Promise<void>;
  onInsertWorkspaceReference: () => void;
  onAttachSelectedWorkspaceNode: () => void;
}

export function WorkspacePanel({
  apiJson,
  workspaceRootLabel,
  workspaceLinkLabel,
  workspaceLinkStageId,
  onCreateWorkspaceFile,
  onCreateWorkspaceDirectory,
  onUploadWorkspaceFiles,
  onSelectWorkspaceNode,
  onExpandWorkspaceNode,
  onInsertWorkspaceReference,
  onAttachSelectedWorkspaceNode,
  schedulerNavigation,
  executionActivity,
}: WorkspacePanelProps) {
  const activeTab = useAgendaoStore((s) => s.workspacePanelTab);
  const setActiveTab = useAgendaoStore((s) => s.setWorkspacePanelTab);
  const workspaceLoading = useAgendaoStore((s) => s.workspaceLoading);
  const workspaceNodeLoading = useAgendaoStore((s) => s.workspaceNodeLoading);
  const fileTree = useAgendaoStore((s) => s.fileTree);
  const selectedWorkspacePath = useAgendaoStore((s) => s.selectedWorkspacePath);
  const workspaceUploadInputRef = useRef<HTMLInputElement | null>(null);
  const workspaceRootName =
    workspaceRootLabel.split("/").filter(Boolean).pop() || workspaceRootLabel || "Workspace";

  return (
    <div className="flex flex-col h-full overflow-hidden" data-testid="workspace-panel">
      <div className="flex h-11 items-center justify-between border-b border-border shrink-0 px-2.5" data-testid="workspace-panel-header">
        <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto" data-testid="workspace-panel-tabs">
          <button
            className={cn(
              "inline-flex min-w-0 items-center justify-center gap-1.5 rounded-md border-b-2 border-transparent px-2.5 py-1 text-[11px] font-medium transition-colors",
              activeTab === "files"
                ? "border-foreground/55 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            type="button"
            onClick={() => setActiveTab("files")}
            title={workspaceRootLabel}
          >
            <FolderTreeIcon className="size-3.25" />
            <span className="truncate">{activeTab === "files" ? workspaceRootName : "Files"}</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-md border-b-2 border-transparent px-2.5 py-1 text-[11px] font-medium transition-colors",
              activeTab === "insights"
                ? "border-foreground/55 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            type="button"
            onClick={() => setActiveTab("insights")}
          >
            <LightbulbIcon className="size-3.25" />
            <span>Insights</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-md border-b-2 border-transparent px-2.5 py-1 text-[11px] font-medium transition-colors",
              activeTab === "preview"
                ? "border-foreground/55 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            type="button"
            onClick={() => setActiveTab("preview")}
          >
            <EyeIcon className="size-3.25" />
            <span>Preview</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-md border-b-2 border-transparent px-2.5 py-1 text-[11px] font-medium transition-colors",
              activeTab === "proposals"
                ? "border-foreground/55 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            type="button"
            onClick={() => setActiveTab("proposals")}
          >
            <SparklesIcon className="size-3.25" />
            <span>Proposals</span>
          </button>
          <button
            className={cn(
              "inline-flex items-center justify-center gap-1.5 rounded-md border-b-2 border-transparent px-2.5 py-1 text-[11px] font-medium transition-colors",
              activeTab === "worktrees"
                ? "border-foreground/55 text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            type="button"
            data-testid="workspace-tab-worktrees"
            onClick={() => setActiveTab("worktrees")}
          >
            <GitBranchIcon className="size-3.25" />
            <span>Worktrees</span>
          </button>
        </div>
        <div className="flex items-center gap-0.5 flex-shrink-0">
          <Button
            variant="ghost"
            size="icon"
            type="button"
            className="h-6.5 w-6.5"
            onClick={() => void onCreateWorkspaceFile()}
            title="New file"
          >
            <PlusIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            type="button"
            className="h-6.5 w-6.5"
            onClick={() => void onCreateWorkspaceDirectory()}
            title="New folder"
          >
            <FolderPlusIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            type="button"
            className="h-6.5 w-6.5"
            data-testid="workspace-insert-reference"
            disabled={!selectedWorkspacePath}
            onClick={onInsertWorkspaceReference}
            title="Insert @ reference"
          >
            <span className="text-[11px] font-semibold">@</span>
          </Button>
          <Button
            variant="ghost"
            size="icon"
            type="button"
            className="h-6.5 w-6.5"
            data-testid="workspace-attach"
            disabled={!selectedWorkspacePath}
            onClick={onAttachSelectedWorkspaceNode}
            title="Attach to composer"
          >
            <PlusIcon className="size-3" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            type="button"
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
                  loadingPaths={workspaceNodeLoading}
                  onSelectNode={(node) => {
                    onSelectWorkspaceNode(node.path, node.type);
                    setActiveTab(node.type === "file" ? "preview" : "files");
                    schedulerNavigation.restoreActiveStage();
                  }}
                  onExpandNode={(node) => onExpandWorkspaceNode(node.path)}
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
        {activeTab === "proposals" ? (
          <PanelErrorBoundary label="Proposals">
            <Suspense
              fallback={
                <div className="flex items-center justify-center py-6 text-muted-foreground/60">
                  <span className="text-[10px]">Loading proposals...</span>
                </div>
              }
            >
              <SkillProposalInbox />
            </Suspense>
          </PanelErrorBoundary>
        ) : null}
        {activeTab === "worktrees" ? (
          <PanelErrorBoundary label="Worktrees">
            <Suspense
              fallback={
                <div className="flex items-center justify-center py-6 text-muted-foreground/60">
                  <span className="text-[10px]">Loading worktrees...</span>
                </div>
              }
            >
              <WorktreePanel className="h-full border-0 bg-transparent p-2 shadow-none" />
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
