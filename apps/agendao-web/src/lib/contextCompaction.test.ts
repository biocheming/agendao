import { describe, expect, it } from "vitest";
import {
  buildSyntheticCompactionFeedMessage,
  withSyntheticCompactionMessage,
} from "./contextCompaction";
import type { FeedMessage } from "./history";

describe("context compaction transcript helpers", () => {
  it("builds a synthetic compaction feed message when runtime is compacting", () => {
    const message = buildSyntheticCompactionFeedMessage({
      sessionId: "ses_1",
      runStatus: "compacting",
      summary: {
        trigger: "auto",
        phase: "pre_request",
        reason: "context_pressure",
        forced: false,
        request_context_tokens: 58_000,
        live_context_tokens: 58_000,
        limit_tokens: 100_000,
        body_chars: null,
        message_count_before: 968,
        compacted_message_count: null,
        kept_message_count: null,
        summary: null,
      },
      lifecycle: {
        trigger: "auto",
        phase: "pre_request",
        reason: "context_pressure",
        status: "started",
        forced: false,
        request_context_tokens: 58_000,
        live_context_tokens: 58_000,
        limit_tokens: 100_000,
        body_chars: null,
        installed: null,
      },
    });

    expect(message).not.toBeNull();
    expect(message?.title).toBe("Compacting conversation");
    expect(message?.summary).toContain("compressing 968 messages");
    expect(message?.summary).toContain("58K");
    expect(message?.text).toContain("context pressure");
    expect(message?.text).toContain("58%");
  });

  it("appends a single synthetic compaction message to the visible timeline", () => {
    const base: FeedMessage[] = [
      {
        kind: "message",
        role: "user",
        id: "msg_1",
        feedId: "feed_1",
        anchorId: "msg_1",
        text: "hello",
      },
    ];

    const timeline = withSyntheticCompactionMessage(base, {
      sessionId: "ses_1",
      runStatus: "compacting",
      summary: {
        trigger: "auto",
        phase: "pre_request",
        reason: "context_pressure",
        forced: false,
        request_context_tokens: 58_000,
        live_context_tokens: 58_000,
        limit_tokens: 100_000,
        body_chars: null,
        message_count_before: null,
        compacted_message_count: null,
        kept_message_count: null,
        summary: null,
      },
      lifecycle: {
        trigger: "auto",
        phase: "pre_request",
        reason: "context_pressure",
        status: "started",
        forced: false,
        request_context_tokens: 58_000,
        live_context_tokens: 58_000,
        limit_tokens: 100_000,
        body_chars: null,
        installed: null,
      },
    });

    expect(timeline).toHaveLength(2);
    expect(timeline[1]?.kind).toBe("status");
    expect(timeline[1]?.feedId).toBe("__compaction__:ses_1");
  });
});
