import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SessionHeader } from "./SessionHeader";

describe("SessionHeader", () => {
  it("shows the effective session model and agent with explicit labels", () => {
    render(
      <SessionHeader
        title="Current session"
        subtitle="/repo"
        modelLabel="deepseek/deepseek-v4-pro"
        agentLabel="build"
        activeStageId={null}
        breadcrumbs={[]}
        provenance={null}
        onNavigateStage={vi.fn<(stageId: string) => void>()}
        onNavigateBreadcrumb={vi.fn<(sessionId: string) => void>()}
        onNavigateProvenanceSession={vi.fn<() => void>()}
        onNavigateProvenanceStage={vi.fn<() => void>()}
        onNavigateProvenanceToolCall={vi.fn<() => void>()}
      />,
    );

    expect(screen.getByText("Model: deepseek/deepseek-v4-pro · Agent: build")).toBeVisible();
  });
});
