import type { ChangeEvent } from "react";
import React, { Suspense, useRef } from "react";
import { FolderTreeIcon, LoaderCircleIcon, SparklesIcon } from "lucide-react";
import type { useConversationJump } from "../../hooks/useConversationJump";
import type { useExecutionActivity } from "../../hooks/useExecutionActivity";
import type { useSchedulerNavigation } from "../../hooks/useSchedulerNavigation";
import type { useTerminalSessions } from "../../hooks/useTerminalSessions";
import type { FeedMessage } from "../../lib/history";
import type { FileTreeNodeRecord } from "../../lib/workspace";
import { WorkspaceTreeNode } from "./WorkspaceTreeNode";
import { DeferredTerminalPanel } from "../terminal/DeferredTerminalPanel";
import { PanelErrorBoundary } from "./PanelErrorBoundary";

const ExecutionActivityPanel = React.lazy(async () => {
  const module = await import("../execution/ExecutionActivityPanel");
  return { default: module.ExecutionActivityPanel };
});

const SessionInsightsPanel = React.lazy(async () => {
  const module = await import("../session/SessionInsightsPanel");
  return { default: module.SessionInsightsPanel };
});

interface WorkspaceInspectorPanelProps {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  workspaceLoading: boolean;
  fileTree: FileTreeNodeRecord | null;
  workspaceRootLabel: string;
  selectedWorkspacePath: string | null;
  workspaceLinkLabel: string | null;
  workspaceLinkStageId: string | null;
  selectedWorkspaceFilename: string | null;
  selectedWorkspaceType: "file" | "directory";
  workspaceDirty: boolean;
  selectedFilePath: string | null;
  selectedFileContent: string;
  fileLoading: boolean;
  fileSaving: boolean;
  fileDeleting: boolean;
  fileUploading: boolean;
  selectedWorkspaceIsRoot: boolean;
  selectedWorkspaceReference: string | null;
  lastAssistant: Pick<FeedMessage, "title" | "text"> | null;
  activeStageId: string | null;
  previewStageId: string | null;
  executionActivity: ReturnType<typeof useExecutionActivity>;
  conversationJump: ReturnType<typeof useConversationJump>;
  schedulerNavigation: ReturnType<typeof useSchedulerNavigation>;
  terminalExpanded: boolean;
  terminalSessions: ReturnType<typeof useTerminalSessions>;
  onExpandTerminal: () => void;
  onCreateWorkspaceFile: () => void | Promise<void>;
  onCreateWorkspaceDirectory: () => void | Promise<void>;
  onUploadWorkspaceFiles: (event: ChangeEvent<HTMLInputElement>) => void | Promise<void>;
  onSelectWorkspaceNode: (path: string, type?: "file" | "directory") => void;
  onWorkspaceContentChange: (value: string) => void;
  onInsertWorkspaceReference: () => void;
  onAttachSelectedWorkspaceNode: () => void;
  onDownloadSelectedFile: () => void;
  onDeleteSelectedWorkspaceNode: () => void | Promise<void>;
  onSaveSelectedFile: () => void | Promise<void>;
}

function InspectorLoadingCard({ label }: { label: string }) {
  return (
    <div className="roc-state-card grid gap-2.5" data-tone="loading">
      <div className="roc-status-row">
        <span className="roc-status-orb" data-tone="loading">
          <LoaderCircleIcon className="size-4 animate-spin" />
        </span>
        <span>Loading {label}…</span>
      </div>
      <p className="text-sm leading-6 text-muted-foreground">This panel is being loaded as a separate chunk.</p>
    </div>
  );
}

const workspaceActionButtonClass = "roc-action roc-action-pill px-4 cursor-pointer";

export function WorkspaceInspectorPanel({
  apiJson,
  workspaceLoading,
  fileTree,
  workspaceRootLabel,
  selectedWorkspacePath,
  workspaceLinkLabel,
  workspaceLinkStageId,
  selectedWorkspaceFilename,
  selectedWorkspaceType,
  workspaceDirty,
  selectedFilePath,
  selectedFileContent,
  fileLoading,
  fileSaving,
  fileDeleting,
  fileUploading,
  selectedWorkspaceIsRoot,
  selectedWorkspaceReference,
  lastAssistant,
  activeStageId,
  previewStageId,
  executionActivity,
  conversationJump,
  schedulerNavigation,
  terminalExpanded,
  terminalSessions,
  onExpandTerminal,
  onCreateWorkspaceFile,
  onCreateWorkspaceDirectory,
  onUploadWorkspaceFiles,
  onSelectWorkspaceNode,
  onWorkspaceContentChange,
  onInsertWorkspaceReference,
  onAttachSelectedWorkspaceNode,
  onDownloadSelectedFile,
  onDeleteSelectedWorkspaceNode,
  onSaveSelectedFile,
}: WorkspaceInspectorPanelProps) {
  const workspaceUploadInputRef = useRef<HTMLInputElement | null>(null);

  return (
    <div className="flex flex-col gap-5 min-h-0 overflow-y-auto p-5" data-testid="workspace-inspector">
      <div className="roc-panel roc-rail-panel min-h-0 p-5" data-testid="workspace-tree-card">
        <div className="roc-rail-header">
          <div className="roc-rail-headline">
            <p className="roc-section-label">Workspace</p>
            <h3 className="roc-rail-title">File Tree</h3>
            <p className="roc-rail-description break-all">{workspaceRootLabel}</p>
          </div>
          <div className="roc-rail-toolbar">
            <span className="roc-badge px-3 py-1.5 text-xs">
              {workspaceLoading ? "loading" : `${fileTree?.children?.length ?? 0} items`}
            </span>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-new-file"
              onClick={() => void onCreateWorkspaceFile()}
            >
              New File
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-new-folder"
              onClick={() => void onCreateWorkspaceDirectory()}
            >
              New Folder
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-upload"
              disabled={fileUploading}
              onClick={() => workspaceUploadInputRef.current?.click()}
            >
              {fileUploading ? "Uploading..." : "Upload"}
            </button>
            <input
              ref={workspaceUploadInputRef}
              className="hidden"
              data-testid="workspace-upload-input"
              type="file"
              multiple
              onChange={onUploadWorkspaceFiles}
            />
          </div>
        </div>
        <div className="mt-3.5 flex flex-col gap-1 max-h-80 overflow-auto" data-testid="workspace-tree">
          {workspaceLoading ? (
            <div className="roc-state-card" data-tone="loading">
              <div className="roc-status-row">
                <span className="roc-status-orb" data-tone="loading">
                  <LoaderCircleIcon className="size-4 animate-spin" />
                </span>
                <span>Loading workspace tree…</span>
              </div>
            </div>
          ) : fileTree ? (
            <WorkspaceTreeNode
              node={fileTree}
              selectedPath={selectedWorkspacePath}
              linkedPath={workspaceLinkLabel ? selectedWorkspacePath : null}
              linkedLabel={workspaceLinkLabel}
              linkedStageId={workspaceLinkStageId}
              onSelectNode={(node) => {
                onSelectWorkspaceNode(node.path, node.type);
                schedulerNavigation.restoreActiveStage();
              }}
              onPreviewStage={schedulerNavigation.previewStage}
            />
          ) : (
            <div className="roc-rail-empty" data-tone="muted">
              <div className="roc-status-row">
                <span className="roc-status-orb">
                  <FolderTreeIcon className="size-4" />
                </span>
                <span>No workspace tree available yet.</span>
              </div>
            </div>
          )}
        </div>
      </div>

      <div className="roc-panel roc-rail-panel min-h-0 p-5" data-testid="workspace-editor-card">
        <div className="roc-rail-header">
          <div className="roc-rail-headline">
            <p className="roc-section-label">Workspace</p>
            <h3 className="roc-rail-title">{selectedWorkspaceFilename || "Workspace Preview"}</h3>
            {selectedWorkspacePath ? (
              <p className="roc-rail-description break-all">{selectedWorkspacePath}</p>
            ) : null}
          </div>
          <div className="roc-rail-toolbar">
            <span className="roc-badge px-3 py-1.5 text-xs">
              {selectedWorkspaceType === "directory" ? "directory" : workspaceDirty ? "dirty" : "saved"}
            </span>
            {selectedWorkspacePath && workspaceLinkLabel ? <span className="roc-badge px-3 py-1.5 text-xs">{workspaceLinkLabel}</span> : null}
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-insert-reference"
              disabled={!selectedWorkspacePath}
              onClick={onInsertWorkspaceReference}
            >
              Insert @
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-attach"
              disabled={!selectedWorkspacePath}
              onClick={onAttachSelectedWorkspaceNode}
            >
              Attach
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-download"
              disabled={!selectedFilePath || fileDeleting || fileSaving}
              onClick={onDownloadSelectedFile}
            >
              Download
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-delete"
              disabled={!selectedWorkspacePath || selectedWorkspaceIsRoot || fileDeleting || fileSaving}
              onClick={() => void onDeleteSelectedWorkspaceNode()}
            >
              {fileDeleting ? "Deleting..." : "Delete"}
            </button>
            <button
              className={workspaceActionButtonClass}
              type="button"
              data-testid="workspace-save"
              disabled={!selectedFilePath || fileLoading || fileSaving || fileDeleting || !workspaceDirty}
              onClick={() => void onSaveSelectedFile()}
            >
              {fileSaving ? "Saving..." : "Save"}
            </button>
          </div>
        </div>
        {selectedFilePath ? (
          <>
            <textarea
              className="roc-form-textarea mt-3.5 min-h-80 font-mono"
              data-testid="workspace-editor"
              value={selectedFileContent}
              onChange={(event) => onWorkspaceContentChange(event.target.value)}
              disabled={fileLoading}
              spellCheck={false}
            />
          </>
        ) : selectedWorkspacePath && selectedWorkspaceType === "directory" ? (
          <div className="roc-rail-empty" data-tone="muted">
            <div className="roc-status-row">
              <span className="roc-status-orb">
                <FolderTreeIcon className="size-4" />
              </span>
              <span>{selectedWorkspaceFilename || "Directory selected"}</span>
            </div>
            <p className="text-sm leading-6 text-muted-foreground">{selectedWorkspacePath}</p>
            <p className="text-sm leading-6 text-muted-foreground">
              This directory can be referenced with `{selectedWorkspaceReference ? `@${selectedWorkspaceReference}` : "@."}`{" "}
              or attached directly as a workspace context part.
            </p>
          </div>
        ) : (
          <div className="roc-rail-empty" data-tone="muted">
            <div className="roc-status-row">
              <span className="roc-status-orb">
                <SparklesIcon className="size-4" />
              </span>
              <span>{lastAssistant?.title || "No file selected"}</span>
            </div>
            <p className="text-sm leading-6 text-muted-foreground">
              {lastAssistant?.text ||
                "Pick a file from the workspace tree to read and edit it in the new frontend."}
            </p>
          </div>
        )}
      </div>

      <PanelErrorBoundary label="Scheduler Activity">
        <Suspense fallback={<InspectorLoadingCard label="scheduler activity" />}>
          <ExecutionActivityPanel
            activity={executionActivity}
            activeStageId={activeStageId}
            previewStageId={previewStageId}
            onJumpToConversation={conversationJump.jumpOrQueueConversationTarget}
            onNavigateStage={schedulerNavigation.navigateToStage}
            onNavigateAttachedSession={schedulerNavigation.navigateToAttachedSession}
            onNavigateToolCall={schedulerNavigation.navigateToToolCall}
          />
        </Suspense>
      </PanelErrorBoundary>

      <PanelErrorBoundary label="Session Insights">
        <Suspense fallback={<InspectorLoadingCard label="session insights" />}>
          <SessionInsightsPanel activity={executionActivity} apiJson={apiJson} />
        </Suspense>
      </PanelErrorBoundary>

      <PanelErrorBoundary label="Terminal">
        <DeferredTerminalPanel
          expanded={terminalExpanded}
          onExpand={onExpandTerminal}
          terminal={terminalSessions}
        />
      </PanelErrorBoundary>
    </div>
  );
}
