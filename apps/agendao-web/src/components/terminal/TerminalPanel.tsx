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
    activeBuffer,
    activeSession,
    createSession,
    creating,
    enabled,
    loading,
    resizeSession,
    sendInput,
    sessions,
  } = terminal;
  const theme = useAgendaoStore((s) => s.theme);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const renderedBufferRef = useRef("");
  const renderedSessionIdRef = useRef<string | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    activeSessionIdRef.current = activeSession?.id ?? null;
  }, [activeSession]);

  useEffect(() => {
    if (!enabled || loading || creating || sessions.length > 0) return;
    void createSession();
  }, [createSession, creating, enabled, loading, sessions.length]);

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
      renderedBufferRef.current = "";
      renderedSessionIdRef.current = null;
    };
  }, [resizeSession, sendInput]);

  useEffect(() => {
    const xterm = xtermRef.current;
    if (!xterm) return;

    if (!activeSession) {
      if (renderedSessionIdRef.current || renderedBufferRef.current) {
        xterm.reset();
      }
      renderedSessionIdRef.current = null;
      renderedBufferRef.current = "";
      return;
    }

    const sessionId = activeSession.id;
    const buffer = activeBuffer;
    const switchingSessions = renderedSessionIdRef.current !== sessionId;

    if (switchingSessions) {
      xterm.reset();
      if (buffer) {
        xterm.write(buffer);
      }
      renderedSessionIdRef.current = sessionId;
      renderedBufferRef.current = buffer;
      fitAddonRef.current?.fit();
      void resizeSession(sessionId, xterm.cols, xterm.rows);
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
  }, [activeBuffer, activeSession, resizeSession]);

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
