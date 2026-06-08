import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { InteractionOverlays } from "./InteractionOverlays";

describe("InteractionOverlays", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("handles question option toggles, free-text answers, submit, and reject", () => {
    const onQuestionAnswerChange = vi.fn<(index: number, value: string | string[]) => void>();
    const onRejectQuestion = vi.fn<() => void>();
    const onSubmitQuestion = vi.fn<() => void>();

    render(
      <InteractionOverlays
        question={{
          request_id: "q-1",
          questions: [
            {
              header: "Mode",
              question: "Choose execution modes",
              multiple: true,
              options: [
                { label: "Fast" },
                { label: "Deep" },
              ],
            },
            {
              question: "Add context",
            },
          ],
        }}
        permission={null}
        questionAnswers={{ 0: ["Fast"] }}
        questionSubmitting={false}
        permissionSubmitting={false}
        permissionSubmitError={null}
        permissionSubmitStartedAt={null}
        permissionSubmitCompletedAt={null}
        onQuestionAnswerChange={onQuestionAnswerChange}
        onRejectQuestion={onRejectQuestion}
        onSubmitQuestion={onSubmitQuestion}
        onReplyPermission={vi.fn<(reply: "once" | "turn" | "session" | "reject") => void>()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Fast" }));
    expect(onQuestionAnswerChange).toHaveBeenCalledWith(0, []);

    const deepOption = screen.getByRole("button", { name: "Deep" });
    fireEvent.click(deepOption);
    expect(onQuestionAnswerChange).toHaveBeenCalledWith(0, ["Fast", "Deep"]);

    fireEvent.change(screen.getByTestId("question-input"), {
      target: { value: "Need more context" },
    });
    expect(onQuestionAnswerChange).toHaveBeenCalledWith(1, "Need more context");

    fireEvent.click(screen.getByTestId("question-submit"));
    expect(onSubmitQuestion).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId("question-reject"));
    expect(onRejectQuestion).toHaveBeenCalledTimes(1);
  });

  it("renders permission details and dispatches supported permission replies", () => {
    const onReplyPermission = vi.fn<(reply: "once" | "turn" | "session" | "reject") => void>();

    render(
      <InteractionOverlays
        question={null}
        permission={{
          permission_id: "perm-1",
          message: "Need approval",
          permission: "workspace.write",
          permission_class_label: "Workspace write",
          scope_label: "Current worktree",
          matcher_label: "path matcher",
          grant_target_summary: "/repo/src",
          supported_lifetimes: ["once", "turn", "session"],
          grant_hint: "once = this request | turn = current turn | session = this session",
          command: "cargo test --all --workspace --features web",
          filepath: "/repo/src/components/really/long/path/file.tsx",
        }}
        questionAnswers={{}}
        questionSubmitting={false}
        permissionSubmitting={false}
        permissionSubmitError={"last request failed"}
        permissionSubmitStartedAt={"2026-06-08T10:00:00Z"}
        permissionSubmitCompletedAt={"2026-06-08T10:00:05Z"}
        onQuestionAnswerChange={vi.fn<(index: number, value: string | string[]) => void>()}
        onRejectQuestion={vi.fn<() => void>()}
        onSubmitQuestion={vi.fn<() => void>()}
        onReplyPermission={onReplyPermission}
      />,
    );

    expect(screen.getByTestId("permission-submit-error")).toHaveTextContent("last request failed");
    expect(screen.getByTestId("permission-submit-started")).toHaveTextContent("2026-06-08T10:00:00Z");
    expect(screen.getByTestId("permission-submit-completed")).toHaveTextContent("2026-06-08T10:00:05Z");
    expect(screen.getByTestId("permission-command")).toBeInTheDocument();
    expect(screen.getByTestId("permission-path")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("permission-turn"));
    fireEvent.click(screen.getByTestId("permission-session"));
    fireEvent.click(screen.getByTestId("permission-once"));
    fireEvent.click(screen.getByTestId("permission-reject"));

    expect(onReplyPermission).toHaveBeenNthCalledWith(1, "turn");
    expect(onReplyPermission).toHaveBeenNthCalledWith(2, "session");
    expect(onReplyPermission).toHaveBeenNthCalledWith(3, "once");
    expect(onReplyPermission).toHaveBeenNthCalledWith(4, "reject");
  });
});
