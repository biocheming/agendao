import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useInteractionReplies } from "./useInteractionReplies";
import { useAgendaoStore } from "../store";
import { resetAgendaoStore } from "../test/store-test-utils";

describe("useInteractionReplies", () => {
  beforeEach(() => {
    resetAgendaoStore();
    useAgendaoStore.setState({
      selectedSessionId: "session-selected",
      permission: {
        permission_id: "permission-1",
        session_id: "session-owner",
        supported_lifetimes: ["once", "turn", "session"],
      },
    });
  });

  function renderReplies(api: (path: string, options?: RequestInit) => Promise<Response>) {
    const apiJson = async <T,>(): Promise<T> => {
      throw new Error("apiJson is not expected in permission reply tests");
    };
    return renderHook(() =>
      useInteractionReplies({
        api,
        apiJson,
        loadPendingQuestion: vi.fn<
          (requestId: string, sessionId?: string | null) => Promise<void>
        >(async () => undefined),
        sendPromptRequest: vi.fn<
          (sessionId: string, payload: Record<string, unknown>) => Promise<{ status: string }>
        >(async () => ({ status: "ok" })),
      }),
    );
  }

  it.each([
    ["trust_workspace" as const, "trusted_workspace"],
    ["full_access" as const, "unsandboxed_yolo"],
  ])("sets the session mode for %s before allowing the pending request", async (reply, mode) => {
    const api = vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(
      async () => new Response(null, { status: 200 }),
    );
    const { result } = renderReplies(api);

    await act(async () => {
      await result.current.replyPermission(reply);
    });

    expect(api).toHaveBeenNthCalledWith(1, "/session/session-owner/permission", {
      method: "PATCH",
      body: JSON.stringify({ mode }),
    });
    expect(api).toHaveBeenNthCalledWith(2, "/permission/permission-1/reply", {
      method: "POST",
      body: JSON.stringify({ reply: "once" }),
    });
    expect(useAgendaoStore.getState().permission).toBeNull();
  });

  it("sends ordinary lifetime replies without changing session mode", async () => {
    const api = vi.fn<(path: string, options?: RequestInit) => Promise<Response>>(
      async () => new Response(null, { status: 200 }),
    );
    const { result } = renderReplies(api);

    await act(async () => {
      await result.current.replyPermission("session");
    });

    expect(api).toHaveBeenCalledTimes(1);
    expect(api).toHaveBeenCalledWith("/permission/permission-1/reply", {
      method: "POST",
      body: JSON.stringify({ reply: "session" }),
    });
  });
});
