import { useMemo, type ChangeEvent, type ClipboardEvent, type DragEvent, type FormEvent } from "react";
import { ComposerPanel } from "./ComposerPanel";
import type { BreadcrumbProvenance } from "../../hooks/useSchedulerNavigation";
import type { ComposerAttachmentRecord } from "../../lib/composerContext";
import { extractPromptReferences, removePromptReference } from "../../lib/composerContext";
import { modeKey } from "../../lib/display";
import { useAgendaoStore } from "../../store";

interface ComposerSectionProps {
  multimodalHints: Array<{ tone: "info" | "warning"; text: string }>;
  allowAudioInput: boolean;
  allowImageInput: boolean;
  allowFileInput: boolean;
  onModelChange: (value: string) => void;
  workspaceRootPath: string;
  contextTokensUsed?: number | null;
  contextTokensLimit?: number | null;
  lastTurnInputTokens?: number | null;
  lastTurnOutputTokens?: number | null;
  cacheReadTokens?: number | null;
  cacheMissTokens?: number | null;
  cacheWriteTokens?: number | null;
  closureDiagnosticLabel?: string | null;
  ingressDiagnosticLabel?: string | null;
  providerDiagnosticLabel?: string | null;
  inputPricePerMillion?: number | null;
  outputPricePerMillion?: number | null;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  permissionStatusLabel?: string | null;
  permissionStatusTone?: "muted" | "warning" | "destructive";
  onPreviewStage?: (stageId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onRemoveAttachment: (index: number) => void;
  onSelectAttachment: (index: number, attachment: ComposerAttachmentRecord) => void;
  onLocateAttachment: (attachment: ComposerAttachmentRecord) => void;
  onNavigateStage: (stageId: string) => void;
  onNavigateProvenanceSession: () => void;
  onNavigateProvenanceStage: () => void;
  onNavigateProvenanceToolCall: () => void;
  onDragEnter: (event: DragEvent<HTMLDivElement>) => void;
  onDragOver: (event: DragEvent<HTMLDivElement>) => void;
  onDragLeave: (event: DragEvent<HTMLDivElement>) => void;
  onDrop: (event: DragEvent<HTMLDivElement>) => void;
  onFileChange: (event: ChangeEvent<HTMLInputElement>) => void | Promise<void>;
  onPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void | Promise<void>;
}

export function ComposerSection(props: ComposerSectionProps) {
  const composer = useAgendaoStore((s) => s.composer);
  const setComposer = useAgendaoStore((s) => s.setComposer);
  const composerDragActive = useAgendaoStore((s) => s.composerDragActive);
  const streaming = useAgendaoStore((s) => s.streaming);
  const modes = useAgendaoStore((s) => s.modes);
  const selectedMode = useAgendaoStore((s) => s.selectedMode);
  const setSelectedMode = useAgendaoStore((s) => s.setSelectedMode);
  const providers = useAgendaoStore((s) => s.providers);
  const workspaceContext = useAgendaoStore((s) => s.workspaceContext);
  const selectedModel = useAgendaoStore((s) => s.selectedModel);
  const attachments = useAgendaoStore((s) => s.attachments);
  const selectedAttachmentIndex = useAgendaoStore((s) => s.selectedAttachmentIndex);
  const selectedWorkspacePath = useAgendaoStore((s) => s.selectedWorkspacePath);
  const modeOptions = useMemo(
    () =>
      modes.map((mode) => ({
        key: modeKey(mode),
        label: mode.kind === "agent" ? mode.name : `${mode.kind}:${mode.name}`,
      })),
    [modes],
  );
  const recentModels = useMemo(
    () => workspaceContext?.recent_models ?? [],
    [workspaceContext?.recent_models],
  );
  const references = useMemo(() => extractPromptReferences(composer), [composer]);
  const selectedAttachment =
    (selectedAttachmentIndex !== null && attachments[selectedAttachmentIndex]) ||
    attachments[attachments.length - 1] ||
    null;

  return (
    <div className="mx-auto w-full max-w-[88rem]">
      <ComposerPanel
        {...props}
        composer={composer}
        composerDragActive={composerDragActive}
        streaming={streaming}
        modeOptions={modeOptions}
        selectedMode={selectedMode}
        onModeChange={setSelectedMode}
        providers={providers}
        recentModels={recentModels}
        selectedModel={selectedModel}
        references={references}
        attachments={attachments}
        selectedAttachmentIndex={selectedAttachmentIndex}
        selectedAttachment={selectedAttachment}
        selectedWorkspacePath={selectedWorkspacePath}
        onRemoveReference={(reference) =>
          setComposer((current) => removePromptReference(current, reference))
        }
        onComposerChange={setComposer}
      />
    </div>
  );
}
