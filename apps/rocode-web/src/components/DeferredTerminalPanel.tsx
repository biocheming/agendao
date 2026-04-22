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
    <div className="roc-panel roc-rail-panel p-5" data-testid="terminal-loading">
      <div className="roc-rail-empty">
        <div className="roc-section-label">Terminal</div>
        <h3 className="text-sm font-semibold tracking-tight text-foreground">Loading terminal…</h3>
        <p className="text-sm leading-6 text-muted-foreground">
          The xterm.js terminal is being loaded as a separate chunk.
        </p>
      </div>
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
      <div className="roc-panel roc-rail-panel p-5" data-testid="terminal-collapsed">
        <div className="roc-rail-header">
          <div className="roc-rail-headline">
            <p className="roc-section-label">Terminal</p>
            <h3 className="roc-rail-title">PTY Sessions</h3>
            <p className="roc-rail-description">Keep the shell lazy by default and only pay the xterm cost when needed.</p>
          </div>
          <button
            className="roc-action roc-action-pill"
            type="button"
            data-testid="terminal-open"
            onClick={onExpand}
          >
            Open Terminal
          </button>
        </div>
        <div className="roc-rail-empty">
          <p>
            Terminal stays collapsed by default, so PTY sessions and `xterm.js` are not loaded on
            first paint.
          </p>
          <p>Expand this panel when you actually need shell access.</p>
        </div>
      </div>
    );
  }

  return (
    <Suspense fallback={<TerminalLoadingFallback />}>
      <TerminalPanel terminal={terminal} />
    </Suspense>
  );
}
