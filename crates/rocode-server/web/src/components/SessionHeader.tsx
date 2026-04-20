import type { BreadcrumbProvenance, SessionBreadcrumb } from "../hooks/useSchedulerNavigation";
import { ProvenanceTrail } from "./ProvenanceTrail";

interface SessionHeaderProps {
  title: string;
  subtitle?: string | null;
  pathLabel?: string | null;
  workspaceLabel?: string | null;
  contextSummary?: string | null;
  contextTitle?: string | null;
  modeLabel?: string | null;
  modelLabel?: string | null;
  activeStageId: string | null;
  currentWorkspaceReference?: string | null;
  breadcrumbs: SessionBreadcrumb[];
  provenance: BreadcrumbProvenance | null;
  onNavigateStage: (stageId: string) => void;
  onNavigateBreadcrumb: (sessionId: string) => void;
  onNavigateProvenanceSession: () => void;
  onNavigateProvenanceStage: () => void;
  onNavigateProvenanceToolCall: () => void;
}

export function SessionHeader({
  title,
  subtitle = null,
  pathLabel = null,
  workspaceLabel = null,
  contextSummary = null,
  contextTitle = null,
  modeLabel = null,
  modelLabel = null,
  activeStageId,
  currentWorkspaceReference = null,
  breadcrumbs,
  provenance,
  onNavigateStage,
  onNavigateBreadcrumb,
  onNavigateProvenanceSession,
  onNavigateProvenanceStage,
  onNavigateProvenanceToolCall,
}: SessionHeaderProps) {
  const showTrace = breadcrumbs.length > 1 || Boolean(provenance);
  const secondaryMeta = subtitle?.trim() || null;
  const modeModelSummary = [modeLabel?.trim(), modelLabel?.trim()].filter(Boolean).join(" · ") || null;

  return (
    <header className="roc-session-header grid gap-1.5" data-testid="session-header">
      <div className="flex flex-col gap-1 xl:flex-row xl:items-center xl:justify-between">
        <div className="min-w-0 flex-1">
          <h1 className="min-w-0 truncate text-xl font-semibold tracking-[-0.02em] text-foreground">
            {title}
          </h1>
          <div className="mt-0.5 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
            {secondaryMeta ? <span className="truncate">{secondaryMeta}</span> : null}
            {modeModelSummary ? (
              <>
                {secondaryMeta ? <span className="text-border">·</span> : null}
                <span className="truncate">{modeModelSummary}</span>
              </>
            ) : null}
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-1.5 xl:justify-end">
          {contextSummary ? (
            <span className="roc-badge" title={contextTitle || contextSummary}>
              {contextSummary}
            </span>
          ) : null}
          {activeStageId ? (
            <button
              className="rounded-full border border-primary/22 bg-primary/8 px-2.5 py-1 text-[11px] font-semibold tracking-tight text-foreground transition-colors hover:border-primary/35 hover:bg-primary/12"
              type="button"
              onClick={() => onNavigateStage(activeStageId)}
            >
              stage {activeStageId}
            </button>
          ) : null}
        </div>
      </div>

      {breadcrumbs.length > 1 ? (
        <nav
          className="flex flex-wrap gap-1.5"
          data-testid="session-breadcrumbs"
          aria-label="Session breadcrumbs"
        >
          {breadcrumbs.map((crumb, index) => (
            <div
              key={`${crumb.sessionId}:${index}`}
              className="inline-flex items-center gap-1.5 rounded-full border border-border/50 bg-background/62 px-2.5 py-1"
            >
              <button
                className="border-0 bg-transparent p-0 text-[12px] text-foreground transition-colors hover:text-primary"
                type="button"
                data-testid="session-breadcrumb"
                data-session-id={crumb.sessionId}
                onClick={() => onNavigateBreadcrumb(crumb.sessionId)}
              >
                {crumb.title}
              </button>
              {crumb.viaLabel ? (
                <span className="text-[10px] text-muted-foreground">
                  {crumb.viaLabel}
                </span>
              ) : null}
            </div>
          ))}
        </nav>
      ) : null}

      {showTrace ? (
        <div className="border-t border-border/40 pt-2">
          <ProvenanceTrail
            provenance={provenance}
            workspaceReference={provenance ? currentWorkspaceReference : null}
            onNavigateSession={onNavigateProvenanceSession}
            onNavigateStage={onNavigateProvenanceStage}
            onNavigateToolCall={onNavigateProvenanceToolCall}
          />
        </div>
      ) : null}
    </header>
  );
}
