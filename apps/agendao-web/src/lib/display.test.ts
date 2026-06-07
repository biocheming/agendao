import { describe, expect, it } from "vitest";
import {
  formatError,
  promptPreviewText,
  findLastMessage,
  metadataValue,
  resolveActiveModelRef,
  shellQuoteCommandValue,
  splitRepeatableAnswer,
  normalizedAnswerValues,
  modeKey,
  splitRecentModelRef,
  pushRecentModel,
  mergePendingCommandArguments,
  runtimeSurfacePreview,
  runtimeSurfaceLabel,
  runtimeSurfacePhase,
  runtimeSurfaceDebugDetail,
  ingressStabilizationLabel,
  readRuntimeBudgetNumber,
} from "./display";
import type { FeedMessage, OutputBlock, RuntimeSurfaceOutputBlock } from "./history";
import type { SessionRecord } from "./session";
import type { ExecutionMode } from "./webRuntime";
import type { PendingCommandInvocation } from "./display";

// ============================================================
// formatError
// ============================================================

describe("formatError", () => {
  it("returns the message from Error instances", () => {
    expect(formatError(new Error("something went wrong"))).toBe("something went wrong");
  });

  it('returns "Unknown error" for strings', () => {
    expect(formatError("plain string")).toBe("Unknown error");
  });

  it('returns "Unknown error" for null', () => {
    expect(formatError(null)).toBe("Unknown error");
  });

  it('returns "Unknown error" for undefined', () => {
    expect(formatError(undefined)).toBe("Unknown error");
  });

  it('returns "Unknown error" for objects', () => {
    expect(formatError({ code: 500 })).toBe("Unknown error");
  });
});

// ============================================================
// promptPreviewText
// ============================================================

describe("promptPreviewText", () => {
  it("returns trimmed content when text is non-empty", () => {
    expect(promptPreviewText("  hello world  ", [])).toBe("hello world");
  });

  it("returns empty string when no text and no attachments", () => {
    expect(promptPreviewText("", [])).toBe("");
  });

  it('returns "[1 attachment]" for single non-text attachment', () => {
    expect(promptPreviewText("", [{ type: "file", url: "file://test.txt" }])).toBe(
      "[1 attachment]",
    );
  });

  it('returns "[2 attachments]" for multiple non-text attachments', () => {
    expect(
      promptPreviewText("", [
        { type: "file", url: "file://a.txt" },
        { type: "file", url: "file://b.txt" },
      ]),
    ).toBe("[2 attachments]");
  });

  it("ignores text-type parts in attachment count", () => {
    expect(
      promptPreviewText("", [
        { type: "text", text: "hello" },
        { type: "file", url: "file://a.txt" },
      ]),
    ).toBe("[1 attachment]");
  });
});

// ============================================================
// findLastMessage
// ============================================================

describe("findLastMessage", () => {
  function makeMsg(role: string, kind: string): FeedMessage {
    return { kind: kind as FeedMessage["kind"], role: role as FeedMessage["role"] } as FeedMessage;
  }

  it("finds the last matching message", () => {
    const msgs: FeedMessage[] = [
      makeMsg("user", "message"),
      makeMsg("assistant", "message"),
      makeMsg("user", "message"),
    ];
    const result = findLastMessage(msgs, (m) => m.role === "user");
    expect(result).toBe(msgs[2]);
  });

  it("returns null when no match", () => {
    const msgs: FeedMessage[] = [makeMsg("assistant", "message")];
    expect(findLastMessage(msgs, (m) => m.role === "user")).toBeNull();
  });

  it("returns null for empty array", () => {
    expect(findLastMessage([], () => true)).toBeNull();
  });
});

// ============================================================
// metadataValue
// ============================================================

describe("metadataValue", () => {
  it("returns a flat key value", () => {
    expect(metadataValue({ foo: "bar" }, "foo")).toBe("bar");
  });

  it("traverses dotted keys", () => {
    expect(metadataValue({ a: { b: { c: 42 } } }, "a.b.c")).toBe(42);
  });

  it("returns undefined for missing key", () => {
    expect(metadataValue({ foo: "bar" }, "baz")).toBeUndefined();
  });

  it("returns undefined for null metadata", () => {
    expect(metadataValue(null, "foo")).toBeUndefined();
  });

  it("returns undefined for undefined metadata", () => {
    expect(metadataValue(undefined, "foo")).toBeUndefined();
  });

  it("returns undefined when intermediate path is not an object", () => {
    expect(metadataValue({ a: 1 }, "a.b")).toBeUndefined();
  });
});

// ============================================================
// resolveActiveModelRef
// ============================================================

describe("resolveActiveModelRef", () => {
  it("returns explicit selectedModel", () => {
    expect(resolveActiveModelRef(null, "openai/gpt-4")).toBe("openai/gpt-4");
  });

  it("returns session hint when no explicit model", () => {
    const session: SessionRecord = {
      id: "s1",
      hints: { current_model: "anthropic/claude" },
    } as SessionRecord;
    expect(resolveActiveModelRef(session, "")).toBe("anthropic/claude");
  });

  it("returns provider/model from session hints when no current_model", () => {
    const session: SessionRecord = {
      id: "s1",
      hints: { model_provider: "openai", model_id: "gpt-4" },
    } as SessionRecord;
    expect(resolveActiveModelRef(session, "")).toBe("openai/gpt-4");
  });

  it("returns null when no model can be resolved", () => {
    expect(resolveActiveModelRef(null, "")).toBeNull();
  });

  it("trims whitespace from selectedModel", () => {
    expect(resolveActiveModelRef(null, "  openai/gpt-4  ")).toBe("openai/gpt-4");
  });
});

// ============================================================
// shellQuoteCommandValue
// ============================================================

describe("shellQuoteCommandValue", () => {
  it("returns safe values without quotes", () => {
    expect(shellQuoteCommandValue("hello")).toBe("hello");
    expect(shellQuoteCommandValue("path/to/file.txt")).toBe("path/to/file.txt");
    expect(shellQuoteCommandValue("abc-123_*.test")).toBe("abc-123_*.test");
  });

  it("quotes values with spaces", () => {
    expect(shellQuoteCommandValue("hello world")).toBe('"hello world"');
  });

  it('returns "" for empty string', () => {
    expect(shellQuoteCommandValue("")).toBe('""');
  });

  it("escapes double quotes and backslashes inside quoted values", () => {
    expect(shellQuoteCommandValue('say "hi"')).toBe('"say \\"hi\\""');
  });
});

// ============================================================
// splitRepeatableAnswer
// ============================================================

describe("splitRepeatableAnswer", () => {
  it("splits by newline", () => {
    expect(splitRepeatableAnswer("a\nb\nc")).toEqual(["a", "b", "c"]);
  });

  it("splits by comma", () => {
    expect(splitRepeatableAnswer("a,b,c")).toEqual(["a", "b", "c"]);
  });

  it("splits by whitespace", () => {
    expect(splitRepeatableAnswer("a b\tc")).toEqual(["a", "b", "c"]);
  });

  it("filters empty values", () => {
    expect(splitRepeatableAnswer("a,,b")).toEqual(["a", "b"]);
  });
});

// ============================================================
// normalizedAnswerValues
// ============================================================

describe("normalizedAnswerValues", () => {
  it("returns trimmed array values", () => {
    expect(normalizedAnswerValues([" a ", " b "], false)).toEqual(["a", "b"]);
  });

  it("returns single value array for simple string", () => {
    expect(normalizedAnswerValues("hello", false)).toEqual(["hello"]);
  });

  it("splits multi-line when multiple=true", () => {
    expect(normalizedAnswerValues("a\nb", true)).toEqual(["a", "b"]);
  });

  it("returns empty array for empty string", () => {
    expect(normalizedAnswerValues("", false)).toEqual([]);
  });

  it("returns empty array for undefined", () => {
    expect(normalizedAnswerValues(undefined, false)).toEqual([]);
  });
});

// ============================================================
// modeKey
// ============================================================

describe("modeKey", () => {
  it("formats agent mode", () => {
    const mode: ExecutionMode = { kind: "agent", id: "coder", name: "Coder" };
    expect(modeKey(mode)).toBe("agent:coder");
  });

  it("formats preset mode", () => {
    const mode: ExecutionMode = { kind: "preset", id: "fast", name: "Fast" };
    expect(modeKey(mode)).toBe("preset:fast");
  });
});

// ============================================================
// splitRecentModelRef
// ============================================================

describe("splitRecentModelRef", () => {
  it("splits provider/model", () => {
    expect(splitRecentModelRef("openai/gpt-4")).toEqual({
      provider: "openai",
      model: "gpt-4",
    });
  });

  it("returns null for missing separator", () => {
    expect(splitRecentModelRef("gpt-4")).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(splitRecentModelRef("")).toBeNull();
  });

  it("returns null when provider is empty", () => {
    expect(splitRecentModelRef("/gpt-4")).toBeNull();
  });

  it("returns null when model is empty", () => {
    expect(splitRecentModelRef("openai/")).toBeNull();
  });
});

// ============================================================
// pushRecentModel
// ============================================================

describe("pushRecentModel", () => {
  it("adds a new model to the front", () => {
    const result = pushRecentModel([], "openai/gpt-4");
    expect(result).toEqual([{ provider: "openai", model: "gpt-4" }]);
  });

  it("deduplicates by case-insensitive match", () => {
    const result = pushRecentModel(
      [{ provider: "openai", model: "gpt-4" }],
      "OpenAI/gpt-4",
    );
    expect(result).toEqual([{ provider: "OpenAI", model: "gpt-4" }]);
  });

  it("keeps at most 5 entries", () => {
    const existing = [
      { provider: "p1", model: "m1" },
      { provider: "p2", model: "m2" },
      { provider: "p3", model: "m3" },
      { provider: "p4", model: "m4" },
      { provider: "p5", model: "m5" },
    ];
    const result = pushRecentModel(existing, "p6/m6");
    expect(result).toHaveLength(5);
    expect(result[0]).toEqual({ provider: "p6", model: "m6" });
  });

  it("returns unchanged list for invalid modelRef", () => {
    const result = pushRecentModel(
      [{ provider: "openai", model: "gpt-4" }],
      "",
    );
    expect(result).toEqual([{ provider: "openai", model: "gpt-4" }]);
  });
});

// ============================================================
// mergePendingCommandArguments
// ============================================================

describe("mergePendingCommandArguments", () => {
  it("merges raw arguments with missing fields", () => {
    const pending: PendingCommandInvocation = {
      command: "deploy",
      rawArguments: "--env prod",
      missingFields: ["region"],
    } as PendingCommandInvocation;
    const result = mergePendingCommandArguments(pending, [["us-east-1"]]);
    expect(result).toBe("--env prod --region us-east-1");
  });

  it("handles multiple values for a field", () => {
    const pending: PendingCommandInvocation = {
      command: "test",
      missingFields: ["tags"],
    } as PendingCommandInvocation;
    const result = mergePendingCommandArguments(pending, [["fast", "smoke"]]);
    expect(result).toBe("--tags fast smoke");
  });

  it("returns empty string when nothing to merge", () => {
    const pending: PendingCommandInvocation = {
      command: "test",
    } as PendingCommandInvocation;
    expect(mergePendingCommandArguments(pending, [])).toBe("");
  });
});

// ============================================================
// runtimeSurfacePreview
// ============================================================

describe("runtimeSurfacePreview", () => {
  it("returns display summary first", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
      display: { summary: "display summary" },
      summary: "plain summary",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfacePreview(block)).toBe("display summary");
  });

  it("falls back to summary", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
      summary: "plain summary",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfacePreview(block)).toBe("plain summary");
  });

  it("returns null when no text field is populated", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfacePreview(block)).toBeNull();
  });
});

// ============================================================
// runtimeSurfaceLabel
// ============================================================

describe("runtimeSurfaceLabel", () => {
  it("returns title when available", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
      title: "Deploy",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfaceLabel(block)).toBe("Deploy");
  });

  it("falls back to kind when nothing else", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfaceLabel(block)).toBe("session_event");
  });
});

// ============================================================
// runtimeSurfacePhase
// ============================================================

describe("runtimeSurfacePhase", () => {
  it("returns phase when non-empty", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
      phase: "init",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfacePhase(block)).toBe("init");
  });

  it("returns null when phase is empty", () => {
    const block: RuntimeSurfaceOutputBlock = {
      kind: "session_event",
      phase: "",
    } as RuntimeSurfaceOutputBlock;
    expect(runtimeSurfacePhase(block)).toBeNull();
  });
});

// ============================================================
// runtimeSurfaceDebugDetail
// ============================================================

describe("runtimeSurfaceDebugDetail", () => {
  it("returns detail string from block", () => {
    const block = { kind: "status", detail: "some detail" } as unknown as OutputBlock;
    expect(runtimeSurfaceDebugDetail(block)).toBe("some detail");
  });

  it("returns undefined when block has no detail property", () => {
    const block = { kind: "message" } as unknown as OutputBlock;
    expect(runtimeSurfaceDebugDetail(block)).toBeUndefined();
  });
});

// ============================================================
// ingressStabilizationLabel
// ============================================================

describe("ingressStabilizationLabel", () => {
  it("returns null for null/undefined", () => {
    expect(ingressStabilizationLabel(null)).toBeNull();
    expect(ingressStabilizationLabel(undefined)).toBeNull();
  });

  it("formats source and policy", () => {
    expect(ingressStabilizationLabel({ source: "sse", policy: "metadata_only" })).toBe(
      "sse · metadata_only",
    );
  });

  it("includes batch count when > 1", () => {
    expect(
      ingressStabilizationLabel({ source: "sse", policy: "streaming", batch_count: 3 }),
    ).toBe("sse · streaming · batch 3");
  });

  it("defaults unknown source and metadata_only policy", () => {
    expect(ingressStabilizationLabel({})).toBe("unknown · metadata_only");
  });
});

// ============================================================
// readRuntimeBudgetNumber
// ============================================================

describe("readRuntimeBudgetNumber", () => {
  it("returns the value from config", () => {
    const config = { runtimeBudget: { web_max_pending_output_blocks: 128 } };
    expect(readRuntimeBudgetNumber(config, "web_max_pending_output_blocks", 256)).toBe(128);
  });

  it("returns fallback when config is null", () => {
    expect(readRuntimeBudgetNumber(null, "web_max_pending_output_blocks", 256)).toBe(256);
  });

  it("returns fallback when key is missing", () => {
    expect(readRuntimeBudgetNumber({}, "web_max_pending_output_blocks", 256)).toBe(256);
  });

  it("reads camelCase variant of snake_case key", () => {
    const config = { runtimeBudget: { webMaxPendingOutputBlocks: 64 } };
    expect(readRuntimeBudgetNumber(config, "web_max_pending_output_blocks", 256)).toBe(64);
  });
});
