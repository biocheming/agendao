import {
  attachmentDownloadUrl,
  attachmentHighlightedHtml,
  attachmentKind,
  attachmentLabel,
  attachmentLanguageLabel,
  attachmentLooksLikeCode,
  attachmentPreviewUrl,
  attachmentSource,
  attachmentTextPreview,
  attachmentTextPreviewState,
  attachmentWorkspacePath,
  toWorkspaceReferencePath,
  type ComposerAttachmentRecord,
} from "../../lib/composerContext";
import type { BreadcrumbProvenance } from "../../hooks/useSchedulerNavigation";
import { ProvenanceTrail } from "./ProvenanceTrail";

interface AttachmentDetailsPanelProps {
  attachment: ComposerAttachmentRecord | null;
  workspaceRootPath: string;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  onLocateAttachment: (attachment: ComposerAttachmentRecord) => void;
  onNavigateStage: (stageId: string) => void;
  onNavigateProvenanceSession: () => void;
  onNavigateProvenanceStage: () => void;
  onNavigateProvenanceToolCall: () => void;
}

export function AttachmentDetailsPanel({
  attachment,
  workspaceRootPath,
  activeStageId,
  provenance,
  onLocateAttachment,
  onNavigateStage,
  onNavigateProvenanceSession,
  onNavigateProvenanceStage,
  onNavigateProvenanceToolCall,
}: AttachmentDetailsPanelProps) {
  if (!attachment) {
    return null;
  }

  const workspacePath = attachmentWorkspacePath(attachment);
  const previewUrl = attachmentPreviewUrl(attachment);
  const textPreview = attachmentTextPreview(attachment);
  const { preview: textPreviewExcerpt, truncated: textPreviewTruncated } =
    attachmentTextPreviewState(attachment);
  const codeLike = attachmentLooksLikeCode(attachment);
  const languageLabel = attachmentLanguageLabel(attachment);
  const textPreviewUrl = attachmentKind(attachment) === "text" ? attachmentDownloadUrl(attachment) : null;
  const relativePath = workspacePath
    ? toWorkspaceReferencePath(workspacePath, workspaceRootPath)
    : null;

  return (
    <div className="roc-panel p-4 grid gap-3.5" data-testid="attachment-details">
      <ProvenanceTrail
        provenance={provenance}
        workspaceReference={relativePath}
        onNavigateSession={onNavigateProvenanceSession}
        onNavigateStage={onNavigateProvenanceStage}
        onNavigateToolCall={onNavigateProvenanceToolCall}
      />
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Attachment</p>
          <h3>{attachmentLabel(attachment)}</h3>
        </div>
        <div className="flex items-center flex-wrap gap-2.5">
          {languageLabel ? <span className="rounded-full border border-primary/20 bg-primary/10 px-3 py-2 text-xs font-bold uppercase tracking-wider">{languageLabel}</span> : null}
          <span className="roc-badge px-3 py-1.5 text-xs">{attachmentSource(attachment)}</span>
          {activeStageId ? (
            <button className="text-xs text-primary underline underline-offset-2 cursor-pointer hover:text-primary/80 border-0 bg-transparent p-0" type="button" onClick={() => onNavigateStage(activeStageId)}>
              stage {activeStageId}
            </button>
          ) : null}
          {workspacePath ? (
            <button
              className="roc-action min-h-[42px] px-4 cursor-pointer transition-colors"
              type="button"
              data-testid="attachment-locate"
              onClick={() => onLocateAttachment(attachment)}
            >
              Locate
            </button>
          ) : null}
        </div>
      </div>

      {previewUrl ? (
        <div className="rounded-xl border border-border/35 bg-background/65 p-3">
          <img className="max-w-full max-h-80 rounded-xl object-contain" src={previewUrl} alt={attachmentLabel(attachment)} />
        </div>
      ) : textPreviewExcerpt ? (
        <div className="rounded-xl border border-border/35 bg-background/65 p-3">
          {codeLike ? (
            <pre
              className="font-mono text-sm leading-relaxed rounded-xl bg-foreground/90 text-background p-3"
              dangerouslySetInnerHTML={{ __html: attachmentHighlightedHtml(textPreviewExcerpt) }}
            />
          ) : (
            <pre className="font-mono text-sm leading-relaxed whitespace-pre-wrap break-words">{textPreviewExcerpt}</pre>
          )}
          {textPreviewTruncated && textPreview ? (
            <details className="mt-3 grid gap-2">
              <summary>Show full text</summary>
              {codeLike ? (
                <pre
                  className="font-mono text-sm leading-relaxed rounded-xl bg-foreground/90 text-background p-3"
                  dangerouslySetInnerHTML={{ __html: attachmentHighlightedHtml(textPreview) }}
                />
              ) : (
                <pre className="font-mono text-sm leading-relaxed whitespace-pre-wrap break-words">{textPreview}</pre>
              )}
            </details>
          ) : null}
        </div>
      ) : textPreviewUrl ? (
        <div className="rounded-xl border border-border/35 bg-background/65 p-3">
          <iframe
            className="w-full min-h-60 border-0 rounded-xl"
            src={textPreviewUrl}
            title={`${attachmentLabel(attachment)} preview`}
          />
        </div>
      ) : (
        <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground py-8">
          <strong>No inline preview</strong>
          <span>This attachment is available as context and metadata only.</span>
        </div>
      )}

      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
        <div>
          <dt>Kind</dt>
          <dd>{attachmentKind(attachment)}</dd>
        </div>
        {languageLabel ? (
          <div>
            <dt>Language</dt>
            <dd>{languageLabel}</dd>
          </div>
        ) : null}
        <div>
          <dt>MIME</dt>
          <dd>{attachment.mime || "unknown"}</dd>
        </div>
        {relativePath ? (
          <div>
            <dt>Workspace Ref</dt>
            <dd>{relativePath}</dd>
          </div>
        ) : null}
        {workspacePath ? (
          <div>
            <dt>Workspace Path</dt>
            <dd>{workspacePath}</dd>
          </div>
        ) : null}
        {activeStageId ? (
          <div>
            <dt>Stage</dt>
            <dd>
              <button className="text-xs text-primary underline underline-offset-2 cursor-pointer hover:text-primary/80 border-0 bg-transparent p-0" type="button" onClick={() => onNavigateStage(activeStageId)}>
                {activeStageId}
              </button>
            </dd>
          </div>
        ) : null}
      </dl>
    </div>
  );
}
