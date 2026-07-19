import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ComposerSection } from "./ComposerSection";
import { resetAgendaoStore } from "../../test/store-test-utils";
import { useAgendaoStore } from "../../store";
import type { CommandApiSpec } from "../../lib/command";

const SLASH_COMMANDS: CommandApiSpec[] = [
  { name: "review", description: "Review code changes", aliases: ["rv"], source: "Builtin" },
  { name: "release", description: "Cut a release", source: "Builtin" },
  { name: "init", description: "Create AGENTS.md", source: "Builtin" },
];

function renderComposerSection(composer: string) {
  useAgendaoStore.setState({
    composer,
    attachments: [],
    streaming: false,
    modes: [],
    providers: [],
    selectedMode: "",
    selectedModel: "",
    workspaceContext: null,
    selectedWorkspacePath: null,
    slashCommands: SLASH_COMMANDS,
  });

  return render(
    <ComposerSection
      multimodalHints={[]}
      allowAudioInput={false}
      allowImageInput={true}
      allowFileInput={true}
      onModelChange={vi.fn<(value: string) => void>()}
      workspaceRootPath="/repo"
      composerNotice={null}
      activeStageId={null}
      provenance={null}
      onSubmit={vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) =>
        event.preventDefault(),
      )}
      onStopStreaming={vi.fn<() => void>()}
      onRemoveAttachment={vi.fn<(index: number) => void>()}
      onSelectAttachment={vi.fn<(index: number, attachment: { type: string }) => void>()}
      onLocateAttachment={vi.fn<(attachment: { type: string }) => void>()}
      onNavigateStage={vi.fn<(stageId: string) => void>()}
      onNavigateProvenanceSession={vi.fn<() => void>()}
      onNavigateProvenanceStage={vi.fn<() => void>()}
      onNavigateProvenanceToolCall={vi.fn<() => void>()}
      onDragEnter={vi.fn<(event: React.DragEvent<HTMLDivElement>) => void>()}
      onDragOver={vi.fn<(event: React.DragEvent<HTMLDivElement>) => void>()}
      onDragLeave={vi.fn<(event: React.DragEvent<HTMLDivElement>) => void>()}
      onDrop={vi.fn<(event: React.DragEvent<HTMLDivElement>) => void>()}
      onAttachFiles={vi.fn<(files: File[], failurePrefix: string) => void>()}
      onFileChange={vi.fn<(event: React.ChangeEvent<HTMLInputElement>) => void>()}
      onPaste={vi.fn<(event: React.ClipboardEvent<HTMLTextAreaElement>) => void>()}
    />,
  );
}

describe("ComposerSection slash commands", () => {
  beforeEach(() => {
    resetAgendaoStore();
  });

  it("lists filtered commands for a slash token and replaces the input on Enter", () => {
    renderComposerSection("/re");

    const menu = screen.getByTestId("slash-command-menu");
    expect(menu).toBeInTheDocument();
    expect(screen.getByTestId("slash-command-item-release")).toBeInTheDocument();
    expect(screen.getByTestId("slash-command-item-review")).toBeInTheDocument();
    expect(screen.queryByTestId("slash-command-item-init")).not.toBeInTheDocument();

    const input = screen.getByTestId("composer-input");
    expect(screen.getByTestId("slash-command-item-release")).toHaveAttribute(
      "data-active",
      "true",
    );
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(screen.getByTestId("slash-command-item-review")).toHaveAttribute(
      "data-active",
      "true",
    );

    fireEvent.keyDown(input, { key: "Enter" });
    expect(useAgendaoStore.getState().composer).toBe("/review ");
    expect(screen.queryByTestId("slash-command-menu")).not.toBeInTheDocument();
  });

  it("closes on Escape without touching the composer text and selects on click", () => {
    renderComposerSection("/re");

    const input = screen.getByTestId("composer-input");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByTestId("slash-command-menu")).not.toBeInTheDocument();
    expect(useAgendaoStore.getState().composer).toBe("/re");

    fireEvent.change(input, { target: { value: "/" } });
    expect(screen.getByTestId("slash-command-menu")).toBeInTheDocument();
    fireEvent.mouseDown(screen.getByTestId("slash-command-item-init"));
    expect(useAgendaoStore.getState().composer).toBe("/init ");
    expect(screen.queryByTestId("slash-command-menu")).not.toBeInTheDocument();
  });

  it("stays hidden once the command token is complete", () => {
    renderComposerSection("/review src/");
    expect(screen.queryByTestId("slash-command-menu")).not.toBeInTheDocument();
  });
});
