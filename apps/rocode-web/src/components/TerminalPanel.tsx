import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import type { useTerminalSessions } from "../hooks/useTerminalSessions";

interface TerminalPanelProps {
  terminal: ReturnType<typeof useTerminalSessions>;
}

export function TerminalPanel({ terminal }: TerminalPanelProps) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const renderedBufferRef = useRef("");
  const renderedSessionIdRef = useRef<string | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    activeSessionIdRef.current = terminal.activeSession?.id ?? null;
  }, [terminal.activeSession]);

  useEffect(() => {
    if (!terminal.enabled || terminal.loading || terminal.creating || terminal.sessions.length > 0) return;
    void terminal.createSession();
  }, [terminal]);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    const xterm = new XTerm({
      cursorBlink: true,
      fontFamily: '"SFMono-Regular", "Cascadia Code", "Fira Code", monospace',
      fontSize: 13,
      lineHeight: 1.3,
      rows: 24,
      cols: 80,
      theme: {
        background: "#0f172a",
        foreground: "#e2e8f0",
        cursor: "#f8fafc",
        cursorAccent: "#0f172a",
        selectionBackground: "rgba(148, 163, 184, 0.34)",
      },
    });
    const fitAddon = new FitAddon();
    xterm.loadAddon(fitAddon);
    xterm.open(viewport);

    const syncSize = () => {
      const activeSessionId = activeSessionIdRef.current;
      if (!activeSessionId) return;
      fitAddon.fit();
      void terminal.resizeSession(activeSessionId, xterm.cols, xterm.rows);
    };

    const queueSizeSync = () => {
      window.requestAnimationFrame(syncSize);
    };

    const dataDisposable = xterm.onData((data) => {
      terminal.sendInput(data);
    });
    const resizeObserver = new ResizeObserver(() => {
      queueSizeSync();
    });

    resizeObserver.observe(viewport);
    xtermRef.current = xterm;
    fitAddonRef.current = fitAddon;
    queueSizeSync();

    return () => {
      resizeObserver.disconnect();
      dataDisposable.dispose();
      fitAddon.dispose();
      xterm.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
      renderedBufferRef.current = "";
      renderedSessionIdRef.current = null;
    };
  }, [terminal.resizeSession, terminal.sendInput]);

  useEffect(() => {
    const xterm = xtermRef.current;
    if (!xterm) return;

    if (!terminal.activeSession) {
      if (renderedSessionIdRef.current || renderedBufferRef.current) {
        xterm.reset();
      }
      renderedSessionIdRef.current = null;
      renderedBufferRef.current = "";
      return;
    }

    const sessionId = terminal.activeSession.id;
    const buffer = terminal.activeBuffer;
    const switchingSessions = renderedSessionIdRef.current !== sessionId;

    if (switchingSessions) {
      xterm.reset();
      if (buffer) {
        xterm.write(buffer);
      }
      renderedSessionIdRef.current = sessionId;
      renderedBufferRef.current = buffer;
      fitAddonRef.current?.fit();
      void terminal.resizeSession(sessionId, xterm.cols, xterm.rows);
      xterm.focus();
      return;
    }

    const previous = renderedBufferRef.current;
    if (!buffer) {
      if (previous) {
        xterm.reset();
      }
      renderedBufferRef.current = "";
      return;
    }

    if (buffer.startsWith(previous)) {
      const delta = buffer.slice(previous.length);
      if (delta) {
        xterm.write(delta);
      }
    } else {
      xterm.reset();
      xterm.write(buffer);
    }

    renderedBufferRef.current = buffer;
  }, [terminal.activeBuffer, terminal.activeSession, terminal.resizeSession]);

  return (
    <div className="h-full min-h-0 bg-transparent p-2" data-testid="terminal-panel">
      <div
        ref={viewportRef}
        data-testid="terminal-viewport"
        className="terminal-viewport roc-terminal-viewport"
        onClick={() => xtermRef.current?.focus()}
      />
    </div>
  );
}
