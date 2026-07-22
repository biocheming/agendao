import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";

export type DiffLineKind = "add" | "del" | "hunk" | "header" | "context";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

const HEADER_PREFIXES = [
  "diff ",
  "index ",
  "old mode",
  "new mode",
  "new file",
  "deleted file",
  "similarity index",
  "dissimilarity index",
  "rename from",
  "rename to",
  "copy from",
  "copy to",
  "Binary files",
];

// Parses unified diff text into classified lines. Purely presentational:
// any line that is not an add/del/hunk/header marker is context.
export function parseUnifiedDiff(text: string): DiffLine[] {
  const trimmed = text.trim();
  if (!trimmed) return [];

  return trimmed.split("\n").map((line) => {
    if (line.startsWith("@@")) return { kind: "hunk", text: line };
    if (line.startsWith("+++") || line.startsWith("---")) {
      return { kind: "header", text: line };
    }
    if (line.startsWith("+")) return { kind: "add", text: line };
    if (line.startsWith("-")) return { kind: "del", text: line };
    if (HEADER_PREFIXES.some((prefix) => line.startsWith(prefix))) {
      return { kind: "header", text: line };
    }
    return { kind: "context", text: line };
  });
}

const DIFF_LINE_CLASSES: Record<DiffLineKind, string> = {
  add: "bg-(--ds-ok)/10 text-(--ds-ok)",
  del: "bg-(--ds-error)/10 text-(--ds-error)",
  hunk: "bg-(--ds-info)/10 text-(--ds-info)",
  header: "text-muted-foreground/70",
  context: "text-muted-foreground",
};

const COLLAPSE_THRESHOLD = 20;
const COLLAPSED_LINE_COUNT = 10;

export function DiffView({
  text,
  truncated = false,
  className,
}: {
  text: string;
  truncated?: boolean;
  className?: string;
}) {
  const lines = useMemo(() => parseUnifiedDiff(text), [text]);
  const [expanded, setExpanded] = useState(false);

  if (lines.length === 0) return null;

  const collapsible = lines.length > COLLAPSE_THRESHOLD;
  const visibleLines = collapsible && !expanded ? lines.slice(0, COLLAPSED_LINE_COUNT) : lines;

  return (
    <div className={cn("grid gap-1", className)}>
      <div
        className="overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-muted/50 p-2 font-mono text-xs leading-relaxed"
        data-testid="diff-view"
      >
        {visibleLines.map((line, index) => (
          <div
            key={`diff-line-${index}`}
            data-diff-kind={line.kind}
            className={cn("px-1", DIFF_LINE_CLASSES[line.kind])}
          >
            {line.text.length ? line.text : " "}
          </div>
        ))}
        {truncated ? (
          <div className="px-1 text-muted-foreground/70">… truncated</div>
        ) : null}
      </div>
      {collapsible ? (
        <button
          type="button"
          className="justify-self-start text-xs text-muted-foreground transition-colors hover:text-primary"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? "Show less" : `Show all (${lines.length} lines)`}
        </button>
      ) : null}
    </div>
  );
}
