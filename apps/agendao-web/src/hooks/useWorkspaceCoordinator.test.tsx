import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceCoordinator } from "./useWorkspaceCoordinator";
import { useAgendaoStore } from "../store";
import { resetAgendaoStore } from "../test/store-test-utils";
import type { FileTreeNodeRecord } from "../lib/workspace";

function createApiJsonStub(
  impl?: (path: string, options?: RequestInit) => Promise<unknown>,
): <T>(path: string, options?: RequestInit) => Promise<T> {
  return vi.fn(async (path: string, options?: RequestInit) => {
    if (!impl) {
      return undefined;
    }
    return impl(path, options);
  }) as unknown as <T>(path: string, options?: RequestInit) => Promise<T>;
}

describe("useWorkspaceCoordinator", () => {
  beforeEach(() => {
    resetAgendaoStore();
    useAgendaoStore.setState({
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
      workspaceRootPath: "/repo",
      selectedWorkspacePath: "/repo",
      selectedWorkspaceType: "directory",
    });
  });

  it("selects a directory node, switches to files tab, and lazily loads its children", async () => {
    const apiJson = createApiJsonStub(async (path: string) => {
      if (path === "/file/tree?path=%2Frepo%2Fsrc&depth=1") {
        return {
          name: "src",
          path: "/repo/src",
          type: "directory",
          hasChildren: true,
          childrenLoaded: true,
          children: [
            {
              name: "index.ts",
              path: "/repo/src/index.ts",
              type: "file",
              children: [],
            },
          ],
        } as FileTreeNodeRecord;
      }
      throw new Error(`Unexpected path ${path}`);
    });

    const { result } = renderHook(() =>
      useWorkspaceCoordinator({
        api: vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(),
        apiJson,
        currentSessionDirectory: "/repo",
        currentWorkspaceSummaryPath: "/repo",
        formatError: (error) => (error instanceof Error ? error.message : "Unknown error"),
        messageHistory: [],
        selectedSessionId: "session-1",
        serviceRootPath: "/repo",
        workspaceContext: null,
      }),
    );

    let selected = false;
    await act(async () => {
      selected = result.current.selectWorkspaceNode("/repo/src", "directory");
      await Promise.resolve();
    });

    expect(selected).toBe(true);
    const state = useAgendaoStore.getState();
    expect(state.selectedWorkspacePath).toBe("/repo/src");
    expect(state.selectedWorkspaceType).toBe("directory");
    expect(state.selectedFilePath).toBeNull();
    expect(state.workspacePanelTab).toBe("files");
    expect(apiJson).toHaveBeenCalledWith("/file/tree?path=%2Frepo%2Fsrc&depth=1");
    const srcNode = state.fileTree?.children?.find((node) => node.path === "/repo/src");
    expect(srcNode?.childrenLoaded).toBe(true);
    expect(srcNode?.children?.[0]?.path).toBe("/repo/src/index.ts");
  });

  it("attaches the selected workspace node once and focuses the existing attachment on repeat", () => {
    useAgendaoStore.setState({
      selectedWorkspacePath: "/repo/main.ts",
      selectedWorkspaceType: "file",
      attachments: [],
      selectedAttachmentIndex: null,
    });

    const { result } = renderHook(() =>
      useWorkspaceCoordinator({
        api: vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(),
        apiJson: createApiJsonStub(),
        currentSessionDirectory: "/repo",
        currentWorkspaceSummaryPath: "/repo",
        formatError: (error) => (error instanceof Error ? error.message : "Unknown error"),
        messageHistory: [],
        selectedSessionId: "session-1",
        serviceRootPath: "/repo",
        workspaceContext: null,
      }),
    );

    act(() => {
      result.current.attachSelectedWorkspaceNode();
    });

    let state = useAgendaoStore.getState();
    expect(state.attachments).toHaveLength(1);
    expect(state.selectedAttachmentIndex).toBe(0);
    expect(state.banner).toBe("Attached file main.ts");

    act(() => {
      result.current.attachSelectedWorkspaceNode();
    });

    state = useAgendaoStore.getState();
    expect(state.attachments).toHaveLength(1);
    expect(state.selectedAttachmentIndex).toBe(0);
  });
});
