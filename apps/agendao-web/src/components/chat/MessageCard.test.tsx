import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MessageCard } from "./MessageCard";
import type { FeedMessage } from "../../lib/history";

function renderMessageCard(message: FeedMessage, onEditAndResend = vi.fn()) {
  return render(
    <MessageCard
      message={message}
      onEditAndResend={onEditAndResend}
      onNavigateStage={vi.fn<(stageId: string) => void>()}
      onNavigateAttachedSession={vi.fn<(sessionId: string) => void>()}
    />,
  );
}

describe("MessageCard", () => {
  it("shows revise and resend for user prompts and invokes the callback", () => {
    const onEditAndResend = vi.fn();
    const message: FeedMessage = {
      kind: "message",
      role: "user",
      id: "msg-user-1",
      feedId: "feed-user-1",
      anchorId: "msg-user-1",
      text: "Refactor this component without changing behavior.",
    };

    renderMessageCard(message, onEditAndResend);

    const action = screen.getByRole("button", { name: /revise & resend/i });
    fireEvent.click(action);

    const card = screen.getByTestId("message-card");
    expect(card).toHaveClass("justify-items-end");
    expect(onEditAndResend).toHaveBeenCalledTimes(1);
    expect(onEditAndResend).toHaveBeenCalledWith(message);
  });

  it("does not show revise and resend for assistant messages", () => {
    const message: FeedMessage = {
      kind: "message",
      role: "assistant",
      id: "msg-assistant-1",
      feedId: "feed-assistant-1",
      anchorId: "msg-assistant-1",
      text: "I refactored the component and preserved behavior.",
    };

    renderMessageCard(message);

    expect(screen.queryByRole("button", { name: /revise & resend/i })).toBeNull();
  });
});
