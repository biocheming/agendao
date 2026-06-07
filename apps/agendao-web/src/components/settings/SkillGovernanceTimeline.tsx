"use client";

import { useEffect, useMemo, useState } from "react";
import type {
  ManagedSkillRecord,
  SkillGovernanceTimelineEntryRecord,
} from "@/lib/skill";
import { cn } from "@/lib/utils";

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
      return "border-green-300 bg-green-50 text-green-800 dark:border-green-700 dark:bg-green-950/60 dark:text-green-300";
    case "warn":
      return "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200";
    case "error":
      return "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950/60 dark:text-red-300";
    default:
      return "border-border bg-muted text-muted-foreground";
  }
}

function managedStateLabel(record: ManagedSkillRecord): string {
  if (record.deleted_locally) return "deleted locally";
  if (record.locally_modified) return "locally modified";
  return "clean";
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
            Governance Timeline
          </p>
          <h3 className="m-0 mt-1">Guard, audit, and managed provenance in one read model</h3>
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
            All · {entries.length}
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
            Skill · {counts.skill}
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
            Source · {counts.source}
          </button>
        </div>
      </div>

      <div className="text-sm text-muted-foreground">
        {scope === "skill" && selectedSkillName ? (
          <span>Focused on selected skill <code>{selectedSkillName}</code>.</span>
        ) : null}
        {scope === "source" && selectedSourceId ? (
          <span>Focused on selected source <code>{selectedSourceId}</code>.</span>
        ) : null}
        {scope === "all" ? (
          <span>Showing the current workspace governance history tail.</span>
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
                  entry.skill_name ? `skill ${entry.skill_name}` : null,
                  entry.source_id ? `source ${entry.source_id}` : null,
                  entry.actor ? `actor ${entry.actor}` : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </div>

              {entry.managed_record ? (
                <div className="mt-3 rounded-lg border border-border/35 bg-background/65 p-3 text-xs text-muted-foreground">
                  <div>
                    managed revision {entry.managed_record.installed_revision || "--"} ·{" "}
                    {managedStateLabel(entry.managed_record)}
                  </div>
                  <div className="mt-1 break-all">
                    locator {entry.managed_record.source?.locator || "--"}
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
            No timeline entries for the current focus.
          </div>
        )}
      </div>
    </div>
  );
}
