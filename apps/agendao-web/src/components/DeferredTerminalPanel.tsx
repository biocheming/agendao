import { Suspense, lazy } from "react";
import type { useTerminalSessions } from "../hooks/useTerminalSessions";

const TerminalPanel = lazy(async () => {
  const module = await import("./TerminalPanel");
  return { default: module.TerminalPanel };
});

interface DeferredTerminalPanelProps {
  expanded: boolean;
  onExpand: () => void;
  terminal: ReturnType<typeof useTerminalSessions>;
}

function TerminalLoadingFallback() {
  return (
    <div className="flex h-full items-center justify-center text-xs text-muted-foreground" data-testid="terminal-loading">
      Loading terminal...
    </div>
  );
}

export function DeferredTerminalPanel({
  expanded,
  onExpand,
  terminal,
}: DeferredTerminalPanelProps) {
  if (!expanded) {
    return (
      <div className="flex h-full items-center justify-center" data-testid="terminal-collapsed">
        <button
          className="roc-action roc-action-pill"
          type="button"
          data-testid="terminal-open"
          onClick={onExpand}
        >
          Open Terminal
        </button>
      </div>
    );
  }

  return (
    <Suspense fallback={<TerminalLoadingFallback />}>
      <TerminalPanel terminal={terminal} />
    </Suspense>
  );
}
