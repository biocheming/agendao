import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTranscriptFeedState } from "./useTranscriptFeedState";
import { useAgendaoStore } from "../store";
import {
  ASSISTANT_TEXT_MAIN_PART_KEY,
  ASSISTANT_REASONING_MAIN_PART_KEY,
} from "../lib/liveIdentity";
import { resetAgendaoStore } from "../test/store-test-utils";

describe("useTranscriptFeedState", () => {
  let requestAnimationFrameSpy: ReturnType<typeof vi.spyOn>;
  let cancelAnimationFrameSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    resetAgendaoStore();
    requestAnimationFrameSpy = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback: FrameRequestCallback) => {
        callback(0);
        return 1;
      });
    cancelAnimationFrameSpy = vi
      .spyOn(window, "cancelAnimationFrame")
      .mockImplementation(() => undefined);
  });

  afterEach(() => {
    requestAnimationFrameSpy.mockRestore();
    cancelAnimationFrameSpy.mockRestore();
  });

  it("queues visible snapshots only for the selected session and flushes them into store messages", () => {
    useAgendaoStore.setState({ selectedSessionId: "session-a" });

    const { result } = renderHook(() =>
      useTranscriptFeedState({
        maxPendingOutputBlocks: 4,
        sessionIds: ["session-a", "session-b"],
        showThinking: true,
      }),
    );

    act(() => {
      result.current.queueVisibleLiveSnapshot("session-b", {
        kind: "message",
        phase: "full",
        text: "hidden",
        live_identity: {
          message_id: "msg-hidden",
          part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
          part_kind: "assistant_text",
          phase: "snapshot",
        },
      });
    });

    expect(useAgendaoStore.getState().messages).toEqual([]);
    expect(result.current.pendingOutputBlocksRef.current["session-b"]).toBeUndefined();

    act(() => {
      result.current.queueVisibleLiveSnapshot("session-a", {
        kind: "message",
        phase: "delta",
        text: "Hello",
        live_identity: {
          message_id: "msg-1",
          part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
          part_kind: "assistant_text",
          phase: "delta",
        },
      });
    });

    const messages = useAgendaoStore.getState().messages;
    expect(messages).toHaveLength(1);
    expect(messages[0]?.kind).toBe("message");
    expect(messages[0]?.text).toBe("Hello");
    expect(result.current.pendingOutputBlocksRef.current["session-a"]).toBeUndefined();
  });

  it("rebuilds feed from history, prunes acknowledged optimistic messages, and clears transcript feed", () => {
    const { result } = renderHook(() =>
      useTranscriptFeedState({
        maxPendingOutputBlocks: 4,
        sessionIds: ["session-a"],
        showThinking: true,
      }),
    );

    act(() => {
      result.current.optimisticMessagesRef.current["session-a"] = [
        {
          kind: "message",
          phase: "full",
          role: "user",
          id: "optimistic-1",
          feedId: "feed-opt-1",
          anchorId: "feed-opt-1",
          text: "repeat me",
        },
        {
          kind: "message",
          phase: "full",
          role: "user",
          id: "optimistic-2",
          feedId: "feed-opt-2",
          anchorId: "feed-opt-2",
          text: "still pending",
        },
      ];
      result.current.rebuildFeedFromHistory({
        sessionId: "session-a",
        streaming: false,
        history: [
          {
            id: "history-1",
            role: "user",
            parts: [{ id: "part-1", type: "text", text: "repeat me" }],
          },
          {
            id: "history-2",
            role: "assistant",
            parts: [{ id: "part-2", type: "reasoning", text: "thinking aloud" }],
          },
        ],
      });
    });

    const state = useAgendaoStore.getState();
    expect(state.messageHistory).toHaveLength(2);
    expect(state.messages.map((message) => message.text)).toEqual([
      "repeat me",
      "thinking aloud",
      "still pending",
    ]);
    expect(result.current.optimisticMessagesRef.current["session-a"]).toHaveLength(1);
    expect(result.current.optimisticMessagesRef.current["session-a"]?.[0]?.text).toBe("still pending");

    act(() => {
      result.current.clearTranscriptFeed();
    });

    expect(useAgendaoStore.getState().messages).toEqual([]);
    expect(useAgendaoStore.getState().messageHistory).toEqual([]);
  });

  it("prunes removed sessions from live and optimistic caches", () => {
    const { result, rerender } = renderHook(
      ({ sessionIds }) =>
        useTranscriptFeedState({
          maxPendingOutputBlocks: 4,
          sessionIds,
          showThinking: true,
        }),
      { initialProps: { sessionIds: ["keep-a", "drop-b"] } },
    );

    act(() => {
      result.current.liveBlocksRef.current = {
        "keep-a": [
          {
            kind: "reasoning",
            phase: "full",
            text: "keep",
            live_identity: {
              message_id: "keep-msg",
              part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
              part_kind: "assistant_reasoning",
              phase: "snapshot",
            },
          },
        ],
        "drop-b": [
          {
            kind: "message",
            phase: "full",
            text: "drop",
            live_identity: {
              message_id: "drop-msg",
              part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
              part_kind: "assistant_text",
              phase: "snapshot",
            },
          },
        ],
      };
      result.current.optimisticMessagesRef.current = {
        "keep-a": [],
        "drop-b": [
          {
            kind: "message",
            phase: "full",
            role: "user",
            id: "opt-drop",
            feedId: "opt-drop",
            anchorId: "opt-drop",
            text: "drop",
          },
        ],
      };
      result.current.pendingOutputBlocksRef.current = {
        "keep-a": [],
        "drop-b": [],
      };
    });

    rerender({ sessionIds: ["keep-a"] });

    expect(Object.keys(result.current.liveBlocksRef.current)).toEqual(["keep-a"]);
    expect(Object.keys(result.current.optimisticMessagesRef.current)).toEqual(["keep-a"]);
    expect(Object.keys(result.current.pendingOutputBlocksRef.current)).toEqual(["keep-a"]);
  });
});
