import { useState } from "react";
import { ChevronDownIcon } from "lucide-react";
import type { BreadcrumbProvenance, SessionBreadcrumb } from "../../hooks/useSchedulerNavigation";
import { ProvenanceTrail } from "../chat/ProvenanceTrail";

interface SessionHeaderProps {
  title: string;
  subtitle?: string | null;
  pathLabel?: string | null;
  workspaceLabel?: string | null;
  usageSummary?: string | null;
  usageTitle?: string | null;
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
  usageSummary = null,
  usageTitle = null,
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
  const [traceExpanded, setTraceExpanded] = useState(false);
  const hasMetaRow = Boolean(secondaryMeta) || Boolean(modeModelSummary) || showTrace;

  return (
    <header className="roc-session-header grid gap-1" data-testid="session-header">
      <div className="flex flex-col gap-1">
        <div className="min-w-0 flex-1">
          <h1 className="roc-session-title">{title}</h1>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-1">
          {usageSummary ? (
            <span className="roc-badge" title={usageTitle || usageSummary}>
              {usageSummary}
            </span>
          ) : null}
          {activeStageId ? (
            <button
              className="roc-accent-chip"
              type="button"
              onClick={() => onNavigateStage(activeStageId)}
            >
              stage {activeStageId}
            </button>
          ) : null}
        </div>
      </div>

      {hasMetaRow ? (
        <div className="flex min-w-0 items-center justify-between gap-2 text-[11px]">
          <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden whitespace-nowrap text-muted-foreground">
            {secondaryMeta ? (
              <span className="min-w-0 flex-1 truncate" title={secondaryMeta}>
                {secondaryMeta}
              </span>
            ) : null}
            {modeModelSummary ? (
              <span className="hidden max-w-[24rem] shrink-0 truncate lg:inline" title={modeModelSummary}>
                {modeModelSummary}
              </span>
            ) : null}
          </div>
          {showTrace ? (
            <button
              type="button"
              className="roc-badge shrink-0 gap-1.5 px-2.5 py-1"
              aria-expanded={traceExpanded}
              onClick={() => setTraceExpanded((value) => !value)}
            >
              <span>Trace</span>
              <ChevronDownIcon className={traceExpanded ? "size-3 rotate-180" : "size-3"} />
            </button>
          ) : null}
        </div>
      ) : null}

      {showTrace && traceExpanded ? (
        <div className="grid gap-2 border-t border-border/40 pt-2">
          {breadcrumbs.length > 1 ? (
            <nav
              className="flex flex-wrap gap-1.5"
              data-testid="session-breadcrumbs"
              aria-label="Session breadcrumbs"
            >
              {breadcrumbs.map((crumb, index) => (
                <div
                  key={`${crumb.sessionId}:${index}`}
                  className="roc-badge gap-1.5 px-2.5 py-1"
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
