import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DiffView, parseUnifiedDiff } from "./DiffView";

const SAMPLE_DIFF = [
  "diff --git a/src/main.ts b/src/main.ts",
  "index 1111111..2222222 100644",
  "--- a/src/main.ts",
  "+++ b/src/main.ts",
  "@@ -1,3 +1,3 @@",
  " const a = 1;",
  "-const b = 2;",
  "+const b = 3;",
  " const c = 4;",
].join("\n");

describe("parseUnifiedDiff", () => {
  it("classifies header, hunk, add, del, and context lines", () => {
    expect(parseUnifiedDiff(SAMPLE_DIFF)).toEqual([
      { kind: "header", text: "diff --git a/src/main.ts b/src/main.ts" },
      { kind: "header", text: "index 1111111..2222222 100644" },
      { kind: "header", text: "--- a/src/main.ts" },
      { kind: "header", text: "+++ b/src/main.ts" },
      { kind: "hunk", text: "@@ -1,3 +1,3 @@" },
      { kind: "context", text: " const a = 1;" },
      { kind: "del", text: "-const b = 2;" },
      { kind: "add", text: "+const b = 3;" },
      { kind: "context", text: " const c = 4;" },
    ]);
  });

  it("returns no lines for an empty diff", () => {
    expect(parseUnifiedDiff("")).toEqual([]);
    expect(parseUnifiedDiff("   \n  ")).toEqual([]);
  });
});

describe("DiffView", () => {
  it("renders classified lines with kind markers", () => {
    const { container } = render(<DiffView text={SAMPLE_DIFF} />);

    const kinds = Array.from(container.querySelectorAll("[data-diff-kind]")).map((node) =>
      node.getAttribute("data-diff-kind"),
    );
    expect(kinds).toEqual([
      "header",
      "header",
      "header",
      "header",
      "hunk",
      "context",
      "del",
      "add",
      "context",
    ]);
    expect(screen.getByTestId("diff-view")).toHaveTextContent("+const b = 3;");
  });

  it("renders nothing for an empty diff", () => {
    const { container } = render(<DiffView text="" />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a truncated marker when the diff is truncated", () => {
    render(<DiffView text={SAMPLE_DIFF} truncated />);
    expect(screen.getByText("… truncated")).toBeInTheDocument();
  });

  it("collapses long diffs and expands on demand", () => {
    const longDiff = Array.from({ length: 25 }, (_, index) => `+line ${index + 1}`).join("\n");
    render(<DiffView text={longDiff} />);

    // Collapsed by default: first 10 lines visible, the rest hidden.
    expect(screen.getByText("+line 10")).toBeInTheDocument();
    expect(screen.queryByText("+line 11")).toBeNull();

    const toggle = screen.getByRole("button", { name: "Show all (25 lines)" });
    fireEvent.click(toggle);

    expect(screen.getByText("+line 25")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show less" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(screen.queryByText("+line 11")).toBeNull();
  });

  it("does not collapse diffs at or below the threshold", () => {
    const shortDiff = Array.from({ length: 20 }, (_, index) => `+line ${index + 1}`).join("\n");
    render(<DiffView text={shortDiff} />);

    expect(screen.getByText("+line 20")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /show all/i })).toBeNull();
  });
});
