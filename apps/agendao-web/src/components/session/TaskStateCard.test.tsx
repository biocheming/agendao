import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/i18n/I18nProvider";
import { ApiHttpError, apiJson } from "@/lib/api";
import type { SessionTaskLedger, TaskLedgerWriteResponse } from "@/lib/taskLedger";
import { useAgendaoStore } from "@/store";
import { resetAgendaoStore } from "@/test/store-test-utils";
import { TaskStateCard } from "./TaskStateCard";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, apiJson: vi.fn<typeof apiJson>() };
});

const apiJsonMock = vi.mocked(apiJson);

function ledger(revision = 3): SessionTaskLedger {
  return {
    session_id: "session-1",
    revision,
    goal_generation: 1,
    goal: {
      statement: "Ship the change",
      acceptance_criteria: ["tests pass"],
      criterion_checks: [],
      set_by: "model",
      set_at: 1,
    },
    core: [
      { id: "core-01", statement: "Preserve API", live: true, set_by: "system", set_at: 1 },
    ],
    verified: [
      {
        id: "chk-01",
        claim: "Unit tests passed",
        verifier: { deterministic_check: { description: "cargo test" } },
        coverage: { scope: "unit tests" },
        goal_generation: 1,
        covered_criteria: ["tests pass"],
        evidence_artifact_ids: ["/repo/test-output.txt"],
        source_stage_id: "verify/tests",
        superseded_by: null,
      },
    ],
    open: [
      {
        id: "open-01",
        question: "Was the UI checked?",
        settled_by: "manual review",
        closed_by_checkpoint_id: null,
      },
    ],
    next: {
      statement: "Review UI",
      provenance: { actor: "model", pre_interrupt: false, set_at: 1 },
    },
    status: "active",
    awaiting_interactions: [],
    blocked_reason: null,
    uncovered_criteria: [],
    updated_at: 1,
  };
}

function writeResponse(snapshot: SessionTaskLedger): TaskLedgerWriteResponse {
  return { ledger: snapshot, cause: "goal_updated", metadata_key: "task_ledger" };
}

function renderCard(props: { onNavigateStage?: (stageId: string) => void } = {}) {
  return render(
    <I18nProvider>
      <TaskStateCard {...props} />
    </I18nProvider>,
  );
}

describe("TaskStateCard", () => {
  beforeEach(() => {
    resetAgendaoStore();
    apiJsonMock.mockReset();
    useAgendaoStore.setState({
      selectedSessionId: "session-1",
      taskLedgers: { "session-1": ledger() },
    });
  });

  it("writes goal edits with CAS revision and user provenance", async () => {
    apiJsonMock.mockResolvedValueOnce(writeResponse(ledger(4)));
    renderCard();
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    fireEvent.click(screen.getByTitle("Edit goal"));
    fireEvent.change(screen.getByTestId("task-ledger-primary"), {
      target: { value: "Ship safely" },
    });
    fireEvent.click(screen.getByTestId("task-ledger-save"));

    await waitFor(() => expect(apiJsonMock).toHaveBeenCalledTimes(1));
    const [, options] = apiJsonMock.mock.calls[0];
    expect(options?.method).toBe("PATCH");
    expect(JSON.parse(String(options?.body))).toMatchObject({
      expected_revision: 3,
      op: {
        op: "set_goal",
        goal: { statement: "Ship safely", set_by: "user" },
      },
    });
  });

  it("reloads the authority snapshot on a revision conflict", async () => {
    apiJsonMock
      .mockRejectedValueOnce(new ApiHttpError(409, "revision conflict"))
      .mockResolvedValueOnce(ledger(7));
    renderCard();
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    fireEvent.click(screen.getByTitle("Edit next action"));
    fireEvent.change(screen.getByTestId("task-ledger-primary"), {
      target: { value: "New next" },
    });
    fireEvent.click(screen.getByTestId("task-ledger-save"));

    await waitFor(() => expect(apiJsonMock).toHaveBeenCalledTimes(2));
    expect(apiJsonMock.mock.calls[1][0]).toBe("/session/session-1/task-ledger");
    expect(useAgendaoStore.getState().taskLedgers["session-1"]?.revision).toBe(7);
    expect(useAgendaoStore.getState().banner).toContain("latest revision");
  });

  it("closes Open only by creating a user-confirmed checkpoint", async () => {
    apiJsonMock.mockResolvedValueOnce(writeResponse(ledger(4)));
    renderCard();
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    fireEvent.click(screen.getByTitle("Close with checkpoint"));
    fireEvent.change(screen.getByTestId("task-ledger-primary"), {
      target: { value: "UI reviewed" },
    });
    fireEvent.change(screen.getByTestId("task-ledger-coverage"), {
      target: { value: "desktop and mobile" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "tests pass" }));
    fireEvent.click(screen.getByTestId("task-ledger-save"));

    await waitFor(() => expect(apiJsonMock).toHaveBeenCalledTimes(1));
    expect(apiJsonMock.mock.calls[0][0]).toContain("/open/open-01/close");
    expect(JSON.parse(String(apiJsonMock.mock.calls[0][1]?.body))).toMatchObject({
      expected_revision: 3,
      verifier: { user_confirmation: { actor: "user" } },
      covered_criteria: ["tests pass"],
    });
  });

  it("navigates checkpoint stage and artifact evidence", () => {
    const onNavigateStage = vi.fn<(stageId: string) => void>();
    renderCard({ onNavigateStage });
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    fireEvent.click(screen.getByRole("button", { name: /Stage verify\/tests/ }));
    expect(onNavigateStage).toHaveBeenCalledWith("verify/tests");
    fireEvent.click(screen.getByRole("button", { name: "/repo/test-output.txt" }));
    expect(useAgendaoStore.getState().selectedFilePath).toBe("/repo/test-output.txt");
    expect(useAgendaoStore.getState().workspacePanelTab).toBe("files");
    expect(useAgendaoStore.getState().rightSidebarOpen).toBe(true);
  });
});
