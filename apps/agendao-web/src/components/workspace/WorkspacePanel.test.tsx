import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspacePanel } from "./WorkspacePanel";
import { resetAgendaoStore } from "../../test/store-test-utils";
import { useAgendaoStore } from "../../store";
import type { FileTreeNodeRecord } from "../../lib/workspace";
import type { useExecutionActivity } from "../../hooks/useExecutionActivity";

function createApiJsonStub(): <T>(path: string, options?: RequestInit) => Promise<T> {
  return vi.fn(async () => undefined) as unknown as <T>(
    path: string,
    options?: RequestInit,
  ) => Promise<T>;
}

function createExecutionActivityStub() {
  return {
    executionNodes: [],
    setSelectedExecutionId: vi.fn<(id: string | null) => void>(),
    patchActivityFilters: vi.fn<(filters: { stageId?: string; executionId?: string; eventType?: string }) => void>(),
  } as unknown as ReturnType<typeof useExecutionActivity>;
}

function renderWorkspacePanel(props: Partial<Parameters<typeof WorkspacePanel>[0]> = {}) {
  return render(
    <WorkspacePanel
      apiJson={createApiJsonStub()}
      workspaceRootLabel="/repo"
      workspaceLinkLabel={null}
      workspaceLinkStageId={null}
      executionActivity={createExecutionActivityStub()}
      schedulerNavigation={{
        navigateToStage: vi.fn<(stageId: string) => void>(),
        navigateToAttachedSession: vi.fn<(sessionId: string, context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null }) => void>(),
        previewStage: vi.fn<(stageId: string | null) => void>(),
        restoreActiveStage: vi.fn<() => void>(),
      }}
      onCreateWorkspaceFile={vi.fn<() => Promise<void>>(async () => undefined)}
      onCreateWorkspaceDirectory={vi.fn<() => Promise<void>>(async () => undefined)}
      onUploadWorkspaceFiles={vi.fn<(event: React.ChangeEvent<HTMLInputElement>) => void>()}
      onSelectWorkspaceNode={vi.fn<(path: string, type: "file" | "directory") => void>()}
      onExpandWorkspaceNode={vi.fn<(path: string) => void>()}
      onInsertWorkspaceReference={vi.fn<() => void>()}
      onAttachSelectedWorkspaceNode={vi.fn<() => void>()}
      {...props}
    />,
  );
}

describe("WorkspacePanel", () => {
  beforeEach(() => {
    resetAgendaoStore();
    useAgendaoStore.setState({
      workspacePanelTab: "files",
      workspaceLoading: false,
      fileTree: {
        name: "repo",
        path: "/repo",
        type: "directory",
        hasChildren: true,
        childrenLoaded: true,
        children: [
          {
            name: "src",
            path: "/repo/src",
            type: "directory",
            hasChildren: true,
            childrenLoaded: false,
            children: [],
          },
          {
            name: "main.ts",
            path: "/repo/main.ts",
            type: "file",
            children: [],
          },
        ],
      } as FileTreeNodeRecord,
      selectedWorkspacePath: null,
    });
  });

  it("keeps attach/reference disabled without a selection and enables them for a selected workspace node", () => {
    const onAttachSelectedWorkspaceNode = vi.fn<() => void>();
    const onInsertWorkspaceReference = vi.fn<() => void>();
    const { rerender } = renderWorkspacePanel({
      onAttachSelectedWorkspaceNode,
      onInsertWorkspaceReference,
    });

    expect(screen.getByTestId("workspace-attach")).toBeDisabled();
    expect(screen.getByTestId("workspace-insert-reference")).toBeDisabled();

    useAgendaoStore.setState({ selectedWorkspacePath: "/repo/main.ts" });
    rerender(
      <WorkspacePanel
        apiJson={createApiJsonStub()}
        workspaceRootLabel="/repo"
        workspaceLinkLabel={null}
        workspaceLinkStageId={null}
        executionActivity={createExecutionActivityStub()}
        schedulerNavigation={{
          navigateToStage: vi.fn<(stageId: string) => void>(),
          navigateToAttachedSession: vi.fn<(sessionId: string, context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null }) => void>(),
          previewStage: vi.fn<(stageId: string | null) => void>(),
          restoreActiveStage: vi.fn<() => void>(),
        }}
        onCreateWorkspaceFile={vi.fn<() => Promise<void>>(async () => undefined)}
        onCreateWorkspaceDirectory={vi.fn<() => Promise<void>>(async () => undefined)}
        onUploadWorkspaceFiles={vi.fn<(event: React.ChangeEvent<HTMLInputElement>) => void>()}
        onSelectWorkspaceNode={vi.fn<(path: string, type: "file" | "directory") => void>()}
        onExpandWorkspaceNode={vi.fn<(path: string) => void>()}
        onInsertWorkspaceReference={onInsertWorkspaceReference}
        onAttachSelectedWorkspaceNode={onAttachSelectedWorkspaceNode}
      />,
    );

    expect(screen.getByTestId("workspace-attach")).not.toBeDisabled();
    expect(screen.getByTestId("workspace-insert-reference")).not.toBeDisabled();

    fireEvent.click(screen.getByTestId("workspace-attach"));
    fireEvent.click(screen.getByTestId("workspace-insert-reference"));

    expect(onAttachSelectedWorkspaceNode).toHaveBeenCalledTimes(1);
    expect(onInsertWorkspaceReference).toHaveBeenCalledTimes(1);
  });

  it("expands a directory node and switches to preview when selecting a file node", () => {
    const onSelectWorkspaceNode = vi.fn<(path: string, type: "file" | "directory") => void>();
    const onExpandWorkspaceNode = vi.fn<(path: string) => void>();

    renderWorkspacePanel({
      onSelectWorkspaceNode,
      onExpandWorkspaceNode,
    });

    const nodes = screen.getAllByTestId("workspace-node");
    const directoryNode = nodes.find((node) => node.getAttribute("data-path") === "/repo/src");
    const fileNode = nodes.find((node) => node.getAttribute("data-path") === "/repo/main.ts");

    expect(directoryNode).toBeTruthy();
    expect(fileNode).toBeTruthy();

    fireEvent.click(directoryNode!);
    expect(onSelectWorkspaceNode).toHaveBeenCalledWith("/repo/src", "directory");
    expect(onExpandWorkspaceNode).toHaveBeenCalledWith("/repo/src");
    expect(useAgendaoStore.getState().workspacePanelTab).toBe("files");

    fireEvent.click(fileNode!);
    expect(onSelectWorkspaceNode).toHaveBeenCalledWith("/repo/main.ts", "file");
    expect(useAgendaoStore.getState().workspacePanelTab).toBe("preview");
  });
});
