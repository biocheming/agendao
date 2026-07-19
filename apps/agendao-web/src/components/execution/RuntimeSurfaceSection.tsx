import type { Dispatch, SetStateAction } from "react";
import { useI18n } from "../../i18n/I18nProvider";
import {
  runtimeSurfaceDebugDetail,
  runtimeSurfaceLabel,
  runtimeSurfacePhase,
  runtimeSurfacePreview,
} from "../../lib/display";
import type { RuntimeSurfaceOutputBlock } from "../../lib/history";
import { cn } from "../../lib/utils";
import type { SessionRuntimeSurface } from "../../store/types";

export type RuntimeSurfaceTab = "queue" | "session" | "inspect";

export interface RuntimeSurfaceTabView {
  key: RuntimeSurfaceTab;
  label: string;
  count: number;
  blocks: RuntimeSurfaceOutputBlock[];
}

function RuntimeSurfaceList({
  title,
  blocks,
  emptyLabel,
}: {
  title: string;
  blocks: RuntimeSurfaceOutputBlock[];
  emptyLabel: string;
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-border/40 bg-background/78">
      <div className="flex items-center justify-between border-b border-border/35 px-3 py-2.5">
        <h3 className="text-sm font-medium text-foreground">{title}</h3>
        <span className="text-xs text-muted-foreground">{blocks.length}</span>
      </div>
      {blocks.length === 0 ? (
        <div className="px-3 py-5 text-sm text-muted-foreground">{emptyLabel}</div>
      ) : (
        <div className="max-h-[124px] space-y-2 overflow-auto px-3 py-2.5">
          {blocks.map((block) => {
            const preview = runtimeSurfacePreview(block);
            const phase = runtimeSurfacePhase(block);
            return (
              <article
                key={block.id ?? `${block.kind}:${block.event ?? block.title ?? preview ?? Math.random()}`}
                className="rounded-lg border border-border/30 bg-card/70 px-2.5 py-2"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">
                    {runtimeSurfaceLabel(block)}
                  </span>
                  {phase ? (
                    <span className="roc-badge px-2 py-0.5 text-[11px]">{phase}</span>
                  ) : null}
                </div>
                {preview ? (
                  <p className="mt-2 whitespace-pre-wrap text-sm leading-6 text-muted-foreground">
                    {preview}
                  </p>
                ) : null}
                {runtimeSurfaceDebugDetail(block) ? (
                  <p className="mt-2 whitespace-pre-wrap text-xs leading-5 text-muted-foreground/80">
                    {runtimeSurfaceDebugDetail(block)}
                  </p>
                ) : null}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}

export function RuntimeSurfaceSection({
  activeRuntimeSurfaceTab,
  currentRuntimeSurface,
  hasCurrentRuntimeSurface,
  hasRuntimeSurfaceContent,
  runtimeSurfaceExpanded,
  runtimeSurfaceSummary,
  runtimeSurfaceTabs,
  selectedSessionId,
  setRuntimeSurfaceExpanded,
  setRuntimeSurfaceTab,
}: {
  activeRuntimeSurfaceTab: RuntimeSurfaceTabView;
  currentRuntimeSurface: SessionRuntimeSurface;
  hasCurrentRuntimeSurface: boolean;
  hasRuntimeSurfaceContent: boolean;
  runtimeSurfaceExpanded: boolean;
  runtimeSurfaceSummary: string;
  runtimeSurfaceTabs: RuntimeSurfaceTabView[];
  selectedSessionId: string | null;
  setRuntimeSurfaceExpanded: Dispatch<SetStateAction<boolean>>;
  setRuntimeSurfaceTab: Dispatch<SetStateAction<RuntimeSurfaceTab>>;
}) {
  const { t } = useI18n();

  if (!selectedSessionId || !hasCurrentRuntimeSurface || !hasRuntimeSurfaceContent) {
    return null;
  }

  return (
    <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
      <div
        className="roc-panel max-h-[240px] overflow-hidden px-0 py-0"
        data-testid="runtime-surface"
        data-expanded={runtimeSurfaceExpanded ? "true" : "false"}
      >
        <button
          type="button"
          data-testid="runtime-surface-toggle"
          className="flex h-10 w-full items-center justify-between gap-3 px-4 text-left"
          aria-expanded={runtimeSurfaceExpanded}
          title={runtimeSurfaceExpanded ? t("app.runtimeSurfaceHideDetails") : t("app.runtimeSurfaceDetails")}
          onClick={() => setRuntimeSurfaceExpanded((value) => !value)}
        >
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium text-foreground">{runtimeSurfaceSummary}</p>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {runtimeSurfaceTabs.map((tab) =>
              tab.count > 0 ? (
                <span key={tab.key} className="roc-badge px-2 py-0.5 text-[11px]">
                  {tab.label} {tab.count}
                </span>
              ) : null,
            )}
          </div>
        </button>
        {runtimeSurfaceExpanded ? (
          <div
            className="max-h-[196px] overflow-hidden border-t border-border/40 px-3 pb-3 pt-2.5"
            data-testid="runtime-surface-expanded"
          >
            <div className="mb-2 flex flex-wrap items-center gap-1.5" data-testid="runtime-surface-tabs">
              {runtimeSurfaceTabs.map((tab) => (
                <button
                  key={tab.key}
                  type="button"
                  data-testid={`runtime-surface-tab-${tab.key}`}
                  className={cn(
                    "inline-flex h-7 items-center rounded-full px-2.5 text-[11px] font-medium transition-colors",
                    activeRuntimeSurfaceTab.key === tab.key
                      ? "bg-foreground/8 text-foreground"
                      : "text-muted-foreground hover:bg-accent/45 hover:text-foreground",
                  )}
                  onClick={() => setRuntimeSurfaceTab(tab.key)}
                >
                  {tab.label}
                </button>
              ))}
            </div>
            {currentRuntimeSurface.banner ? (
              <div
                className="mb-2 rounded-lg border border-(--ds-warn)/25 bg-(--ds-warn)/8 px-3 py-2 text-sm leading-5 text-(--ds-warn)"
                data-testid="runtime-surface-banner"
              >
                {currentRuntimeSurface.banner}
              </div>
            ) : null}
            <RuntimeSurfaceList
              title={activeRuntimeSurfaceTab.label}
              blocks={activeRuntimeSurfaceTab.blocks}
              emptyLabel={t("app.noEventsYet")}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}
