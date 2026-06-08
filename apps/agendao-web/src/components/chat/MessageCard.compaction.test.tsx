import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MessageCard } from "./MessageCard";
import type { FeedMessage } from "../../lib/history";

describe("MessageCard compaction block", () => {
  it("renders synthetic compaction status as a session-local compaction block", () => {
    const message: FeedMessage = {
      kind: "status",
      role: "system",
      id: "__compaction__:ses_1",
      feedId: "__compaction__:ses_1",
      title: "Compacting conversation",
      summary: "compressing conversation · ~58K tok",
      text: "context pressure · pre request · 58K/100K 58%",
      metadata: {
        agendao_web_synthetic_compaction: true,
        agendao_web_compaction_status_line: "compressing conversation · ~58K tok",
        agendao_web_compaction_detail_line: "context pressure · pre request · 58K/100K 58%",
      },
    };

    render(
      <MessageCard
        message={message}
        onNavigateStage={vi.fn<(stageId: string) => void>()}
        onNavigateAttachedSession={vi.fn<(sessionId: string) => void>()}
      />,
    );

    expect(screen.getByText("Compacting conversation")).toBeInTheDocument();
    expect(screen.getByText(/compressing conversation/i)).toBeInTheDocument();
    expect(screen.getByText(/context pressure/i)).toBeInTheDocument();
  });
});
