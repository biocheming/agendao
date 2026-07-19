"use client";

import { useEffect, useMemo, useState } from "react";
import type {
  ManagedSkillRecord,
  SkillGovernanceTimelineEntryRecord,
} from "@/lib/skill";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";

type TimelineScope = "all" | "skill" | "source";

interface SkillGovernanceTimelineProps {
  entries: SkillGovernanceTimelineEntryRecord[];
  selectedSkillName?: string | null;
  selectedSourceId?: string | null;
}

function formatTimestamp(ts: number): string {
  if (!ts) return "timestamp --";
  return new Date(ts * 1000).toLocaleString();
}

function statusClasses(status: SkillGovernanceTimelineEntryRecord["status"]): string {
  switch (status) {
    case "success":
      return "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)";
    case "warn":
      return "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)";
    case "error":
      return "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)";
    default:
      return "border-border bg-muted text-muted-foreground";
  }
}

function managedStateLabel(record: ManagedSkillRecord, t: (key: string) => string): string {
  if (record.deleted_locally) return t("settings.skills.state.deletedLocally");
  if (record.locally_modified) return t("settings.skills.state.locallyModified");
  return t("settings.skills.state.clean");
}

function matchesSkill(
  entry: SkillGovernanceTimelineEntryRecord,
  selectedSkillName: string | null | undefined,
): boolean {
  if (!selectedSkillName?.trim()) return false;
  return entry.skill_name?.trim().toLowerCase() === selectedSkillName.trim().toLowerCase();
}

function matchesSource(
  entry: SkillGovernanceTimelineEntryRecord,
  selectedSourceId: string | null | undefined,
): boolean {
  if (!selectedSourceId?.trim()) return false;
  return entry.source_id?.trim() === selectedSourceId.trim();
}

export function SkillGovernanceTimeline({
  entries,
  selectedSkillName,
  selectedSourceId,
}: SkillGovernanceTimelineProps) {
  const { t } = useI18n();
  const [scope, setScope] = useState<TimelineScope>("all");

  const counts = useMemo(() => {
    let skill = 0;
    let source = 0;
    for (const entry of entries) {
      if (matchesSkill(entry, selectedSkillName)) skill += 1;
      if (matchesSource(entry, selectedSourceId)) source += 1;
    }
    return { skill, source };
  }, [entries, selectedSkillName, selectedSourceId]);

  useEffect(() => {
    if (scope === "skill" && counts.skill === 0) {
      setScope(counts.source > 0 ? "source" : "all");
      return;
    }
    if (scope === "source" && counts.source === 0) {
      setScope(counts.skill > 0 ? "skill" : "all");
    }
  }, [counts.skill, counts.source, scope]);

  const filteredEntries = useMemo(() => {
    if (scope === "skill") {
      return entries.filter((entry) => matchesSkill(entry, selectedSkillName));
    }
    if (scope === "source") {
      return entries.filter((entry) => matchesSource(entry, selectedSourceId));
    }
    return entries;
  }, [entries, scope, selectedSkillName, selectedSourceId]);

  return (
    <div className="roc-panel grid gap-4 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="m-0 text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.timeline.title")}
          </p>
          <h3 className="m-0 mt-1">{t("settings.skills.timeline.subtitle")}</h3>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className={cn(
              "min-h-[34px] rounded-lg px-4 text-sm transition-colors",
              scope === "all"
                ? "bg-foreground text-background"
                : "border border-transparent bg-transparent text-foreground hover:bg-accent",
            )}
            onClick={() => setScope("all")}
          >
            {t("settings.skills.timeline.scopeAll", { count: entries.length })}
          </button>
          <button
            type="button"
            disabled={counts.skill === 0}
            className={cn(
              "min-h-[34px] rounded-lg px-4 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50",
              scope === "skill"
                ? "bg-foreground text-background"
                : "border border-transparent bg-transparent text-foreground hover:bg-accent",
            )}
            onClick={() => setScope("skill")}
          >
            {t("settings.skills.timeline.scopeSkill", { count: counts.skill })}
          </button>
          <button
            type="button"
            disabled={counts.source === 0}
            className={cn(
              "min-h-[34px] rounded-lg px-4 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50",
              scope === "source"
                ? "bg-foreground text-background"
                : "border border-transparent bg-transparent text-foreground hover:bg-accent",
            )}
            onClick={() => setScope("source")}
          >
            {t("settings.skills.timeline.scopeSource", { count: counts.source })}
          </button>
        </div>
      </div>

      <div className="text-sm text-muted-foreground">
        {scope === "skill" && selectedSkillName ? (
          <span>{t("settings.skills.timeline.focusSkill", { name: selectedSkillName })}</span>
        ) : null}
        {scope === "source" && selectedSourceId ? (
          <span>{t("settings.skills.timeline.focusSource", { id: selectedSourceId })}</span>
        ) : null}
        {scope === "all" ? (
          <span>{t("settings.skills.timeline.showAll")}</span>
        ) : null}
      </div>

      <div className="grid gap-3 max-h-[34rem] overflow-y-auto pr-1">
        {filteredEntries.length ? (
          filteredEntries.map((entry) => (
            <article
              key={entry.entry_id}
              className="rounded-xl border border-border/35 bg-muted/8 p-4 text-sm"
            >
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="grid gap-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <strong>{entry.title}</strong>
                    <span
                      className={cn(
                        "rounded-full border px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wide",
                        statusClasses(entry.status),
                      )}
                    >
                      {entry.status}
                    </span>
                    <span className="text-[11px] uppercase tracking-wide text-muted-foreground">
                      {entry.kind}
                    </span>
                  </div>
                  <div className="text-muted-foreground">{entry.summary}</div>
                </div>
                <span className="text-xs text-muted-foreground">
                  {formatTimestamp(entry.created_at)}
                </span>
              </div>

              <div className="mt-3 text-xs text-muted-foreground">
                {[
                  entry.skill_name ? t("settings.skills.skillPrefix", { name: entry.skill_name }) : null,
                  entry.source_id ? t("settings.skills.sourcePrefix", { value: entry.source_id }) : null,
                  entry.actor ? t("settings.skills.timeline.actorPrefix", { name: entry.actor }) : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </div>

              {entry.managed_record ? (
                <div className="mt-3 rounded-lg border border-border/35 bg-background/65 p-3 text-xs text-muted-foreground">
                  <div>
                    {t("settings.skills.timeline.managedRevision", {
                      revision: entry.managed_record.installed_revision || "--",
                      state: managedStateLabel(entry.managed_record, t),
                    })}
                  </div>
                  <div className="mt-1 break-all">
                    {t("settings.skills.timeline.locatorLine", { value: entry.managed_record.source?.locator || "--" })}
                  </div>
                </div>
              ) : null}

              {entry.guard_report?.violations?.length ? (
                <div className="mt-3 grid gap-2">
                  {entry.guard_report.violations.slice(0, 3).map((violation, index) => (
                    <div
                      key={`${entry.entry_id}:${violation.rule_id}:${index}`}
                      className="rounded-xl border border-border/70 bg-card/70 p-3 text-xs"
                    >
                      <div className="flex items-center gap-2">
                        <strong>{violation.rule_id}</strong>
                        <span className="text-muted-foreground">{violation.severity}</span>
                      </div>
                      <div className="mt-1 text-muted-foreground">{violation.message}</div>
                      {violation.file_path ? (
                        <div className="mt-1 break-all text-muted-foreground">
                          {violation.file_path}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : null}
            </article>
          ))
        ) : (
          <div className="rounded-xl border border-border bg-muted/10 p-4 text-sm text-muted-foreground">
            {t("settings.skills.timeline.empty")}
          </div>
        )}
      </div>
    </div>
  );
}
