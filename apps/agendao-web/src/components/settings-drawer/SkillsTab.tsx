import { useMemo, useState } from "react";
import type {
  ManagedSkillRecord,
  SkillArtifactCacheEntryRecord,
  SkillCatalogEntry,
  SkillDetailResponseRecord,
  SkillDistributionRecord,
  SkillGovernanceTimelineEntryRecord,
  SkillGuardReportRecord,
  SkillHubPolicyRecord,
  SkillNegativeEntropyDiagnosticRecord,
  SkillManagedLifecycleRecord,
  SkillOperationalSnapshotRecord,
  SkillRemoteInstallPlanRecord,
  SkillSemanticConflictDiagnosticRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
  SkillSyncPlanRecord,
} from "@/lib/skill";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { SkillMethodologyDraft } from "../settings/SkillMethodologyEditor";
import type { SkillEditorMode } from "./types";
import type { SkillsTabStyles, SkillSubtabId } from "./skills/shared";
import {
  negativeEntropySignalLabel,
  semanticConflictKindLabel,
  skillNameKey,
  usageWriteCount,
} from "./skills/shared";
import type { SkillHubSearchResult, SkillReviewCandidateView } from "./skills/shared";
import { SkillsOverviewSection } from "./skills/SkillsOverviewSection";
import { SkillsHubSection } from "./skills/SkillsHubSection";
import { SkillsCatalogSection } from "./skills/SkillsCatalogSection";
import { SkillsGovernanceSection } from "./skills/SkillsGovernanceSection";

export interface SkillsTabProps {
  workspaceRootPath: string;
  selectedSessionId: string | null;
  skillWorkspaceRoot: string;
  skillsMutationsEnabled: boolean;
  styles: SkillsTabStyles;
  busyKey: string | null;
  skillCatalog: SkillCatalogEntry[];
  managedSkills: ManagedSkillRecord[];
  skillUsageLedger: SkillOperationalSnapshotRecord[];
  skillNegativeEntropy: SkillNegativeEntropyDiagnosticRecord[];
  skillSemanticConflicts: SkillSemanticConflictDiagnosticRecord[];
  skillSourceIndices: SkillSourceIndexSnapshotRecord[];
  skillDistributions: SkillDistributionRecord[];
  skillArtifactCache: SkillArtifactCacheEntryRecord[];
  skillHubPolicy: SkillHubPolicyRecord | null;
  skillLifecycle: SkillManagedLifecycleRecord[];
  skillGovernanceTimeline: SkillGovernanceTimelineEntryRecord[];
  skillSyncSourceId: string;
  onSkillSyncSourceIdChange: (value: string) => void;
  skillSyncSourceKind: SkillSourceRefRecord["source_kind"];
  onSkillSyncSourceKindChange: (value: SkillSourceRefRecord["source_kind"]) => void;
  skillSyncLocator: string;
  onSkillSyncLocatorChange: (value: string) => void;
  skillSyncRevision: string;
  onSkillSyncRevisionChange: (value: string) => void;
  skillSyncPlan: SkillSyncPlanRecord | null;
  onPlanSkillSync: () => void;
  onApplySkillSync: () => void;
  onRefreshSkillSourceIndex: () => void;
  onRunSelectedSourceGuard: () => void;
  remoteInstallSkillName: string;
  onRemoteInstallSkillNameChange: (value: string) => void;
  remoteInstallPlan: SkillRemoteInstallPlanRecord | null;
  onPlanRemoteInstall: () => void;
  onPlanRemoteUpdate: () => void;
  onApplyRemoteInstall: () => void;
  onApplyRemoteUpdate: () => void;
  onDetachManagedSkill: () => void;
  onRemoveManagedSkill: () => void;
  skillGuardReports: SkillGuardReportRecord[];
  skillGuardTarget: string | null;
  selectedSkillName: string | null;
  onSelectedSkillNameChange: (value: string) => void;
  selectedSkillEntry: SkillCatalogEntry | null;
  skillDetail: SkillDetailResponseRecord | null;
  skillDetailLoading: boolean;
  skillEditorContent: string;
  onSkillEditorContentChange: (value: string) => void;
  editSkillEditorMode: SkillEditorMode;
  onEditSkillEditorModeChange: (value: SkillEditorMode) => void;
  editSkillDescription: string;
  onEditSkillDescriptionChange: (value: string) => void;
  editSkillMethodologyDraft: SkillMethodologyDraft;
  onEditSkillMethodologyDraftChange: (value: SkillMethodologyDraft) => void;
  editSkillMethodologyMatched: boolean;
  editSkillMethodologyPreview: string;
  editSkillMethodologyPreviewError: string | null;
  newSkillName: string;
  onNewSkillNameChange: (value: string) => void;
  newSkillDescription: string;
  onNewSkillDescriptionChange: (value: string) => void;
  newSkillCategory: string;
  onNewSkillCategoryChange: (value: string) => void;
  newSkillBody: string;
  onNewSkillBodyChange: (value: string) => void;
  newSkillEditorMode: SkillEditorMode;
  onNewSkillEditorModeChange: (value: SkillEditorMode) => void;
  newSkillMethodologyDraft: SkillMethodologyDraft;
  onNewSkillMethodologyDraftChange: (value: SkillMethodologyDraft) => void;
  newSkillMethodologyPreview: string;
  newSkillMethodologyPreviewError: string | null;
  onCreateSkill: () => void;
  onRunSelectedSkillGuard: () => void;
  onSaveSelectedSkill: () => void;
  onDeleteSelectedSkill: () => void;
  managedRecordBySkill: Map<string, ManagedSkillRecord>;
  latestGuardBySkill: Map<string, SkillGuardReportRecord>;
  selectedHubSourceSnapshot: SkillSourceIndexSnapshotRecord | null;
  selectedRemoteSourceEntry: SkillSourceIndexSnapshotRecord["entries"][number] | null;
  selectedRemoteDistribution: SkillDistributionRecord | null;
  selectedRemoteArtifactCache: SkillArtifactCacheEntryRecord | null;
  selectedRemoteLifecycle: SkillManagedLifecycleRecord | null;
}

export function SkillsTab({
  workspaceRootPath,
  selectedSessionId,
  skillWorkspaceRoot,
  skillsMutationsEnabled,
  styles,
  busyKey,
  skillCatalog,
  managedSkills,
  skillUsageLedger,
  skillNegativeEntropy,
  skillSemanticConflicts,
  skillSourceIndices,
  skillDistributions,
  skillHubPolicy,
  skillLifecycle,
  skillGovernanceTimeline,
  skillSyncSourceId,
  onSkillSyncSourceIdChange,
  skillSyncSourceKind,
  onSkillSyncSourceKindChange,
  skillSyncLocator,
  onSkillSyncLocatorChange,
  skillSyncRevision,
  onSkillSyncRevisionChange,
  skillSyncPlan,
  onPlanSkillSync,
  onApplySkillSync,
  onRefreshSkillSourceIndex,
  onRunSelectedSourceGuard,
  remoteInstallSkillName,
  onRemoteInstallSkillNameChange,
  remoteInstallPlan,
  onPlanRemoteInstall,
  onPlanRemoteUpdate,
  onApplyRemoteInstall,
  onApplyRemoteUpdate,
  onDetachManagedSkill,
  onRemoveManagedSkill,
  skillGuardReports,
  skillGuardTarget,
  selectedSkillName,
  onSelectedSkillNameChange,
  selectedSkillEntry,
  skillDetailLoading,
  skillEditorContent,
  onSkillEditorContentChange,
  editSkillEditorMode,
  onEditSkillEditorModeChange,
  editSkillDescription,
  onEditSkillDescriptionChange,
  editSkillMethodologyDraft,
  onEditSkillMethodologyDraftChange,
  editSkillMethodologyMatched,
  editSkillMethodologyPreview,
  editSkillMethodologyPreviewError,
  newSkillName,
  onNewSkillNameChange,
  newSkillDescription,
  onNewSkillDescriptionChange,
  newSkillCategory,
  onNewSkillCategoryChange,
  newSkillBody,
  onNewSkillBodyChange,
  newSkillEditorMode,
  onNewSkillEditorModeChange,
  newSkillMethodologyDraft,
  onNewSkillMethodologyDraftChange,
  newSkillMethodologyPreview,
  newSkillMethodologyPreviewError,
  onCreateSkill,
  onRunSelectedSkillGuard,
  onSaveSelectedSkill,
  onDeleteSelectedSkill,
  managedRecordBySkill,
  latestGuardBySkill,
  selectedHubSourceSnapshot,
  selectedRemoteSourceEntry,
  selectedRemoteDistribution,
  selectedRemoteArtifactCache,
  selectedRemoteLifecycle,
}: SkillsTabProps) {
  const {
    primaryButtonClass,
    secondaryButtonClass,
    summaryCardClass,
    sectionCardClass,
    mutedCardClass,
    editorTextareaClass,
  } = styles;
  const { t } = useI18n();

  const [activeSubtab, setActiveSubtab] = useState<SkillSubtabId>("overview");
  const [hubSearchDraft, setHubSearchDraft] = useState("");
  const selectedSourceId = skillSyncSourceId.trim();
  const selectedSourceLocator = skillSyncLocator.trim();
  const selectedRemoteSkillName = remoteInstallSkillName.trim();
  const selectedManagedRecord = selectedSkillEntry
    ? managedRecordBySkill.get(selectedSkillEntry.name.trim().toLowerCase()) ?? null
    : null;
  const selectedLatestGuard = selectedSkillEntry
    ? latestGuardBySkill.get(selectedSkillEntry.name.trim().toLowerCase()) ?? null
    : null;
  const blockedGuardCount = skillGuardReports.filter((report) => report.status === "blocked").length;
  const warnedGuardCount = skillGuardReports.filter((report) => report.status === "warn").length;
  const passedGuardCount = skillGuardReports.filter((report) => report.status === "passed").length;
  const totalGuardViolations = skillGuardReports.reduce(
    (count, report) => count + report.violations.length,
    0,
  );
  const runtimeUsedSkillCount = useMemo(
    () =>
      skillUsageLedger.filter((entry) => (entry.usage?.runtime_use_count ?? 0) > 0).length,
    [skillUsageLedger],
  );
  const neverReusedSkillCount = useMemo(
    () =>
      skillUsageLedger.filter(
        (entry) => usageWriteCount(entry) > 0 && (entry.usage?.runtime_use_count ?? 0) === 0,
      ).length,
    [skillUsageLedger],
  );
  const retiredSkillCount = useMemo(
    () => skillUsageLedger.filter((entry) => entry.vitality?.state === "retired").length,
    [skillUsageLedger],
  );
  const reviewCandidateCount = useMemo(
    () => skillUsageLedger.filter((entry) => entry.vitality?.state === "review_candidate").length,
    [skillUsageLedger],
  );
  const reviewCandidateEntries = useMemo<SkillReviewCandidateView[]>(
    () =>
      skillUsageLedger
        .filter((entry) => entry.vitality?.state === "review_candidate" && entry.vitality?.reason)
        .map((entry) => {
          const reason = entry.vitality!.reason;
          const relatedSkillName = reason.related_skill_name ?? null;
          const skillKey = skillNameKey(entry.skill_name);
          const relatedSkillKey = relatedSkillName ? skillNameKey(relatedSkillName) : null;
          const evidenceBadges: string[] = [];
          const evidenceLines: string[] = [];

          if (reason.kind === "negative_entropy") {
            const diagnostic =
              skillNegativeEntropy.find((item) => skillNameKey(item.skill_name) === skillKey) ?? null;
            if (diagnostic) {
              evidenceBadges.push(...diagnostic.signals.map(negativeEntropySignalLabel));
              evidenceLines.push(
                t("settings.skills.evidenceUsage", {
                  use: diagnostic.runtime_use_count,
                  writes: diagnostic.write_count,
                  overlap: diagnostic.semantic_overlap_count,
                }),
              );
              diagnostic.reasons
                .filter((line) => line !== reason.summary)
                .slice(0, 2)
                .forEach((line) => evidenceLines.push(line));
            }
          } else if (reason.kind === "semantic_conflict") {
            const conflict =
              skillSemanticConflicts.find((item) => {
                const matchesSkill =
                  skillNameKey(item.left_skill_name) === skillKey ||
                  skillNameKey(item.right_skill_name) === skillKey;
                if (!matchesSkill) return false;
                if (!relatedSkillKey) return true;
                return (
                  skillNameKey(item.left_skill_name) === relatedSkillKey ||
                  skillNameKey(item.right_skill_name) === relatedSkillKey ||
                  (item.preferred_skill_name
                    ? skillNameKey(item.preferred_skill_name) === relatedSkillKey
                    : false)
                );
              }) ?? null;
            if (conflict) {
              evidenceBadges.push(
                semanticConflictKindLabel(conflict.kind),
                t("settings.skills.evidenceScore", { score: conflict.score }),
              );
              evidenceLines.push(
                t("settings.skills.evidenceConflictLedger", {
                  left: conflict.left_skill_name,
                  leftCount: conflict.left_runtime_use_count,
                  right: conflict.right_skill_name,
                  rightCount: conflict.right_runtime_use_count,
                }),
              );
              if (conflict.preferred_skill_name) {
                evidenceLines.push(
                  t("settings.skills.evidenceLedgerPrefers", { name: conflict.preferred_skill_name }),
                );
              }
              conflict.reasons
                .filter((line) => line !== reason.summary)
                .slice(0, 2)
                .forEach((line) => evidenceLines.push(line));
            }
          }

          return {
            entry,
            reasonKind: reason.kind,
            relatedSkillName,
            summary: reason.summary,
            evidenceBadges,
            evidenceLines,
          };
        })
        .sort(
          (left, right) =>
            (right.entry.vitality?.updated_at ?? 0) - (left.entry.vitality?.updated_at ?? 0) ||
            left.entry.skill_name.localeCompare(right.entry.skill_name),
        ),
    [skillNegativeEntropy, skillSemanticConflicts, skillUsageLedger, t],
  );
  const topUsageEntries = useMemo(
    () =>
      [...skillUsageLedger]
        .filter((entry) => (entry.usage?.runtime_use_count ?? 0) > 0)
        .sort(
          (left, right) =>
            (right.usage?.runtime_use_count ?? 0) - (left.usage?.runtime_use_count ?? 0) ||
            left.skill_name.localeCompare(right.skill_name),
        )
        .slice(0, 6),
    [skillUsageLedger],
  );
  const negativeEntropyPreview = useMemo(
    () =>
      [...skillNegativeEntropy]
        .sort(
          (left, right) =>
            (right.semantic_overlap_count ?? 0) - (left.semantic_overlap_count ?? 0) ||
            right.write_count - left.write_count ||
            left.skill_name.localeCompare(right.skill_name),
        )
        .slice(0, 6),
    [skillNegativeEntropy],
  );
  const semanticConflictPreview = useMemo(
    () =>
      [...skillSemanticConflicts]
        .sort(
          (left, right) =>
            right.score - left.score ||
            left.left_skill_name.localeCompare(right.left_skill_name) ||
            left.right_skill_name.localeCompare(right.right_skill_name),
        )
        .slice(0, 6),
    [skillSemanticConflicts],
  );
  const recentGuardReports = [...skillGuardReports]
    .sort((left, right) => right.scanned_at - left.scanned_at)
    .slice(0, 4);
  const hubSearchResults = useMemo<SkillHubSearchResult[]>(() => {
    const query = hubSearchDraft.trim().toLowerCase();
    const rows = skillSourceIndices.flatMap((snapshot) =>
      snapshot.entries.map((entry) => {
        const searchText = [
          entry.skill_name,
          entry.description ?? "",
          entry.category ?? "",
          entry.revision ?? "",
          snapshot.source.source_id,
          snapshot.source.source_kind,
          snapshot.source.locator,
          snapshot.source.revision ?? "",
        ]
          .join("\n")
          .toLowerCase();
        const installedRecord = managedRecordBySkill.get(entry.skill_name.trim().toLowerCase()) ?? null;
        const distribution = skillDistributions.find(
          (record) =>
            record.source.source_id === snapshot.source.source_id &&
            record.skill_name.trim().toLowerCase() === entry.skill_name.trim().toLowerCase(),
        ) ?? null;
        const lifecycle = distribution
          ? skillLifecycle.find((record) => record.distribution_id === distribution.distribution_id) ?? null
          : skillLifecycle.find(
              (record) =>
                record.source_id === snapshot.source.source_id &&
                record.skill_name.trim().toLowerCase() === entry.skill_name.trim().toLowerCase(),
            ) ?? null;
        const score = entry.skill_name.trim().toLowerCase() === query
          ? 0
          : entry.skill_name.trim().toLowerCase().startsWith(query)
            ? 1
            : searchText.includes(query)
              ? 2
              : 3;
        return { distribution, entry, installedRecord, lifecycle, score, searchText, source: snapshot.source };
      }),
    );
    return rows
      .filter((row) => !query || row.searchText.includes(query))
      .sort((left, right) => {
        if (left.score !== right.score) return left.score - right.score;
        if (!!left.installedRecord !== !!right.installedRecord) return left.installedRecord ? -1 : 1;
        return left.entry.skill_name.localeCompare(right.entry.skill_name);
      })
      .slice(0, 24);
  }, [hubSearchDraft, managedRecordBySkill, skillDistributions, skillLifecycle, skillSourceIndices]);

  const selectHubRemoteSkill = (
    source: SkillSourceRefRecord,
    skillName: string,
  ) => {
    onSkillSyncSourceIdChange(source.source_id);
    onSkillSyncSourceKindChange(source.source_kind);
    onSkillSyncLocatorChange(source.locator);
    onSkillSyncRevisionChange(source.revision ?? "");
    onRemoteInstallSkillNameChange(skillName);
  };

  return (
    <div className="relative grid gap-4" data-testid="settings-skills-root">
      {/* ── Header + Sub-tabs ── */}
      <div className="flex items-center justify-between gap-3">
        <h3 className="m-0 text-base font-semibold">{t("settings.skills.title")}</h3>
        <div className="flex gap-1 rounded-lg bg-muted/40 p-1">
          {(["overview", "hub", "catalog", "governance"] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              data-testid={`settings-skills-subtab-${tab}`}
              data-active={activeSubtab === tab ? "true" : "false"}
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                activeSubtab === tab
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
              onClick={() => setActiveSubtab(tab)}
            >
              {t(`settings.skills.subtab.${tab}`)}
            </button>
          ))}
        </div>
      </div>

      {/* Inline stats row */}
      <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
        <span><strong className="text-foreground">{skillCatalog.length}</strong> {t("settings.skills.stats.skills")}</span>
        <span><strong className="text-foreground">{managedSkills.length}</strong> {t("settings.skills.stats.managed")}</span>
        <span><strong className="text-foreground">{skillSourceIndices.length}</strong> {t("settings.skills.stats.sources")}</span>
        <span><strong className="text-foreground">{skillGovernanceTimeline.length}</strong> {t("settings.skills.stats.events")}</span>
      </div>

      {!skillsMutationsEnabled ? (
        <div className="rounded-lg border border-(--ds-warn)/40 bg-(--ds-warn)/12 px-4 py-2.5 text-sm leading-relaxed text-(--ds-warn)">
          {t("settings.skills.mutationsDisabled")}
        </div>
      ) : null}

      {activeSubtab === "overview" ? (
        <SkillsOverviewSection
          workspaceRootPath={workspaceRootPath}
          selectedSessionId={selectedSessionId}
          skillWorkspaceRoot={skillWorkspaceRoot}
          skillsMutationsEnabled={skillsMutationsEnabled}
          busyKey={busyKey}
          styles={{ primaryButtonClass, editorTextareaClass }}
          newSkillName={newSkillName}
          onNewSkillNameChange={onNewSkillNameChange}
          newSkillDescription={newSkillDescription}
          onNewSkillDescriptionChange={onNewSkillDescriptionChange}
          newSkillCategory={newSkillCategory}
          onNewSkillCategoryChange={onNewSkillCategoryChange}
          newSkillBody={newSkillBody}
          onNewSkillBodyChange={onNewSkillBodyChange}
          newSkillEditorMode={newSkillEditorMode}
          onNewSkillEditorModeChange={onNewSkillEditorModeChange}
          newSkillMethodologyDraft={newSkillMethodologyDraft}
          onNewSkillMethodologyDraftChange={onNewSkillMethodologyDraftChange}
          newSkillMethodologyPreview={newSkillMethodologyPreview}
          newSkillMethodologyPreviewError={newSkillMethodologyPreviewError}
          onCreateSkill={onCreateSkill}
        />
      ) : null}

      {activeSubtab === "hub" ? (
        <SkillsHubSection
          styles={{ primaryButtonClass, secondaryButtonClass }}
          busyKey={busyKey}
          skillsMutationsEnabled={skillsMutationsEnabled}
          hubSearchDraft={hubSearchDraft}
          onHubSearchDraftChange={setHubSearchDraft}
          hubSearchResults={hubSearchResults}
          managedSkills={managedSkills}
          skillSourceIndices={skillSourceIndices}
          skillSyncSourceId={skillSyncSourceId}
          onSkillSyncSourceIdChange={onSkillSyncSourceIdChange}
          skillSyncSourceKind={skillSyncSourceKind}
          onSkillSyncSourceKindChange={onSkillSyncSourceKindChange}
          skillSyncLocator={skillSyncLocator}
          onSkillSyncLocatorChange={onSkillSyncLocatorChange}
          skillSyncRevision={skillSyncRevision}
          onSkillSyncRevisionChange={onSkillSyncRevisionChange}
          skillSyncPlan={skillSyncPlan}
          onPlanSkillSync={onPlanSkillSync}
          onApplySkillSync={onApplySkillSync}
          onRefreshSkillSourceIndex={onRefreshSkillSourceIndex}
          onRunSelectedSourceGuard={onRunSelectedSourceGuard}
          remoteInstallSkillName={remoteInstallSkillName}
          onRemoteInstallSkillNameChange={onRemoteInstallSkillNameChange}
          remoteInstallPlan={remoteInstallPlan}
          onPlanRemoteInstall={onPlanRemoteInstall}
          onPlanRemoteUpdate={onPlanRemoteUpdate}
          onApplyRemoteInstall={onApplyRemoteInstall}
          onApplyRemoteUpdate={onApplyRemoteUpdate}
          onDetachManagedSkill={onDetachManagedSkill}
          onRemoveManagedSkill={onRemoveManagedSkill}
          skillHubPolicy={skillHubPolicy}
          selectedHubSourceSnapshot={selectedHubSourceSnapshot}
          selectedRemoteSourceEntry={selectedRemoteSourceEntry}
          selectedRemoteDistribution={selectedRemoteDistribution}
          selectedRemoteArtifactCache={selectedRemoteArtifactCache}
          selectedRemoteLifecycle={selectedRemoteLifecycle}
          selectedSourceId={selectedSourceId}
          selectedSourceLocator={selectedSourceLocator}
          selectedRemoteSkillName={selectedRemoteSkillName}
          onSelectHubRemoteSkill={selectHubRemoteSkill}
        />
      ) : null}

      {activeSubtab === "catalog" ? (
        <SkillsCatalogSection
          styles={{ primaryButtonClass, secondaryButtonClass, mutedCardClass }}
          busyKey={busyKey}
          skillsMutationsEnabled={skillsMutationsEnabled}
          skillWorkspaceRoot={skillWorkspaceRoot}
          skillCatalog={skillCatalog}
          selectedSkillEntry={selectedSkillEntry}
          onSelectedSkillNameChange={onSelectedSkillNameChange}
          managedRecordBySkill={managedRecordBySkill}
          latestGuardBySkill={latestGuardBySkill}
          selectedManagedRecord={selectedManagedRecord}
          selectedLatestGuard={selectedLatestGuard}
          skillDetailLoading={skillDetailLoading}
          skillEditorContent={skillEditorContent}
          onSkillEditorContentChange={onSkillEditorContentChange}
          editSkillEditorMode={editSkillEditorMode}
          onEditSkillEditorModeChange={onEditSkillEditorModeChange}
          editSkillDescription={editSkillDescription}
          onEditSkillDescriptionChange={onEditSkillDescriptionChange}
          editSkillMethodologyDraft={editSkillMethodologyDraft}
          onEditSkillMethodologyDraftChange={onEditSkillMethodologyDraftChange}
          editSkillMethodologyMatched={editSkillMethodologyMatched}
          editSkillMethodologyPreview={editSkillMethodologyPreview}
          editSkillMethodologyPreviewError={editSkillMethodologyPreviewError}
          onRunSelectedSkillGuard={onRunSelectedSkillGuard}
          onSaveSelectedSkill={onSaveSelectedSkill}
          onDeleteSelectedSkill={onDeleteSelectedSkill}
        />
      ) : null}

      {activeSubtab === "governance" ? (
        <SkillsGovernanceSection
          styles={{ summaryCardClass, sectionCardClass }}
          skillUsageLedger={skillUsageLedger}
          runtimeUsedSkillCount={runtimeUsedSkillCount}
          neverReusedSkillCount={neverReusedSkillCount}
          retiredSkillCount={retiredSkillCount}
          reviewCandidateCount={reviewCandidateCount}
          skillNegativeEntropy={skillNegativeEntropy}
          skillSemanticConflicts={skillSemanticConflicts}
          reviewCandidateEntries={reviewCandidateEntries}
          topUsageEntries={topUsageEntries}
          negativeEntropyPreview={negativeEntropyPreview}
          semanticConflictPreview={semanticConflictPreview}
          skillGuardReports={skillGuardReports}
          skillGuardTarget={skillGuardTarget}
          blockedGuardCount={blockedGuardCount}
          warnedGuardCount={warnedGuardCount}
          passedGuardCount={passedGuardCount}
          totalGuardViolations={totalGuardViolations}
          recentGuardReports={recentGuardReports}
          selectedSkillName={selectedSkillName}
          selectedSourceId={selectedSourceId || null}
          skillGovernanceTimeline={skillGovernanceTimeline}
        />
      ) : null}
    </div>
  );
}
