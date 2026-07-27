import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTerminalSessions } from "./useTerminalSessions";

class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readyState = FakeWebSocket.CONNECTING;
  binaryType = "";
  addEventListener() {}
  removeEventListener() {}
  close() {
    this.readyState = 3;
  }
  send() {}
}

function createApiJson(behavior: (body: Record<string, unknown>) => Promise<unknown>) {
  const impl = <T,>(path: string, options?: RequestInit): Promise<T> => {
    if (path === "/pty" && options?.method === "POST") {
      return behavior(JSON.parse(String(options.body))) as Promise<T>;
    }
    if (path === "/pty") {
      return Promise.resolve([] as T);
    }
    return Promise.resolve(undefined as T);
  };
  return vi.fn<typeof impl>(impl);
}

type ApiJson = Parameters<typeof useTerminalSessions>[0]["apiJson"];

function renderTerminalHook(apiJson: ReturnType<typeof createApiJson>, setBanner = vi.fn<(message: string) => void>()) {
  const api = vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(() => Promise.resolve(new Response("true")));
  const utils = renderHook(() =>
    useTerminalSessions({
      api,
      apiJson: apiJson as unknown as ApiJson,
      setBanner,
      enabled: false,
      defaultCwd: "/gone/deleted-dir",
      sessionId: "chat-session-1",
    }),
  );
  return { api, setBanner, ...utils };
}

describe("useTerminalSessions", () => {
  beforeEach(() => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  it("creates a terminal with the requested cwd when the server accepts it", async () => {
    const apiJson = createApiJson((body) =>
      Promise.resolve({ id: "pty-1", command: String(body.command), cwd: String(body.cwd), status: "running" }),
    );
    const setBanner = vi.fn<(message: string) => void>();
    const { result } = renderTerminalHook(apiJson, setBanner);

    await act(async () => {
      await result.current.createSession();
    });

    expect(apiJson).toHaveBeenCalledTimes(1);
    const body = JSON.parse(String(apiJson.mock.calls[0][1]?.body));
    expect(body.cwd).toBe("/gone/deleted-dir");
    expect(result.current.sessions.map((s) => s.id)).toContain("pty-1");
    expect(result.current.activeId).toBe("pty-1");
    expect(setBanner).toHaveBeenCalledWith("Created terminal pty-1");
  });

  it("falls back to the project root when the session directory was deleted", async () => {
    const seenBodies: Record<string, unknown>[] = [];
    const apiJson = createApiJson((body) => {
      seenBodies.push(body);
      if (body.cwd) {
        return Promise.reject(new Error(
          '{"error":{"message":"Failed to resolve PTY cwd: No such file or directory (os error 2)","type":"bad_request"}}',
        ));
      }
      return Promise.resolve({ id: "pty-2", command: "/bin/bash", cwd: "/repo", status: "running" });
    });
    const setBanner = vi.fn<(message: string) => void>();
    const { result } = renderTerminalHook(apiJson, setBanner);

    await act(async () => {
      await result.current.createSession();
    });

    expect(seenBodies).toHaveLength(2);
    expect(seenBodies[0].cwd).toBe("/gone/deleted-dir");
    expect(seenBodies[1].cwd).toBeUndefined();
    expect(result.current.sessions.map((s) => s.id)).toContain("pty-2");
    await waitFor(() => {
      expect(setBanner).toHaveBeenCalledWith(
        "Session directory unavailable; opened terminal pty-2 in project root",
      );
    });
  });

  it("does not retry on non-cwd failures such as permission denial", async () => {
    const apiJson = createApiJson(() =>
      Promise.reject(new Error('{"error":{"message":"Permission denied","type":"permission_denied"}}')),
    );
    const setBanner = vi.fn<(message: string) => void>();
    const { result } = renderTerminalHook(apiJson, setBanner);

    await act(async () => {
      await result.current.createSession();
    });

    expect(apiJson.mock.calls.filter(([path, options]) => path === "/pty" && options?.method === "POST")).toHaveLength(1);
    expect(result.current.sessions).toHaveLength(0);
    expect(setBanner).toHaveBeenCalledWith(expect.stringContaining("Failed to create terminal"));
  });
});
