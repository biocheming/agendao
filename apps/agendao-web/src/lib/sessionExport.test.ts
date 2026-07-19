import { describe, expect, it } from "vitest";
import type { MessageRecord } from "./history";
import { buildSessionExportMarkdown, sessionExportFileName } from "./sessionExport";

function message(overrides: Partial<MessageRecord>): MessageRecord {
  return { id: "msg-1", role: "user", ...overrides };
}

describe("buildSessionExportMarkdown", () => {
  const exportedAt = new Date("2026-07-18T01:02:03.000Z");

  it("renders heading, export metadata, and per-message role sections", () => {
    const markdown = buildSessionExportMarkdown({
      title: "Refactor Plan",
      sessionId: "sess-1",
      exportedAt,
      messages: [
        message({
          id: "m1",
          role: "user",
          parts: [{ id: "p1", type: "text", text: "Rename the sidebar." }],
        }),
        message({
          id: "m2",
          role: "assistant",
          parts: [{ id: "p2", type: "text", text: "Done, see the diff." }],
        }),
      ],
    });

    expect(markdown).toBe(
      [
        "# Refactor Plan",
        "",
        "- Session: sess-1",
        "- Exported at: 2026-07-18T01:02:03.000Z",
        "",
        "## user",
        "",
        "Rename the sidebar.",
        "",
        "## assistant",
        "",
        "Done, see the diff.",
        "",
      ].join("\n"),
    );
  });

  it("falls back to the session id when the title is blank", () => {
    const markdown = buildSessionExportMarkdown({
      title: "  ",
      sessionId: "sess-2",
      exportedAt,
      messages: [],
    });

    expect(markdown).toContain("# sess-2");
  });

  it("skips tool parts and keeps a reasoning marker", () => {
    const markdown = buildSessionExportMarkdown({
      title: "Run",
      sessionId: "sess-3",
      exportedAt,
      messages: [
        message({
          id: "m1",
          role: "assistant",
          parts: [
            { id: "p1", type: "reasoning", text: "thinking about it" },
            {
              id: "p2",
              type: "tool_call",
              text: undefined,
              output_block: { kind: "tool", text: "secret tool payload" },
            },
            { id: "p3", type: "text", text: "Final answer." },
          ],
        }),
      ],
    });

    expect(markdown).toContain("## assistant");
    expect(markdown).toContain("> *(reasoning)*");
    expect(markdown).toContain("Final answer.");
    expect(markdown).not.toContain("thinking about it");
    expect(markdown).not.toContain("secret tool payload");
  });

  it("omits messages without visible text and ignored parts", () => {
    const markdown = buildSessionExportMarkdown({
      title: "Run",
      sessionId: "sess-4",
      exportedAt,
      messages: [
        message({
          id: "m1",
          role: "user",
          parts: [{ id: "p1", type: "text", text: "   ", ignored: true }],
        }),
        message({
          id: "m2",
          role: "user",
          parts: [{ id: "p2", type: "text", text: "Keep me." }],
        }),
      ],
    });

    expect(markdown.match(/^## /gm)).toHaveLength(1);
    expect(markdown).toContain("Keep me.");
  });
});

describe("sessionExportFileName", () => {
  it("uses the trimmed title and strips filesystem-hostile characters", () => {
    expect(sessionExportFileName('  My: Session/Plan?  ', "sess-1")).toBe("My- Session-Plan.md");
  });

  it("falls back to the session id for empty titles", () => {
    expect(sessionExportFileName("", "sess-9")).toBe("sess-9.md");
    expect(sessionExportFileName(null, "sess-9")).toBe("sess-9.md");
  });
});
