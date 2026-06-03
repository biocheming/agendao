import {
  attachmentContainsWorkspacePath,
  attachmentKind,
  attachmentLabel,
  attachmentSource,
  attachmentTone,
  attachmentWorkspacePath,
  toWorkspaceReferencePath,
  type ComposerAttachmentRecord,
} from "../lib/composerContext";
import type { BreadcrumbProvenance } from "../hooks/useSchedulerNavigation";
import { cn } from "@/lib/utils";

interface ComposerContextStripProps {
  references: string[];
  attachments: ComposerAttachmentRecord[];
  selectedAttachmentIndex: number | null;
  selectedWorkspacePath: string | null;
  workspaceRootPath: string;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  onRemoveReference: (reference: string) => void;
  onRemoveAttachment: (index: number) => void;
  onSelectAttachment: (index: number, attachment: ComposerAttachmentRecord) => void;
  onPreviewStage?: (stageId: string | null) => void;
}

const toneClassMap: Record<string, string> = {
  reference: "bg-primary/10 border-primary/20",
  workspace: "bg-green-500/10 border-green-500/20",
  directory: "bg-amber-600/12 border-amber-600/20",
  image: "bg-purple-500/10 border-purple-500/20",
};

export function ComposerContextStrip({
  references,
  attachments,
  selectedAttachmentIndex,
  selectedWorkspacePath,
  workspaceRootPath,
  activeStageId,
  provenance,
  onRemoveReference,
  onRemoveAttachment,
  onSelectAttachment,
  onPreviewStage,
}: ComposerContextStripProps) {
  if (references.length === 0 && attachments.length === 0) {
    return null;
  }

  return (
    <div className="flex flex-wrap gap-2" data-testid="context-strip">
      {references.map((reference) => (
        <button
          key={`reference:${reference}`}
          className="min-h-9 rounded-full border border-border bg-card/75 text-foreground inline-flex items-center gap-2.5 pr-1.5 bg-primary/10 border-primary/20"
          type="button"
          data-testid="context-reference-chip"
          data-reference={reference}
          onClick={() => onRemoveReference(reference)}
          title={`Remove @${reference}`}
        >
          <span className="max-w-60 overflow-hidden text-ellipsis whitespace-nowrap">@{reference}</span>
          <span className="context-chip-remove">×</span>
        </button>
      ))}

      {attachments.map((attachment, index) =>
        (() => {
          const workspaceLinked = attachmentContainsWorkspacePath(attachment, selectedWorkspacePath);
          const selected = selectedAttachmentIndex === index;
          const hoverStageId = activeStageId && (selected || workspaceLinked)
            ? activeStageId
            : selected
              ? provenance?.stageId ?? null
              : null;

          const tone = attachmentTone(attachment);

          return (
            <div
              key={`attachment:${attachmentLabel(attachment)}:${index}`}
              data-testid="context-attachment-chip"
              data-index={index}
              data-source={attachmentSource(attachment)}
              data-kind={attachmentKind(attachment)}
              data-workspace-path={attachmentWorkspacePath(attachment) ?? ""}
              className={cn(
                "min-h-9 rounded-full border border-border bg-card/75 text-foreground inline-flex items-center gap-2.5 pr-1.5",
                toneClassMap[tone],
                workspaceLinked && "border-primary/30 shadow-inner shadow-primary/20",
                selected && "border-amber-600/30 shadow-inner shadow-amber-600/20",
              )}
              onMouseEnter={() => hoverStageId ? onPreviewStage?.(hoverStageId) : undefined}
              onMouseLeave={() => hoverStageId ? onPreviewStage?.(null) : undefined}
            >
              <button
                className="border-0 bg-transparent text-inherit inline-flex items-center gap-2.5 pl-3 cursor-pointer"
                type="button"
                data-testid="context-attachment-main"
                onClick={() => onSelectAttachment(index, attachment)}
                title={
                  attachmentWorkspacePath(attachment)
                    ? `${attachmentWorkspacePath(attachment)}\nClick to inspect and locate in workspace`
                    : `Inspect ${attachmentLabel(attachment)}`
                }
              >
                {tone === "image" && attachment.url?.startsWith("data:image/") ? (
                  <img
                    className="context-chip-preview"
                    src={attachment.url}
                    alt={attachmentLabel(attachment)}
                  />
                ) : null}
                <span className="context-chip-body">
                  <span className="max-w-60 overflow-hidden text-ellipsis whitespace-nowrap">{attachmentLabel(attachment)}</span>
                  <span className="context-chip-meta">
                    {attachmentSource(attachment)} · {attachmentKind(attachment)}
                    {attachmentWorkspacePath(attachment)
                      ? ` · ${toWorkspaceReferencePath(attachmentWorkspacePath(attachment)!, workspaceRootPath)}`
                      : ""}
                    {provenance ? ` · ${provenance.toolCallId ? `tool ${provenance.toolCallId}` : provenance.stageId ? `stage ${provenance.stageId}` : "source trail"}` : ""}
                  </span>
                </span>
              </button>
              <button
                className="border-0 bg-transparent text-inherit inline-flex items-center justify-center w-7 h-7 p-0 cursor-pointer"
                type="button"
                data-testid="context-attachment-remove"
                onClick={() => onRemoveAttachment(index)}
                title={`Remove ${attachmentLabel(attachment)}`}
              >
                <span className="context-chip-remove">×</span>
              </button>
            </div>
          );
        })()
      )}
    </div>
  );
}
