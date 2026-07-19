import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  SkillGovernanceTimelineEntryRecord,
  SkillGuardReportRecord,
  SkillNegativeEntropyDiagnosticRecord,
  SkillOperationalSnapshotRecord,
  SkillSemanticConflictDiagnosticRecord,
} from "@/lib/skill";
import { SkillGovernanceTimeline } from "../../settings/SkillGovernanceTimeline";
import type { SkillReviewCandidateView } from "./shared";
import {
  formatVitalityStateLabel,
  governanceSeverityClass,
  latestGuardStatusLabel,
  negativeEntropySignalLabel,
  reviewReasonKindClass,
  reviewReasonKindLabel,
  semanticConflictKindLabel,
  unixTimeLabel,
  vitalityStateClass,
} from "./shared";

interface SkillsGovernanceSectionStyles {
  summaryCardClass: string;
  sectionCardClass: string;
}

export interface SkillsGovernanceSectionProps {
  styles: SkillsGovernanceSectionStyles;
  skillUsageLedger: SkillOperationalSnapshotRecord[];
  runtimeUsedSkillCount: number;
  neverReusedSkillCount: number;
  retiredSkillCount: number;
  reviewCandidateCount: number;
  skillNegativeEntropy: SkillNegativeEntropyDiagnosticRecord[];
  skillSemanticConflicts: SkillSemanticConflictDiagnosticRecord[];
  reviewCandidateEntries: SkillReviewCandidateView[];
  topUsageEntries: SkillOperationalSnapshotRecord[];
  negativeEntropyPreview: SkillNegativeEntropyDiagnosticRecord[];
  semanticConflictPreview: SkillSemanticConflictDiagnosticRecord[];
  skillGuardReports: SkillGuardReportRecord[];
  skillGuardTarget: string | null;
  blockedGuardCount: number;
  warnedGuardCount: number;
  passedGuardCount: number;
  totalGuardViolations: number;
  recentGuardReports: SkillGuardReportRecord[];
  selectedSkillName: string | null;
  selectedSourceId: string | null;
  skillGovernanceTimeline: SkillGovernanceTimelineEntryRecord[];
}

export function SkillsGovernanceSection({
  styles,
  skillUsageLedger,
  runtimeUsedSkillCount,
  neverReusedSkillCount,
  retiredSkillCount,
  reviewCandidateCount,
  skillNegativeEntropy,
  skillSemanticConflicts,
  reviewCandidateEntries,
  topUsageEntries,
  negativeEntropyPreview,
  semanticConflictPreview,
  skillGuardReports,
  skillGuardTarget,
  blockedGuardCount,
  warnedGuardCount,
  passedGuardCount,
  totalGuardViolations,
  recentGuardReports,
  selectedSkillName,
  selectedSourceId,
  skillGovernanceTimeline,
}: SkillsGovernanceSectionProps) {
  const { summaryCardClass, sectionCardClass } = styles;
  const { t } = useI18n();

  return (
    <div className="grid gap-5" data-testid="settings-skills-governance">
      <div className="grid gap-3 md:grid-cols-4">
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.governance.usageLedger")}
          </span>
          <div className="mt-2 text-2xl font-semibold text-foreground">
            {skillUsageLedger.length}
          </div>
          <div className="mt-1 text-sm text-muted-foreground">
            {t("settings.skills.governance.runtimeUsedLine", { used: runtimeUsedSkillCount, neverReused: neverReusedSkillCount })}
          </div>
          <div className="mt-1 text-xs text-muted-foreground">
            {t("settings.skills.governance.reviewRetiredLine", { review: reviewCandidateCount, retired: retiredSkillCount })}
          </div>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.governance.negativeEntropy")}
          </span>
          <div className="mt-2 text-2xl font-semibold text-foreground">
            {skillNegativeEntropy.length}
          </div>
          <div className="mt-1 text-sm text-muted-foreground">
            {t("settings.skills.governance.negativeEntropyNote")}
          </div>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.governance.semanticOverlap")}
          </span>
          <div className="mt-2 text-2xl font-semibold text-foreground">
            {skillSemanticConflicts.length}
          </div>
          <div className="mt-1 text-sm text-muted-foreground">
            {t("settings.skills.governance.semanticOverlapNote")}
          </div>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.governance.readModels")}
          </span>
          <div className="mt-2 text-sm text-muted-foreground leading-relaxed">
            /skill/hub/usage · /negative-entropy · /semantic-conflicts · /timeline
          </div>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <div className={sectionCardClass + " lg:col-span-3"}>
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
              {t("settings.skills.governance.reviewCandidates")}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("settings.skills.governance.reviewCandidatesNote")}
            </span>
          </div>
          <div className="mt-3 grid gap-2">
            {reviewCandidateEntries.length ? (
              reviewCandidateEntries.map((item) => (
                <div
                  key={item.entry.skill_name}
                  className="rounded-lg bg-muted/30 px-3 py-2 text-sm"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <strong className="block truncate">{item.entry.skill_name}</strong>
                      <div className="mt-1 flex flex-wrap gap-1.5 text-[10px]">
                        <span
                          className={cn(
                            "rounded-full border px-2 py-0.5 font-semibold uppercase tracking-wide",
                            vitalityStateClass(item.entry.vitality?.state),
                          )}
                        >
                          {formatVitalityStateLabel(item.entry.vitality?.state, t)}
                        </span>
                        <span
                          className={cn(
                            "rounded-full border px-2 py-0.5 font-semibold uppercase tracking-wide",
                            reviewReasonKindClass(item.reasonKind),
                          )}
                        >
                          {reviewReasonKindLabel(item.reasonKind, t)}
                        </span>
                        {item.relatedSkillName ? (
                          <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                            {t("settings.skills.governance.relatedPrefix", { name: item.relatedSkillName })}
                          </span>
                        ) : null}
                      </div>
                    </div>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {unixTimeLabel(item.entry.vitality?.updated_at)}
                    </span>
                  </div>
                  <div className="mt-2 text-xs text-muted-foreground">
                    {t("settings.skills.governance.markedBecause", { summary: item.summary })}
                  </div>
                  {item.evidenceBadges.length ? (
                    <div className="mt-2 flex flex-wrap gap-1.5 text-[10px]">
                      {item.evidenceBadges.map((badge) => (
                        <span
                          key={`${item.entry.skill_name}:${badge}`}
                          className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground"
                        >
                          {badge}
                        </span>
                      ))}
                    </div>
                  ) : null}
                  {item.evidenceLines.length ? (
                    <div className="mt-2 grid gap-1 text-xs text-muted-foreground">
                      {item.evidenceLines.map((line, index) => (
                        <div key={`${item.entry.skill_name}:${index}`}>{line}</div>
                      ))}
                    </div>
                  ) : null}
                </div>
              ))
            ) : (
              <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
                {t("settings.skills.governance.noReviewCandidates")}
              </div>
            )}
          </div>
        </div>

        <div className={sectionCardClass}>
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
              {t("settings.skills.governance.topRuntimeUse")}
            </span>
            <span className="text-xs text-muted-foreground">{t("settings.skills.governance.shownCount", { count: topUsageEntries.length })}</span>
          </div>
          <div className="mt-3 grid gap-2">
            {topUsageEntries.length ? (
              topUsageEntries.map((entry) => (
                <div key={entry.skill_name} className="rounded-lg bg-muted/30 px-3 py-2 text-sm">
                  <div className="flex items-start justify-between gap-3">
                    <strong>{entry.skill_name}</strong>
                    <div className="flex flex-wrap items-center justify-end gap-1.5">
                      <span
                        className={cn(
                          "rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                          vitalityStateClass(entry.vitality?.state),
                        )}
                      >
                        {formatVitalityStateLabel(entry.vitality?.state, t)}
                      </span>
                      <span className="text-muted-foreground">
                        {t("settings.skills.governance.usesCount", { count: entry.usage?.runtime_use_count ?? 0 })}
                      </span>
                    </div>
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.skills.governance.lastUsedLine", {
                      lastUsed: unixTimeLabel(entry.usage?.last_used_at),
                      lastWritten: unixTimeLabel(entry.writes?.last_write_at),
                    })}
                  </div>
                  {entry.vitality?.reason?.summary ? (
                    <div className="mt-1 text-xs text-muted-foreground">
                      {entry.vitality.reason.summary}
                    </div>
                  ) : null}
                </div>
              ))
            ) : (
              <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
                {t("settings.skills.governance.noRuntimeUsage")}
              </div>
            )}
          </div>
        </div>

        <div className={sectionCardClass}>
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
              {t("settings.skills.governance.negativeEntropy")}
            </span>
            <span className="text-xs text-muted-foreground">{t("settings.skills.governance.shownCount", { count: negativeEntropyPreview.length })}</span>
          </div>
          <div className="mt-3 grid gap-2">
            {negativeEntropyPreview.length ? (
              negativeEntropyPreview.map((item) => (
                <div key={item.skill_name} className="rounded-lg bg-muted/30 px-3 py-2 text-sm">
                  <div className="flex items-start justify-between gap-3">
                    <strong>{item.skill_name}</strong>
                    <span
                      className={cn(
                        "rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                        governanceSeverityClass(item.severity),
                      )}
                    >
                      {item.severity}
                    </span>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-1.5 text-[10px]">
                    {item.signals.map((signal) => (
                      <span
                        key={`${item.skill_name}:${signal}`}
                        className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground"
                      >
                        {negativeEntropySignalLabel(signal)}
                      </span>
                    ))}
                  </div>
                  <div className="mt-2 text-xs text-muted-foreground">
                    {t("settings.skills.evidenceUsage", {
                      use: item.runtime_use_count,
                      writes: item.write_count,
                      overlap: item.semantic_overlap_count,
                    })}
                  </div>
                  {item.reasons[0] ? (
                    <div className="mt-1 text-xs text-muted-foreground">{item.reasons[0]}</div>
                  ) : null}
                </div>
              ))
            ) : (
              <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
                {t("settings.skills.governance.noNegativeEntropy")}
              </div>
            )}
          </div>
        </div>

        <div className={sectionCardClass}>
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
              {t("settings.skills.governance.semanticOverlap")}
            </span>
            <span className="text-xs text-muted-foreground">{t("settings.skills.governance.shownCount", { count: semanticConflictPreview.length })}</span>
          </div>
          <div className="mt-3 grid gap-2">
            {semanticConflictPreview.length ? (
              semanticConflictPreview.map((item) => (
                <div
                  key={`${item.left_skill_name}:${item.right_skill_name}`}
                  className="rounded-lg bg-muted/30 px-3 py-2 text-sm"
                >
                  <div className="flex items-start justify-between gap-3">
                    <strong>
                      {item.left_skill_name}
                      {" <> "}
                      {item.right_skill_name}
                    </strong>
                    <span
                      className={cn(
                        "rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                        governanceSeverityClass(item.severity),
                      )}
                    >
                      {item.score}
                    </span>
                  </div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {t("settings.skills.governance.runtimeUsesVs", {
                      kind: semanticConflictKindLabel(item.kind),
                      left: item.left_runtime_use_count,
                      right: item.right_runtime_use_count,
                    })}
                  </div>
                  {item.preferred_skill_name ? (
                    <div className="mt-1 text-xs text-muted-foreground">
                      {t("settings.skills.evidenceLedgerPrefers", { name: item.preferred_skill_name })}
                    </div>
                  ) : null}
                  {item.reasons[0] ? (
                    <div className="mt-1 text-xs text-muted-foreground">{item.reasons[0]}</div>
                  ) : null}
                </div>
              ))
            ) : (
              <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
                {t("settings.skills.governance.noSemanticOverlap")}
              </div>
            )}
          </div>
        </div>
      </div>

      <div>
        <div className="flex items-center justify-between gap-3 mb-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.governance.guardSummary")}
          </span>
          <span className="text-xs text-muted-foreground">
            {selectedSkillName ? t("settings.skills.governance.guardScopeSkill", { name: selectedSkillName }) : t("settings.skills.governance.guardScopeAll")}
          </span>
        </div>

        <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
          <span><strong className="text-foreground">{skillGuardReports.length}</strong> {t("settings.skills.governance.stats.reports")}</span>
          <span><strong className="text-foreground">{blockedGuardCount}</strong> {t("settings.skills.governance.stats.blocked")}</span>
          <span><strong className="text-foreground">{warnedGuardCount}</strong> {t("settings.skills.governance.stats.warn")}</span>
          <span><strong className="text-foreground">{totalGuardViolations}</strong> {t("settings.skills.governance.stats.violations")}</span>
        </div>

        {skillGuardTarget ? (
          <div className="mt-3 text-xs text-muted-foreground">
            {t("settings.skills.governance.latestGuardRun", { target: skillGuardTarget, count: skillGuardReports.length })}
          </div>
        ) : null}
      </div>

      <div className="grid gap-2">
        <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
          {t("settings.skills.governance.recentGuardReports")}
        </div>
        {recentGuardReports.length ? (
          recentGuardReports.map((report) => (
            <div key={`${report.skill_name}:${report.scanned_at}`} className="rounded-lg bg-muted/40 p-3 text-sm">
              <div className="flex items-start justify-between gap-3">
                <strong>{report.skill_name}</strong>
                <span
                  className={cn(
                    "rounded-full border px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wide",
                    report.status === "blocked"
                      ? "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)"
                      : report.status === "warn"
                        ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                        : "border-border/40 bg-muted text-muted-foreground",
                  )}
                >
                  {latestGuardStatusLabel(report, t)}
                </span>
              </div>
              <div className="mt-2 text-muted-foreground">
                {report.violations.length
                  ? t("settings.skills.governance.guardViolations", { count: report.violations.length })
                  : passedGuardCount
                    ? t("settings.skills.governance.guardNoViolations")
                    : t("settings.skills.governance.guardPassed")}
                {" · "}
                {t("settings.skills.governance.scannedAt", { time: unixTimeLabel(report.scanned_at) })}
              </div>
            </div>
          ))
        ) : (
          <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
            {t("settings.skills.governance.noGuardReports")}
          </div>
        )}
      </div>

      <SkillGovernanceTimeline
        entries={skillGovernanceTimeline}
        selectedSkillName={selectedSkillName}
        selectedSourceId={selectedSourceId}
      />
    </div>
  );
}
