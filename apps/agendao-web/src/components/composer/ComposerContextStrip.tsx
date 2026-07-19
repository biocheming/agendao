import {
  attachmentContainsWorkspacePath,
  attachmentKind,
  attachmentLabel,
  attachmentSource,
  attachmentTone,
  attachmentWorkspacePath,
  toWorkspaceReferencePath,
  type ComposerAttachmentRecord,
} from "../../lib/composerContext";
import type { BreadcrumbProvenance } from "../../hooks/useSchedulerNavigation";
import { useI18n } from "@/i18n/I18nProvider";
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
  workspace: "bg-(--ds-ok)/10 border-(--ds-ok)/20",
  directory: "bg-(--ds-warn)/12 border-(--ds-warn)/20",
  image: "bg-(--ds-water)/10 border-(--ds-water)/20",
};

// Display-only translation keys for attachmentSource()/attachmentKind() values.
// The raw values stay in data-* attributes; only the rendered label is localized.
const ATTACHMENT_SOURCE_KEYS: Record<string, string> = {
  "inline image": "composer.attachmentSource.inlineImage",
  "inline file": "composer.attachmentSource.inlineFile",
  uploaded: "composer.attachmentSource.uploaded",
  workspace: "composer.attachmentSource.workspace",
  remote: "composer.attachmentSource.remote",
};

const ATTACHMENT_KIND_KEYS: Record<string, string> = {
  directory: "composer.attachmentKind.directory",
  image: "composer.attachmentKind.image",
  text: "composer.attachmentKind.text",
  file: "composer.attachmentKind.file",
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
  const { t } = useI18n();
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
          title={t("composer.removeReference", { reference })}
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
          const sourceLabel = attachmentSource(attachment);
          const kindLabel = attachmentKind(attachment);

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
                selected && "border-(--ds-wood)/30 shadow-inner shadow-(--ds-wood)/20",
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
                    ? t("composer.inspectLocateAttachment", { path: attachmentWorkspacePath(attachment)! })
                    : t("composer.inspectAttachment", { name: attachmentLabel(attachment) })
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
                    {t(ATTACHMENT_SOURCE_KEYS[sourceLabel] ?? sourceLabel)} · {t(ATTACHMENT_KIND_KEYS[kindLabel] ?? kindLabel)}
                    {attachmentWorkspacePath(attachment)
                      ? ` · ${toWorkspaceReferencePath(attachmentWorkspacePath(attachment)!, workspaceRootPath)}`
                      : ""}
                    {provenance ? ` · ${provenance.toolCallId ? t("composer.provenanceTool", { id: provenance.toolCallId }) : provenance.stageId ? t("composer.provenanceStage", { id: provenance.stageId }) : t("composer.provenanceSourceTrail")}` : ""}
                  </span>
                </span>
              </button>
              <button
                className="border-0 bg-transparent text-inherit inline-flex items-center justify-center w-7 h-7 p-0 cursor-pointer"
                type="button"
                data-testid="context-attachment-remove"
                onClick={() => onRemoveAttachment(index)}
                title={t("composer.removeAttachment", { name: attachmentLabel(attachment) })}
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
