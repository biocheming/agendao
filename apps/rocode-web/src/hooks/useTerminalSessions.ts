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
  const [buffers, setBuffers] = useState<Map<string, string>>(new Map());
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const socketsRef = useRef<Map<string, WebSocket>>(new Map());
  const decoderRef = useRef(new TextDecoder());

  const appendOutput = useCallback((sessionId: string, chunk: string) => {
    setBuffers((current) => {
      const next = new Map(current);
      const existing = next.get(sessionId) ?? "";
      let value = existing + chunk;
      if (value.length > MAX_BUFFER_SIZE) {
        value = value.slice(-MAX_BUFFER_SIZE);
      }
      next.set(sessionId, value);
      return next;
    });
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
    setCreating(true);
    try {
      const session = await apiJson<PtySession>("/pty", {
        method: "POST",
        body: JSON.stringify({
          command: "/bin/bash",
          cwd: defaultCwd.trim() || undefined,
          session_id: sessionId ?? undefined,
        }),
      });
      setSessions((current) => [...current, session]);
      setActiveId(session.id);
      connectSocket(session.id);
      setBanner(`Created terminal ${session.id}`);
    } catch (error) {
      setBanner(`Failed to create terminal: ${formatError(error)}`);
    } finally {
      setCreating(false);
    }
  }, [apiJson, connectSocket, defaultCwd, sessionId, setBanner]);

  const deleteSession = useCallback(
    async (sessionId: string) => {
      try {
        closeSocket(sessionId);
        await api(`/pty/${sessionId}`, { method: "DELETE" });
        setSessions((current) => current.filter((session) => session.id !== sessionId));
        setBuffers((current) => {
          const next = new Map(current);
          next.delete(sessionId);
          return next;
        });
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
    activeBuffer: activeId ? buffers.get(activeId) ?? "" : "",
    loading,
    creating,
    enabled,
    setActiveId,
    createSession,
    deleteSession,
    sendInput,
    resizeSession,
    refresh,
  };
}
