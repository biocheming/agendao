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
  } as unknown as ReturnType<typeof useExecutionActivity>;
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
    const { result } = renderHook(() =>
      useSchedulerNavigation({
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
  });

});
