import { useCallback, useEffect, useRef, useState } from "react";
import { webSocketUrl } from "@/lib/api";

interface PtySession {
  id: string;
  command: string;
  cwd: string;
  status: string;
}

interface UseTerminalSessionsOptions {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  setBanner: (message: string) => void;
  enabled?: boolean;
  defaultCwd?: string;
  sessionId?: string | null;
}

const MAX_BUFFER_SIZE = 200 * 1024;

function formatError(error: unknown) {
  return error instanceof Error ? error.message : String(error ?? "Unknown error");
}

/// The server rejects PTY creation (400) when the requested cwd was deleted or
/// lies outside the server project root. `api()` throws the raw response body,
/// so match the server-side error text (crates/agendao-server/src/routes/pty.rs).
function isPtyCwdError(error: unknown) {
  const message = formatError(error);
  return message.includes("Failed to resolve PTY cwd")
    || message.includes("PTY cwd must stay inside the project directory");
}

export function useTerminalSessions({
  api,
  apiJson,
  setBanner,
  enabled = false,
  defaultCwd = "",
  sessionId = null,
}: UseTerminalSessionsOptions) {
  const [sessions, setSessions] = useState<PtySession[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const socketsRef = useRef<Map<string, WebSocket>>(new Map());
  const decoderRef = useRef(new TextDecoder());
  // Scrollback lives in refs (not React state): WS chunks append here and are
  // pushed straight to subscribed xterm instances, so a busy terminal causes
  // zero React re-renders and zero full-buffer string copies/diffs.
  const buffersRef = useRef<Map<string, string>>(new Map());
  const outputListenersRef = useRef<Map<string, Set<(chunk: string) => void>>>(new Map());

  const appendOutput = useCallback((sessionId: string, chunk: string) => {
    const existing = buffersRef.current.get(sessionId) ?? "";
    let value = existing + chunk;
    if (value.length > MAX_BUFFER_SIZE) {
      value = value.slice(-MAX_BUFFER_SIZE);
    }
    buffersRef.current.set(sessionId, value);
    const listeners = outputListenersRef.current.get(sessionId);
    if (listeners) {
      for (const listener of listeners) {
        listener(chunk);
      }
    }
  }, []);

  const getBuffer = useCallback((sessionId: string) => buffersRef.current.get(sessionId) ?? "", []);

  const subscribeOutput = useCallback((sessionId: string, listener: (chunk: string) => void) => {
    const listeners = outputListenersRef.current.get(sessionId) ?? new Set<(chunk: string) => void>();
    listeners.add(listener);
    outputListenersRef.current.set(sessionId, listeners);
    return () => {
      const current = outputListenersRef.current.get(sessionId);
      if (!current) return;
      current.delete(listener);
      if (current.size === 0) {
        outputListenersRef.current.delete(sessionId);
      }
    };
  }, []);

  const closeSocket = useCallback((sessionId: string) => {
    const socket = socketsRef.current.get(sessionId);
    if (!socket) return;
    socket.close();
    socketsRef.current.delete(sessionId);
  }, []);

  const connectSocket = useCallback(
    (sessionId: string) => {
      const existing = socketsRef.current.get(sessionId);
      if (existing && (existing.readyState === WebSocket.OPEN || existing.readyState === WebSocket.CONNECTING)) {
        return;
      }

      const socket = new WebSocket(webSocketUrl(`/pty/${sessionId}/connect?cursor=-1`));
      socket.binaryType = "arraybuffer";

      socket.addEventListener("message", (event) => {
        if (event.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(event.data);
          if (bytes.length > 0 && bytes[0] === 0x00) return;
          appendOutput(sessionId, decoderRef.current.decode(bytes));
          return;
        }
        appendOutput(sessionId, String(event.data ?? ""));
      });

      socket.addEventListener("close", () => {
        if (socketsRef.current.get(sessionId) === socket) {
          socketsRef.current.delete(sessionId);
        }
      });

      socket.addEventListener("error", () => {
        setBanner(`Terminal socket error for ${sessionId}`);
      });

      socketsRef.current.set(sessionId, socket);
    },
    [appendOutput, setBanner],
  );

  const loadSessions = useCallback(async () => {
    setLoading(true);
    try {
      const result = await apiJson<PtySession[]>("/pty");
      setSessions(result ?? []);
      setActiveId((current) => current && result.some((session) => session.id === current)
        ? current
        : result[0]?.id ?? null);
    } catch (error) {
      setBanner(`Failed to load terminal sessions: ${formatError(error)}`);
    } finally {
      setLoading(false);
    }
  }, [apiJson, setBanner]);

  useEffect(() => {
    if (!enabled) return;
    void loadSessions();
  }, [enabled, loadSessions, refreshToken]);

  useEffect(() => {
    if (!enabled) {
      for (const sessionId of socketsRef.current.keys()) {
        closeSocket(sessionId);
      }
      return;
    }
    sessions.forEach((session) => connectSocket(session.id));
    const validIds = new Set(sessions.map((session) => session.id));
    for (const sessionId of socketsRef.current.keys()) {
      if (!validIds.has(sessionId)) {
        closeSocket(sessionId);
      }
    }
  }, [closeSocket, connectSocket, enabled, sessions]);

  useEffect(
    () => () => {
      for (const sessionId of socketsRef.current.keys()) {
        closeSocket(sessionId);
      }
    },
    [closeSocket],
  );

  const createSession = useCallback(async () => {
    if (!sessionId) {
      // The server requires a session_id for PTY creation (permission
      // requests are bound to a chat session), so opening a terminal without
      // a selected session can never succeed — explain instead of looping
      // on 400s.
      setBanner("Select a session before opening a terminal");
      return;
    }
    setCreating(true);
    try {
      const postCreate = (cwd: string | undefined) =>
        apiJson<PtySession>("/pty", {
          method: "POST",
          body: JSON.stringify({
            command: "/bin/bash",
            cwd,
            session_id: sessionId ?? undefined,
          }),
        });
      const requestedCwd = defaultCwd.trim() || undefined;
      let session: PtySession;
      let fellBackToProjectRoot = false;
      try {
        session = await postCreate(requestedCwd);
      } catch (error) {
        // The chat session's directory may have been deleted (or live outside
        // the server project root): without a fallback the terminal stays an
        // empty, unusable box. Retry without cwd — the server then opens the
        // shell in the project root. Other failures (permission denied,
        // missing session_id) are not retried.
        if (!requestedCwd || !isPtyCwdError(error)) throw error;
        session = await postCreate(undefined);
        fellBackToProjectRoot = true;
      }
      setSessions((current) => [...current, session]);
      setActiveId(session.id);
      connectSocket(session.id);
      setBanner(fellBackToProjectRoot
        ? `Session directory unavailable; opened terminal ${session.id} in project root`
        : `Created terminal ${session.id}`);
    } catch (error) {
      setBanner(`Failed to create terminal: ${formatError(error)}`);
      // The request may have been aborted client-side (e.g. the 30s fetch
      // timeout) while the server later completed creation after a delayed
      // permission approval — re-list so such a session is not lost.
      await loadSessions().catch(() => {});
    } finally {
      setCreating(false);
    }
  }, [apiJson, connectSocket, defaultCwd, loadSessions, sessionId, setBanner]);

  const deleteSession = useCallback(
    async (sessionId: string) => {
      try {
        closeSocket(sessionId);
        await api(`/pty/${sessionId}`, { method: "DELETE" });
        setSessions((current) => current.filter((session) => session.id !== sessionId));
        buffersRef.current.delete(sessionId);
        setActiveId((current) => {
          if (current !== sessionId) return current;
          const remaining = sessions.filter((session) => session.id !== sessionId);
          return remaining[0]?.id ?? null;
        });
      } catch (error) {
        setBanner(`Failed to delete terminal ${sessionId}: ${formatError(error)}`);
      }
    },
    [api, closeSocket, sessions, setBanner],
  );

  const sendInput = useCallback(
    (value: string) => {
      if (!activeId || !value.length) return;
      const socket = socketsRef.current.get(activeId);
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        setBanner("Active terminal socket is not connected");
        return;
      }
      socket.send(value);
    },
    [activeId, setBanner],
  );

  const resizeSession = useCallback(
    async (sessionId: string, cols: number, rows: number) => {
      if (!sessionId || cols < 2 || rows < 2) return;
      try {
        await api(`/pty/${sessionId}/resize`, {
          method: "POST",
          body: JSON.stringify({
            cols,
            rows,
          }),
        });
      } catch (error) {
        setBanner(`Failed to resize terminal ${sessionId}: ${formatError(error)}`);
      }
    },
    [api, setBanner],
  );

  const refresh = useCallback(() => {
    setRefreshToken((current) => current + 1);
  }, []);

  return {
    sessions,
    activeId,
    activeSession: sessions.find((session) => session.id === activeId) ?? null,
    loading,
    creating,
    enabled,
    sessionId,
    setActiveId,
    createSession,
    deleteSession,
    sendInput,
    resizeSession,
    refresh,
    getBuffer,
    subscribeOutput,
  };
}
