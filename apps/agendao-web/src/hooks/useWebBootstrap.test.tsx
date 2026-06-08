import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWebBootstrap } from "./useWebBootstrap";
import { resetAgendaoStore, resetBrowserRoute } from "../test/store-test-utils";
import { useAgendaoStore } from "../store";
import type { SessionListResponseRecord } from "../lib/session";
import type { PathsResponseRecord, WorkspaceContextRecord } from "../lib/workspace";

function emptySessionList(items: SessionListResponseRecord["items"] = []): SessionListResponseRecord {
  return {
    items,
    contract: {
      filter_query_parameters: [],
      search_fields: [],
      non_search_fields: [],
      note: "",
    },
  };
}

describe("useWebBootstrap", () => {
  beforeEach(() => {
    resetAgendaoStore();
    resetBrowserRoute();
  });

  it("keeps session bootstrap alive when config surface loading fails", async () => {
    const preferencesReadyRef = { current: false };
    const sessionId = "session-life";

    async function apiJsonImpl<T>(path: string): Promise<T> {
      if (path === "/session?limit=500") {
        return emptySessionList([
          {
            id: sessionId,
            title: "Life root",
            directory: "/repo/life",
            updated: 42,
          },
        ]) as T;
      }
      if (path === "/path") {
        return {
          cwd: "/repo/life",
          home: "/home/test",
          config: "/home/test/.config",
          data: "/home/test/.local/share",
        } satisfies PathsResponseRecord as T;
      }
      if (path === "/workspace/context") {
        return {
          identity: {
            requested_dir: "/repo/life",
            workspace_root: "/repo/life",
            config_dir: "/repo/life/.agendao",
            workspace_key: "/repo/life",
          },
          mode: "isolated",
          config: {},
        } satisfies WorkspaceContextRecord as T;
      }
      if (path === "/config/providers") {
        return { providers: [] } as T;
      }
      if (path === "/provider/connect/schema") {
        return { providers: [], protocols: [] } as T;
      }
      if (path === "/mode") {
        throw new Error("mode failed");
      }
      throw new Error(`Unexpected path ${path}`);
    }

    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    renderHook(() =>
      useWebBootstrap({
        apiJson,
        formatError: (error) => (error instanceof Error ? error.message : "Unknown error"),
        preferencesReadyRef,
        provisionExternalAdapterSession: vi.fn(async () => sessionId),
      }),
    );

    await waitFor(() => {
      const state = useAgendaoStore.getState();
      expect(apiJson).toHaveBeenCalledWith("/session?limit=500");
      expect(state.sessions.some((session) => session.id === sessionId)).toBe(true);
      expect(state.selectedSessionId).toBe(sessionId);
      expect(state.serviceRootPath).toBe("/repo/life");
      expect(state.banner).toContain("Config surface degraded:");
      expect(preferencesReadyRef.current).toBe(true);
    });
  });
});
