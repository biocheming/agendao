import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSchedulerNavigation } from "./useSchedulerNavigation";
import { useAgendaoStore } from "../store";
import { resetAgendaoStore } from "../test/store-test-utils";
import type { SessionRecord } from "../lib/session";
import type { useExecutionActivity } from "./useExecutionActivity";

function createExecutionActivity() {
  return {
    executionNodes: [
      { id: "exec-1", stage_id: "stage-1" },
      { id: "exec-2", stage_id: "stage-2" },
    ],
    setSelectedExecutionId: vi.fn<(id: string | null) => void>(),
    patchActivityFilters: vi.fn<(filters: { stageId?: string; executionId?: string; eventType?: string }) => void>(),
  } as unknown as ReturnType<typeof useExecutionActivity>;
}

function apiJsonNoop<T>(): Promise<T> {
  return Promise.resolve(undefined as T);
}

function apiJsonAttachedSession<T>(): Promise<T> {
  return Promise.resolve({
    id: "attached",
    title: "Attached session",
    directory: "/repo",
    updated: 30,
  } as T);
}

describe("useSchedulerNavigation", () => {
  beforeEach(() => {
    resetAgendaoStore();
    useAgendaoStore.setState({
      sessions: [
        { id: "root", title: "Root session", directory: "/repo", updated: 20 } as SessionRecord,
        { id: "child", title: "Child session", directory: "/repo", updated: 10 } as SessionRecord,
      ],
      selectedSessionId: "root",
    });
  });

  it("focuses a stage, clears preview, updates execution activity, and sets banner", () => {
    const executionActivity = createExecutionActivity();
    const apiJson = vi.fn<typeof apiJsonNoop>(apiJsonNoop) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    const { result } = renderHook(() =>
      useSchedulerNavigation({
        apiJson,
        executionActivity,
        jumpToConversationTarget: vi.fn<(target: unknown) => void>(),
        queueConversationJumpTarget: vi.fn<(target: unknown) => void>(),
      }),
    );

    act(() => {
      result.current.previewStage("stage-2");
    });

    expect(useAgendaoStore.getState().previewStageId).toBe("stage-2");

    act(() => {
      result.current.navigateToStage("stage-1");
    });

    const state = useAgendaoStore.getState();
    expect(state.previewStageId).toBeNull();
    expect(state.activeStageContext).toEqual({
      stageId: "stage-1",
      executionId: null,
      toolCallId: null,
      label: "stage-1",
      sessionId: "root",
    });
    expect(state.banner).toBe("Focused stage stage-1");
    expect(executionActivity.setSelectedExecutionId).toHaveBeenCalledWith("exec-1");
    expect(executionActivity.patchActivityFilters).toHaveBeenCalledWith({
      stageId: "stage-1",
      executionId: "",
    });
  });

  it("navigates to an attached session and records breadcrumb provenance", async () => {
    const executionActivity = createExecutionActivity();
    const apiJson = vi.fn<typeof apiJsonAttachedSession>(apiJsonAttachedSession) as unknown as <T>(
      path: string,
      options?: RequestInit,
    ) => Promise<T>;

    const { result } = renderHook(() =>
      useSchedulerNavigation({
        apiJson,
        executionActivity,
        jumpToConversationTarget: vi.fn<(target: unknown) => void>(),
        queueConversationJumpTarget: vi.fn<(target: unknown) => void>(),
      }),
    );

    await act(async () => {
      await result.current.navigateToAttachedSession("attached", {
        stageId: "stage-2",
        toolCallId: "tool-7",
        label: "from tool",
      });
    });

    const state = useAgendaoStore.getState();
    expect(apiJson).toHaveBeenCalledWith("/session/attached");
    expect(state.selectedSessionId).toBe("attached");
    expect(state.sessions.map((session) => session.id)).toContain("attached");
    expect(state.sessionBreadcrumbs).toEqual([
      {
        sessionId: "root",
        title: "Root session",
        viaLabel: "from tool",
        viaStageId: "stage-2",
        viaToolCallId: "tool-7",
      },
      {
        sessionId: "attached",
        title: "Attached session",
      },
    ]);
    expect(result.current.currentBreadcrumbProvenance).toEqual({
      sourceSessionId: "root",
      sourceSessionTitle: "Root session",
      label: "from tool",
      stageId: "stage-2",
      toolCallId: "tool-7",
    });
  });
});
