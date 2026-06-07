import type { BreadcrumbProvenance } from "../../hooks/useSchedulerNavigation";

interface ProvenanceTrailProps {
  provenance: BreadcrumbProvenance | null;
  workspaceReference?: string | null;
  onNavigateSession: () => void;
  onNavigateStage: () => void;
  onNavigateToolCall: () => void;
}

export function ProvenanceTrail({
  provenance,
  workspaceReference = null,
  onNavigateSession,
  onNavigateStage,
  onNavigateToolCall,
}: ProvenanceTrailProps) {
  if (!provenance && !workspaceReference) return null;

  return (
    <div className="flex flex-wrap items-center gap-2 text-xs" data-testid="provenance-trail">
      {provenance ? (
        <>
          <span className="text-[10px] font-semibold uppercase tracking-[0.2em] text-muted-foreground">
            Source
          </span>
          <button
            className="roc-badge text-foreground hover:border-primary/35 hover:text-primary transition-colors"
            data-testid="provenance-session"
            type="button"
            onClick={onNavigateSession}
          >
            {provenance.sourceSessionTitle}
          </button>
          {provenance.stageId ? (
            <button
              className="roc-badge hover:border-primary/35 hover:text-primary transition-colors"
              data-testid="provenance-stage"
              type="button"
              onClick={onNavigateStage}
            >
              stage {provenance.stageId}
            </button>
          ) : null}
          {provenance.toolCallId ? (
            <button
              className="roc-badge hover:border-primary/35 hover:text-primary transition-colors"
              data-testid="provenance-tool"
              type="button"
              onClick={onNavigateToolCall}
            >
              tool {provenance.toolCallId}
            </button>
          ) : null}
          {provenance.label ? (
            <span className="text-xs text-muted-foreground">{provenance.label}</span>
          ) : null}
        </>
      ) : null}
      {workspaceReference ? (
        <span className="roc-badge max-w-[18rem] truncate" data-testid="provenance-workspace" title={`@${workspaceReference}`}>
          @{workspaceReference}
        </span>
      ) : null}
    </div>
  );
}
