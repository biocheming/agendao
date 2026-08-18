import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DIRECT_BLUEPRINT_STARTER,
  formatBlueprintDocument,
  type SessionBlueprintViewRecord,
} from "@/lib/blueprint";
import { useAgendaoStore } from "@/store";
import { SessionBlueprintEditor } from "./SessionBlueprintEditor";

const plannerView: SessionBlueprintViewRecord = {
  blueprint: DIRECT_BLUEPRINT_STARTER,
  generatedAgents: [
    {
      id: "security-reviewer",
      base_agent: "build",
      system_policy: "Focus on security boundaries.",
    },
  ],
  fingerprint: "planner-fingerprint",
  selectionSource: "planner",
};

type ApiJson = Parameters<typeof SessionBlueprintEditor>[0]["apiJson"];

describe("SessionBlueprintEditor", () => {
  beforeEach(() => {
    useAgendaoStore.setState({ selectedMode: "agent:build" });
  });

  it("loads the current document and saves the edited Blueprint payload", async () => {
    const onChanged = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    const apiJsonImpl = async <T,>(_path: string, options?: RequestInit): Promise<T> => {
      if (!options) return plannerView as T;
      const request = JSON.parse(String(options.body)) as { blueprint: typeof DIRECT_BLUEPRINT_STARTER };
      return {
        ...plannerView,
        blueprint: request.blueprint,
        generatedAgents: [],
        selectionSource: "user",
      } as T;
    };
    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as ApiJson;
    render(
      <SessionBlueprintEditor
        sessionId="session/a"
        hasBlueprint
        apiJson={apiJson}
        onChanged={onChanged}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Manage Blueprint" }));
    const editor = await screen.findByLabelText("Scheduler Blueprint document");
    const edited = { ...DIRECT_BLUEPRINT_STARTER, name: "review-release" };
    fireEvent.change(editor, { target: { value: formatBlueprintDocument(edited) } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(apiJson).toHaveBeenCalledWith("/session/session%2Fa/blueprint", {
        method: "PUT",
        body: JSON.stringify({ blueprint: edited }),
      });
    });
    expect(useAgendaoStore.getState().selectedMode).toBe("scheduler:auto");
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

  it("shows generated Agents and rejects an AI-planned Blueprint", async () => {
    const onChanged = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    const apiJsonImpl = async <T,>(_path: string, options?: RequestInit): Promise<T> => {
      if (!options) return plannerView as T;
      return { rejectedFingerprint: plannerView.fingerprint } as T;
    };
    const apiJson = vi.fn<typeof apiJsonImpl>(apiJsonImpl) as unknown as ApiJson;
    render(
      <SessionBlueprintEditor
        sessionId="session-1"
        hasBlueprint
        apiJson={apiJson}
        onChanged={onChanged}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Manage Blueprint" }));
    expect(await screen.findByText("security-reviewer")).toBeInTheDocument();
    expect(screen.getByText("build")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));

    await waitFor(() => {
      expect(apiJson).toHaveBeenCalledWith("/session/session-1/blueprint/reject", {
        method: "POST",
      });
    });
    expect(onChanged).toHaveBeenCalledTimes(1);
  });
});
