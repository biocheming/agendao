import type { ChangeEvent, ClipboardEvent, DragEvent, FormEvent } from "react";
import { ComposerPanel } from "./ComposerPanel";
import type { BreadcrumbProvenance } from "../hooks/useSchedulerNavigation";
import type { ComposerAttachmentRecord } from "../lib/composerContext";
import type { ProviderRecord } from "../lib/provider";

interface ComposerSectionProps {
  composer: string;
  composerDragActive: boolean;
  streaming: boolean;
  multimodalHints: Array<{ tone: "info" | "warning"; text: string }>;
  allowAudioInput: boolean;
  allowImageInput: boolean;
  allowFileInput: boolean;
  modeOptions: Array<{ key: string; label: string }>;
  selectedMode: string;
  onModeChange: (value: string) => void;
  providers: ProviderRecord[];
  selectedModel: string;
  onModelChange: (value: string) => void;
  references: string[];
  attachments: ComposerAttachmentRecord[];
  selectedAttachmentIndex: number | null;
  selectedAttachment: ComposerAttachmentRecord | null;
  selectedWorkspacePath: string | null;
  workspaceRootPath: string;
  contextTokensUsed?: number | null;
  contextTokensLimit?: number | null;
  sessionCost?: number | null;
  inputPricePerMillion?: number | null;
  outputPricePerMillion?: number | null;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  onPreviewStage?: (stageId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onRemoveReference: (reference: string) => void;
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
  onComposerChange: (value: string) => void;
}

export function ComposerSection(props: ComposerSectionProps) {
  return (
    <div className="mx-auto w-full max-w-[76rem]">
      <ComposerPanel {...props} />
    </div>
  );
}
