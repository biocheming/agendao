import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useServerEventStream } from "./useServerEventStream";
import { useAgendaoStore } from "../store";
import { resetAgendaoStore } from "../test/store-test-utils";
import * as apiModule from "../lib/api";

describe("useServerEventStream", () => {
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    resetAgendaoStore();
    useAgendaoStore.setState({ selectedSessionId: "session-1" });
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  it("routes output_block events into runtime surface, transcript queue, and activity hooks", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      onEvent("message", {
        type: "output_block",
        session_id: "session-1",
        block: {
          kind: "status",
          text: "Running",
        },
      });
      onEvent("message", {
        type: "output_block",
        session_id: "session-1",
        block: {
          kind: "session_event",
          id: "evt-1",
          text: "Session event",
        },
      });
      onEvent("message", {
        type: "output_block",
        session_id: "session-1",
        block: {
          kind: "tool",
          id: "tool-event-1",
          text: "Tool output",
          live_identity: {
            message_id: "msg-tool",
            part_key: "tool_result/tool-1",
            part_kind: "tool_result",
            phase: "snapshot",
          },
        },
      });
      onEvent("message", {
        type: "output_block",
        session_id: "session-1",
        block: {
          kind: "message",
          text: "Hello from stream",
          live_identity: {
            message_id: "msg-1",
            part_key: "text/main",
            part_kind: "assistant_text",
            phase: "snapshot",
          },
        },
      });
      throw abortError;
    });

    globalThis.fetch = vi.fn<typeof fetch>(async (_input, init) => {
      if (init?.signal?.aborted) {
        throw abortError;
      }
      return new Response("", { status: 200 });
    }) as typeof fetch;

    const applyLiveExecutionOutputBlock = vi.fn<(block: unknown, sessionId: string) => void>();
    const queueVisibleLiveSnapshot = vi.fn<(sessionId: string, block: unknown) => void>();

    const { unmount } = renderHook(() =>
      useServerEventStream({
        applyLiveExecutionOutputBlock,
        applySchedulerStageOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks: vi.fn<() => void>(),
        onConfigUpdated: vi.fn<() => void>(),
        queueVisibleLiveSnapshot,
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });

    const state = useAgendaoStore.getState();
    expect(state.currentRuntimeSurfaceFor("session-1").banner).toBe("Running");
    expect(state.currentRuntimeSurfaceFor("session-1").sessionEvents).toHaveLength(1);
    expect(applyLiveExecutionOutputBlock).toHaveBeenCalledTimes(1);
    expect(queueVisibleLiveSnapshot).toHaveBeenCalledTimes(2);

    unmount();
  });

  it("handles error and permission lifecycle events by updating streaming state and interaction state", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      onEvent("message", {
        type: "permission.requested",
        session_id: "session-1",
        permissionID: "perm-1",
        info: {
          message: "Need approval",
          permission_class: "workspace_write",
        },
      });
      onEvent("message", {
        type: "permission.resolved",
        permissionID: "perm-1",
      });
      onEvent("message", {
        type: "error",
        session_id: "session-1",
        error: "boom",
      });
      throw abortError;
    });

    globalThis.fetch = vi.fn<typeof fetch>(async (_input, init) => {
      if (init?.signal?.aborted) {
        throw abortError;
      }
      return new Response("", { status: 200 });
    }) as typeof fetch;

    const flushPendingOutputBlocks = vi.fn<() => void>();

    const { unmount } = renderHook(() =>
      useServerEventStream({
        applyLiveExecutionOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        applySchedulerStageOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks,
        onConfigUpdated: vi.fn<() => void>(),
        queueVisibleLiveSnapshot: vi.fn<(sessionId: string, block: unknown) => void>(),
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });

    const state = useAgendaoStore.getState();
    expect(flushPendingOutputBlocks).toHaveBeenCalled();
    expect(state.permission).toBeNull();
    expect(state.permissionSubmitCompletedAt).not.toBeNull();
    expect(state.latestRuntimeError).toBe("boom");
    expect(state.streaming).toBe(false);
    expect(state.statusLine).toBe("reconnecting");
    expect(state.permissionSubmitting).toBe(false);

    unmount();
  });
});
