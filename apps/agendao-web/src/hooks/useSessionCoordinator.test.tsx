import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionCoordinator } from "./useSessionCoordinator";
import { useAgendaoStore } from "../store";
import { resetAgendaoStore, resetBrowserRoute } from "../test/store-test-utils";
import type { SessionRecord } from "../lib/session";
import type { SessionListResponseRecord } from "../lib/session";

function createEmptySessionListResponse(): SessionListResponseRecord {
  return {
    items: [],
    contract: {
      filter_query_parameters: [],
      search_fields: [],
      non_search_fields: [],
      note: "",
    },
  };
}

async function defaultApiJson<T>(): Promise<T> {
  return createEmptySessionListResponse() as T;
}

function renderSessionCoordinator(
  overrides: Partial<Parameters<typeof useSessionCoordinator>[0]> = {},
) {
  return renderHook(() => {
    const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
    return useSessionCoordinator({
      api: vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(async () => new Response(null, { status: 200 })),
      apiJson: vi.fn<typeof defaultApiJson>(defaultApiJson) as unknown as <T>(
        path: string,
        options?: RequestInit,
      ) => Promise<T>,
      currentWorkspacePath: "/repo",
      currentWorkspaceSummaryPath: null,
      formatError: (error) => (error instanceof Error ? error.message : "Unknown error"),
      selectedSessionId,
      serviceRootPath: "/repo",
      workspaceContextRootPath: null,
      ...overrides,
    });
  });
}

describe("useSessionCoordinator", () => {
  beforeEach(() => {
    resetAgendaoStore();
    resetBrowserRoute();
  });

  it("creates a session optimistically, then replaces it with the server session and syncs route", async () => {
    const createdSession: SessionRecord = {
      id: "session-real",
      title: "Real session",
      directory: "/repo/new",
      updated: 50,
    };
    async function apiJsonImpl<T>(path: string): Promise<T> {
      if (path === "/session") return createdSession as T;
      if (path === "/session?limit=500") {
        return createEmptySessionListResponse() as T;
      }
      throw new Error(`Unexpected path ${path}`);
    }
    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    const { result } = renderSessionCoordinator({ apiJson });

    let createdId = "";
    await act(async () => {
      createdId = await result.current.createSession({ title: "Draft", directory: "/repo/new" });
    });

    const state = useAgendaoStore.getState();
    expect(createdId).toBe("session-real");
    expect(state.selectedSessionId).toBe("session-real");
    expect(state.currentWorkspacePath).toBe("/repo/new");
    expect(state.sessions).toHaveLength(1);
    expect(state.sessions[0]?.id).toBe("session-real");
    expect(state.sessions[0]?.title).toBe("Real session");
    expect(window.location.search).toContain("session=session-real");
  });

  it("rolls back optimistic session on create failure", async () => {
    useAgendaoStore.setState({
      selectedSessionId: "existing",
      sessions: [{ id: "existing", title: "Existing", directory: "/repo", updated: 10 } as SessionRecord],
    });

    async function apiJsonImpl<T>(path: string): Promise<T> {
      if (path === "/session") throw new Error("create failed");
      if (path === "/session?limit=500") {
        return createEmptySessionListResponse() as T;
      }
      throw new Error(`Unexpected path ${path}`);
    }
    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    const { result } = renderSessionCoordinator({ apiJson });

    await act(async () => {
      await expect(
        result.current.createSession({ title: "Will fail", directory: "/repo/fail" }),
      ).rejects.toThrow("create failed");
    });

    const state = useAgendaoStore.getState();
    expect(state.selectedSessionId).toBe("existing");
    expect(state.sessions).toHaveLength(1);
    expect(state.sessions[0]?.id).toBe("existing");
  });

  it("deletes only root selections and falls back to the newest remaining workspace root session", async () => {
    useAgendaoStore.setState({
      sessions: [
        { id: "root-1", title: "Root 1", directory: "/repo", updated: 100 } as SessionRecord,
        { id: "child-1", title: "Child 1", directory: "/repo", updated: 90, parent_id: "root-1" } as SessionRecord,
        { id: "root-2", title: "Root 2", directory: "/repo", updated: 80 } as SessionRecord,
      ],
      selectedSessionId: "root-1",
      currentWorkspacePath: "/repo",
    });

    const api = vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(async () => new Response(null, { status: 200 }));
    async function apiJsonImpl<T>(path: string): Promise<T> {
      if (path === "/session?limit=500") {
        return {
          items: [{ id: "root-2", title: "Root 2", directory: "/repo", updated: 80 }],
          contract: createEmptySessionListResponse().contract,
        } as T;
      }
      throw new Error(`Unexpected path ${path}`);
    }
    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    const { result } = renderSessionCoordinator({
      api,
      apiJson,
      currentWorkspaceSummaryPath: "/repo",
    });

    await act(async () => {
      await result.current.deleteSelectedSessions(["root-1", "child-1"]);
    });

    expect(api).toHaveBeenCalledTimes(1);
    expect(api).toHaveBeenCalledWith("/session/root-1", { method: "DELETE" });
    const state = useAgendaoStore.getState();
    expect(state.selectedSessionId).toBe("root-2");
    expect(state.banner).toBe("Deleted 1 session.");
    expect(state.deletingSessions).toBe(false);
  });
});
