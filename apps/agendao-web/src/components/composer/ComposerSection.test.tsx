import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ComposerSection } from "./ComposerSection";
import { resetAgendaoStore } from "../../test/store-test-utils";
import { useAgendaoStore } from "../../store";
import type { ExecutionMode } from "../../lib/webRuntime";
import type { ProviderRecord } from "../../lib/provider";

function renderComposerSection() {
  const modes: ExecutionMode[] = [{ id: "auto", name: "auto", kind: "preset" }];
  const providers: ProviderRecord[] = [{ id: "openai", name: "OpenAI", models: [] }];
  useAgendaoStore.setState({
    modes,
    providers,
    selectedMode: "preset:auto",
    selectedModel: "",
    workspaceContext: null,
    selectedWorkspacePath: null,
  });

  const onSubmit = vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) => event.preventDefault());
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
      onSubmit={onSubmit}
      onStopStreaming={vi.fn<() => void>()}
      onRemoveAttachment={vi.fn<(index: number) => void>()}
      onSelectAttachment={vi.fn<(index: number, attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
      onLocateAttachment={vi.fn<(attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
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

describe("ComposerSection", () => {
  beforeEach(() => {
    resetAgendaoStore();
  });

  it("disables send for an empty composer and enables it when text or attachments exist", () => {
    useAgendaoStore.setState({
      composer: "",
      attachments: [],
      streaming: false,
    });

    renderComposerSection();

    const send = screen.getByTestId("composer-send");
    expect(send).toBeDisabled();

    fireEvent.change(screen.getByTestId("composer-input"), {
      target: { value: "hello agendao" },
    });
    expect(useAgendaoStore.getState().composer).toBe("hello agendao");
    expect(screen.getByTestId("composer-send")).not.toBeDisabled();

    useAgendaoStore.setState({
      composer: "",
      attachments: [{ type: "file", url: "file:///repo/notes.md", filename: "notes.md", mime: "text/markdown" }],
    });
    expect(screen.getByTestId("composer-send")).not.toBeDisabled();
  });

  it("disables composer input during streaming and restores it after streaming stops", () => {
    useAgendaoStore.setState({
      composer: "draft",
      attachments: [],
      streaming: true,
    });

    const { rerender } = renderComposerSection();

    expect(screen.getByTestId("composer-input")).toBeDisabled();

    useAgendaoStore.setState({ streaming: false });
    rerender(
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
        onSubmit={vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) => event.preventDefault())}
        onStopStreaming={vi.fn<() => void>()}
        onRemoveAttachment={vi.fn<(index: number) => void>()}
        onSelectAttachment={vi.fn<(index: number, attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
        onLocateAttachment={vi.fn<(attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
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

    expect(screen.getByTestId("composer-input")).not.toBeDisabled();
  });

  it("shows a stop button during streaming and routes clicks to the stop handler", () => {
    const modes: ExecutionMode[] = [{ id: "auto", name: "auto", kind: "preset" }];
    const providers: ProviderRecord[] = [{ id: "openai", name: "OpenAI", models: [] }];
    const onStopStreaming = vi.fn<() => void>();

    useAgendaoStore.setState({
      composer: "draft",
      attachments: [],
      streaming: true,
      modes,
      providers,
      selectedMode: "preset:auto",
      selectedModel: "",
      workspaceContext: null,
      selectedWorkspacePath: null,
    });

    render(
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
        onSubmit={vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) => event.preventDefault())}
        onStopStreaming={onStopStreaming}
        onRemoveAttachment={vi.fn<(index: number) => void>()}
        onSelectAttachment={vi.fn<(index: number, attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
        onLocateAttachment={vi.fn<(attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
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

    fireEvent.click(screen.getByTestId("composer-stop"));
    expect(onStopStreaming).toHaveBeenCalledTimes(1);
  });

  it("renders attach success notice inside the composer block", () => {
    useAgendaoStore.setState({
      composer: "",
      attachments: [],
      streaming: false,
    });

    render(
      <ComposerSection
        multimodalHints={[]}
        allowAudioInput={false}
        allowImageInput={true}
        allowFileInput={true}
        onModelChange={vi.fn<(value: string) => void>()}
        workspaceRootPath="/repo"
        composerNotice={{ id: 1, text: "Image ready: screenshot.png", count: 1 }}
        activeStageId={null}
        provenance={null}
        onSubmit={vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) => event.preventDefault())}
        onStopStreaming={vi.fn<() => void>()}
        onRemoveAttachment={vi.fn<(index: number) => void>()}
        onSelectAttachment={vi.fn<(index: number, attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
        onLocateAttachment={vi.fn<(attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
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

    expect(screen.getByTestId("composer-notice")).toHaveTextContent("Image ready: screenshot.png");
    expect(screen.getByTestId("composer-input")).toBeInTheDocument();
  });

  it("shows a count badge for multi-attachment notices", () => {
    useAgendaoStore.setState({
      composer: "",
      attachments: [],
      streaming: false,
    });

    render(
      <ComposerSection
        multimodalHints={[]}
        allowAudioInput={false}
        allowImageInput={true}
        allowFileInput={true}
        onModelChange={vi.fn<(value: string) => void>()}
        workspaceRootPath="/repo"
        composerNotice={{ id: 2, text: "3 images ready", count: 3 }}
        activeStageId={null}
        provenance={null}
        onSubmit={vi.fn<(event: React.FormEvent<HTMLFormElement>) => void>((event) => event.preventDefault())}
        onStopStreaming={vi.fn<() => void>()}
        onRemoveAttachment={vi.fn<(index: number) => void>()}
        onSelectAttachment={vi.fn<(index: number, attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
        onLocateAttachment={vi.fn<(attachment: { type: string; url?: string; filename?: string; mime?: string }) => void>()}
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

    expect(screen.getByTestId("composer-notice")).toHaveTextContent("3 images ready");
    expect(screen.getByText("3")).toBeInTheDocument();
  });
});
