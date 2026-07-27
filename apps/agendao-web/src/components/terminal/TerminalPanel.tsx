import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import type { useTerminalSessions } from "../../hooks/useTerminalSessions";
import { useAgendaoStore } from "../../store";
import type { ThemeId } from "../../lib/webRuntime";
import {
  THEME_SEMANTIC_TOKENS,
  WEB_THEME_TOKEN_SOURCE,
} from "../../generated/themeTokens.generated";

/// xterm 主题按全局主题联动。base 色为 globals.css 三套主题的同源近似 hex
/// （xterm canvas 不吃 oklch()）；cursor/selection 取色自 Rust Palette 生成的
/// 五行 token（`generate-theme-tokens.mjs`，与 TUI 单一真源）。
function xtermThemes(): Record<ThemeId, { background: string; foreground: string; cursor: string; cursorAccent: string; selectionBackground: string }> {
  const cursorOf = (t: ThemeId) =>
    THEME_SEMANTIC_TOKENS[WEB_THEME_TOKEN_SOURCE[t]].wood;
  return {
    daylight: {
      background: "#f9f8f4",
      foreground: "#232a3d",
      cursor: cursorOf("daylight"),
      cursorAccent: "#f9f8f4",
      selectionBackground: "rgba(35, 42, 61, 0.22)",
    },
    sunset: {
      background: "#f7f1e6",
      foreground: "#2e2820",
      cursor: cursorOf("sunset"),
      cursorAccent: "#f7f1e6",
      selectionBackground: "rgba(46, 40, 32, 0.22)",
    },
    cobalt: {
      background: "#101a2e",
      foreground: "#e8edf6",
      cursor: cursorOf("cobalt"),
      cursorAccent: "#101a2e",
      selectionBackground: "rgba(168, 182, 208, 0.34)",
    },
  };
}
const XTERM_THEMES = xtermThemes();

interface TerminalPanelProps {
  terminal: ReturnType<typeof useTerminalSessions>;
}

export function TerminalPanel({ terminal }: TerminalPanelProps) {
  const {
    activeSession,
    createSession,
    creating,
    enabled,
    getBuffer,
    loading,
    resizeSession,
    sendInput,
    sessionId,
    sessions,
    subscribeOutput,
  } = terminal;
  const theme = useAgendaoStore((s) => s.theme);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const renderedSessionIdRef = useRef<string | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    activeSessionIdRef.current = activeSession?.id ?? null;
  }, [activeSession]);

  // Auto-create at most once per chat session per mount: without a chat
  // session the server rejects PTY creation (session_id is required), and a
  // failed attempt must not retrigger this effect in a tight retry loop.
  const autoCreateAttemptedForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!enabled || loading || creating || sessions.length > 0 || !sessionId) return;
    if (autoCreateAttemptedForRef.current === sessionId) return;
    autoCreateAttemptedForRef.current = sessionId;
    void createSession();
  }, [createSession, creating, enabled, loading, sessionId, sessions.length]);

  // 主题切换时热更新（xterm.options.theme 支持运行时赋值）。
  useEffect(() => {
    if (xtermRef.current) {
      xtermRef.current.options.theme = XTERM_THEMES[theme] ?? XTERM_THEMES.daylight;
    }
  }, [theme]);

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
      theme: XTERM_THEMES[useAgendaoStore.getState().theme] ?? XTERM_THEMES.daylight,
    });
    const fitAddon = new FitAddon();
    xterm.loadAddon(fitAddon);
    xterm.open(viewport);

    const syncSize = () => {
      const activeSessionId = activeSessionIdRef.current;
      if (!activeSessionId) return;
      fitAddon.fit();
      void resizeSession(activeSessionId, xterm.cols, xterm.rows);
    };

    const queueSizeSync = () => {
      window.requestAnimationFrame(syncSize);
    };

    const dataDisposable = xterm.onData((data) => {
      sendInput(data);
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
      renderedSessionIdRef.current = null;
    };
  }, [resizeSession, sendInput]);

  // Stream output straight into xterm: the hook buffers scrollback in refs
  // and notifies subscribers per WS chunk, so this effect only re-runs when
  // the ACTIVE SESSION changes (snapshot replay), never per chunk — no React
  // re-render and no full-buffer startsWith diff on the output hot path.
  useEffect(() => {
    const xterm = xtermRef.current;
    if (!xterm) return;

    const sessionId = activeSession?.id ?? null;
    renderedSessionIdRef.current = sessionId;
    if (!sessionId) {
      xterm.reset();
      return;
    }

    // subscribe → snapshot → replay is synchronous, so no WS chunk can slip
    // between the snapshot and the subscription (no gaps, no duplicates).
    const unsubscribe = subscribeOutput(sessionId, (chunk) => {
      xterm.write(chunk);
    });
    const snapshot = getBuffer(sessionId);
    xterm.reset();
    if (snapshot) {
      xterm.write(snapshot);
    }
    fitAddonRef.current?.fit();
    void resizeSession(sessionId, xterm.cols, xterm.rows);
    xterm.focus();
    return unsubscribe;
  }, [activeSession, getBuffer, resizeSession, subscribeOutput]);

  return (
    <div className="h-full min-h-0 bg-transparent" data-testid="terminal-panel">
      <div
        ref={viewportRef}
        data-testid="terminal-viewport"
        className="terminal-viewport roc-terminal-viewport"
        onClick={() => xtermRef.current?.focus()}
      />
    </div>
  );
}
