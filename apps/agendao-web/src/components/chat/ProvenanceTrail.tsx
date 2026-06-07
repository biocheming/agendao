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
            className="text-xs text-foreground transition-colors hover:text-primary"
            data-testid="provenance-session"
            type="button"
            onClick={onNavigateSession}
          >
            {provenance.sourceSessionTitle}
          </button>
          {provenance.stageId ? (
            <button
              className="text-xs text-muted-foreground transition-colors hover:text-primary"
              data-testid="provenance-stage"
              type="button"
              onClick={onNavigateStage}
            >
              stage {provenance.stageId}
            </button>
          ) : null}
          {provenance.toolCallId ? (
            <button
              className="text-xs text-muted-foreground transition-colors hover:text-primary"
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
        <span className="max-w-[18rem] truncate text-xs text-muted-foreground" data-testid="provenance-workspace" title={`@${workspaceReference}`}>
          @{workspaceReference}
        </span>
      ) : null}
    </div>
  );
}
