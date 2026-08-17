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
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks: vi.fn<() => void>(),
        onConfigUpdated: vi.fn<() => void>(),
        onStreamReconnected: vi.fn<() => void>(),
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

  it("applies web-tier question.upsert / question.removed events to the interaction state", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    let removed = false;
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      if (!removed) {
        onEvent("message", {
          type: "question.upsert",
          sessionID: "session-1",
          question: {
            id: "q-1",
            session_id: "session-1",
            items: [
              {
                question: "Proceed with the refactor?",
                options: [{ label: "Yes" }, { label: "No" }],
                multiple: false,
              },
            ],
          },
        });
      } else {
        onEvent("message", {
          type: "question.removed",
          sessionID: "session-1",
          questionID: "q-1",
        });
      }
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
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks,
        onConfigUpdated: vi.fn<() => void>(),
        onStreamReconnected: vi.fn<() => void>(),
        queueVisibleLiveSnapshot: vi.fn<(sessionId: string, block: unknown) => void>(),
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });

    const upserted = useAgendaoStore.getState();
    expect(flushPendingOutputBlocks).toHaveBeenCalled();
    expect(upserted.question?.request_id).toBe("q-1");
    expect(upserted.question?.questions).toHaveLength(1);
    expect(upserted.streaming).toBe(false);

    // Second connection round delivers question.removed: the turn continues.
    removed = true;
    await waitFor(
      () => {
        expect(useAgendaoStore.getState().question).toBeNull();
      },
      { timeout: 5000 },
    );
    expect(useAgendaoStore.getState().streaming).toBe(true);

    unmount();
  });

  it("applies web-tier permission.upsert / permission.removed events to the interaction state", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    let removed = false;
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      if (!removed) {
        onEvent("message", {
          type: "permission.upsert",
          sessionID: "session-1",
          permission: {
            id: "perm-pty-1",
            session_id: "session-1",
            tool: "pty",
            permission_class: "dangerous_exec",
            supported_lifetimes: ["once"],
            input: {
              patterns: ["/bin/bash"],
              metadata: { command: "/bin/bash" },
            },
            message: "Start terminal command `/bin/bash`",
          },
        });
      } else {
        onEvent("message", {
          type: "permission.removed",
          sessionID: "session-1",
          permissionID: "perm-pty-1",
          reply: "once",
        });
      }
      throw abortError;
    });

    globalThis.fetch = vi.fn<typeof fetch>(async (_input, init) => {
      if (init?.signal?.aborted) {
        throw abortError;
      }
      return new Response("", { status: 200 });
    }) as typeof fetch;

    const { unmount } = renderHook(() =>
      useServerEventStream({
        applyLiveExecutionOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks: vi.fn<() => void>(),
        onConfigUpdated: vi.fn<() => void>(),
        onStreamReconnected: vi.fn<() => void>(),
        queueVisibleLiveSnapshot: vi.fn<(sessionId: string, block: unknown) => void>(),
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });

    const upserted = useAgendaoStore.getState();
    expect(upserted.permission?.permission_id).toBe("perm-pty-1");
    expect(upserted.permission?.permission).toBe("pty");
    expect(upserted.permission?.permission_class).toBe("dangerous_exec");
    expect(upserted.permission?.command).toBe("/bin/bash");
    expect(upserted.permission?.supported_lifetimes).toEqual(["once"]);

    // Second connection round delivers permission.removed for the same id.
    removed = true;
    useAgendaoStore.getState().setSessionStatusLine("session-1", "running");
    await waitFor(
      () => {
        expect(useAgendaoStore.getState().permission).toBeNull();
      },
      { timeout: 5000 },
    );

    unmount();
  });

  it("clears streaming on session.runtime.replaced idle and re-arms on running", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    let phase: "idle" | "running" = "idle";
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      onEvent("message", {
        type: "session.runtime.replaced",
        sessionID: "session-1",
        runtime: {
          session_id: "session-1",
          run_status: phase,
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

    useAgendaoStore.setState({ streaming: true, statusLine: "running" });

    const { unmount } = renderHook(() =>
      useServerEventStream({
        applyLiveExecutionOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks: vi.fn<() => void>(),
        onConfigUpdated: vi.fn<() => void>(),
        onStreamReconnected: vi.fn<() => void>(),
        queueVisibleLiveSnapshot: vi.fn<(sessionId: string, block: unknown) => void>(),
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });
    // statusLine is overwritten by the reconnect cycle after the mocked
    // stream aborts; the streaming flag is the stable assertion here.
    expect(useAgendaoStore.getState().streaming).toBe(false);

    // Reconnect round delivers a running snapshot: streaming re-arms.
    phase = "running";
    await waitFor(
      () => {
        expect(useAgendaoStore.getState().streaming).toBe(true);
      },
      { timeout: 5000 },
    );

    unmount();
  });

  it("keeps runtime state per session: another session's events do not leak into the selected one", async () => {
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    const parseSSESpy = vi.spyOn(apiModule, "parseSSE").mockImplementation(async (_response, onEvent) => {
      // session-2 finishes while session-1 (selected) is running.
      onEvent("message", {
        type: "session.runtime.replaced",
        sessionID: "session-2",
        runtime: { session_id: "session-2", run_status: "idle" },
      });
      // session-2 raises a permission request; it must not hijack the
      // selected session's overlay…
      onEvent("message", {
        type: "permission.upsert",
        sessionID: "session-2",
        permission: {
          id: "perm-s2",
          session_id: "session-2",
          tool: "bash",
          permission_class: "dangerous_exec",
          supported_lifetimes: ["once"],
          input: { patterns: ["/bin/rm"], metadata: { command: "/bin/rm -rf /tmp/x" } },
          message: "Run `/bin/rm -rf /tmp/x`",
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

    useAgendaoStore.setState({ streaming: true, statusLine: "running" });

    const { unmount } = renderHook(() =>
      useServerEventStream({
        applyLiveExecutionOutputBlock: vi.fn<(block: unknown, sessionId: string) => void>(),
        clearPendingOutputBlockFlush: vi.fn<() => void>(),
        clearPendingSessionRefresh: vi.fn<() => void>(),
        flushPendingOutputBlocks: vi.fn<() => void>(),
        onConfigUpdated: vi.fn<() => void>(),
        onStreamReconnected: vi.fn<() => void>(),
        queueVisibleLiveSnapshot: vi.fn<(sessionId: string, block: unknown) => void>(),
        refreshExecutionActivity: vi.fn<(sessionId: string) => void>(),
        scheduleSessionRefresh: vi.fn<() => void>(),
      }),
    );

    await waitFor(() => {
      expect(parseSSESpy).toHaveBeenCalled();
    });

    const state = useAgendaoStore.getState();
    // session-1 keeps its own running state and shows no overlay…
    expect(state.streaming).toBe(true);
    expect(state.permission).toBeNull();
    // …while session-2's view records idle + its pending permission.
    expect(state.runtimeViews["session-2"]?.streaming).toBe(false);
    expect(state.runtimeViews["session-2"]?.permission?.permission_id).toBe("perm-s2");

    // Switching to session-2 restores its view, overlay included; the
    // pending permission correctly marks it as awaiting the user.
    useAgendaoStore.getState().selectSession("session-2");
    const switched = useAgendaoStore.getState();
    expect(switched.streaming).toBe(false);
    expect(switched.statusLine).toBe("awaiting_user");
    expect(switched.permission?.permission_id).toBe("perm-s2");

    unmount();
  });
});
