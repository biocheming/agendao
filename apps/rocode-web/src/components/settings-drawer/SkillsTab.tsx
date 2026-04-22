import { useState } from "react";
import type {
  ManagedSkillRecord,
  SkillArtifactCacheEntryRecord,
  SkillCatalogEntry,
  SkillDetailResponseRecord,
  SkillDistributionRecord,
  SkillGovernanceTimelineEntryRecord,
  SkillGuardReportRecord,
  SkillHubPolicyRecord,
  SkillManagedLifecycleRecord,
  SkillRemoteInstallPlanRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
  SkillSyncPlanRecord,
} from "@/lib/skill";
import { cn } from "@/lib/utils";
import { SkillGovernanceTimeline } from "../SkillGovernanceTimeline";
import {
  SkillMethodologyDraft,
  SkillMethodologyEditor,
} from "../SkillMethodologyEditor";

type SkillEditorMode = "methodology" | "raw";
type SkillSubtabId = "overview" | "hub" | "catalog" | "governance";

interface SkillsTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  summaryCardClass: string;
  sectionCardClass: string;
  mutedCardClass: string;
  editorTextareaClass: string;
}

interface SkillsTabProps {
  workspaceRootPath: string;
  selectedSessionId: string | null;
  skillWorkspaceRoot: string;
  skillsMutationsEnabled: boolean;
  styles: SkillsTabStyles;
  busyKey: string | null;
  skillCatalog: SkillCatalogEntry[];
  managedSkills: ManagedSkillRecord[];
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

function managedSkillStateLabel(record: ManagedSkillRecord): string {
  if (record.deleted_locally) return "deleted locally";
  if (record.locally_modified) return "locally modified";
  return "managed clean";
}

function latestGuardStatusLabel(report: SkillGuardReportRecord): string {
  switch (report.status) {
    case "blocked":
      return "guard blocked";
    case "warn":
      return "guard warn";
    default:
      return "guard passed";
  }
}

function lifecycleStatusClass(state: string): string {
  const normalized = state.trim().toLowerCase();
  if (normalized.includes("failed") || normalized === "diverged") {
    return "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950/60 dark:text-red-300";
  }
  if (
    normalized === "updateavailable" ||
    normalized === "update_available" ||
    normalized === "plannedinstall" ||
    normalized === "planned_install" ||
    normalized === "removepending" ||
    normalized === "remove_pending"
  ) {
    return "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200";
  }
  if (normalized === "installed" || normalized === "fetched" || normalized === "resolved") {
    return "border-green-300 bg-green-50 text-green-800 dark:border-green-700 dark:bg-green-950 dark:text-green-300";
  }
  return "border-border/40 bg-muted text-muted-foreground";
}

function formatHubDurationSeconds(value?: number | null): string {
  if (!value) return "--";
  if (value % 86400 === 0) return `${value / 86400}d`;
  if (value % 3600 === 0) return `${value / 3600}h`;
  if (value % 60 === 0) return `${value / 60}m`;
  return `${value}s`;
}

function formatHubDurationMs(value?: number | null): string {
  if (!value) return "--";
  if (value % 1000 === 0) return `${value / 1000}s`;
  return `${value}ms`;
}

function formatHubBytes(value?: number | null): string {
  if (!value) return "--";
  if (value >= 1024 * 1024 && value % (1024 * 1024) === 0) {
    return `${value / (1024 * 1024)} MiB`;
  }
  if (value >= 1024 && value % 1024 === 0) {
    return `${value / 1024} KiB`;
  }
  return `${value} bytes`;
}

function unixTimeLabel(value?: number | null): string {
  if (!value) return "--";
  try {
    const timestamp = value > 1_000_000_000_000 ? value : value * 1000;
    return new Date(timestamp).toLocaleString();
  } catch {
    return String(value);
  }
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
  skillSourceIndices,
  skillDistributions,
  skillArtifactCache,
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
  skillDetail,
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

  const [activeSubtab, setActiveSubtab] = useState<SkillSubtabId>("overview");
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
  const recentGuardReports = [...skillGuardReports]
    .sort((left, right) => right.scanned_at - left.scanned_at)
    .slice(0, 4);

  return (
    <div className="relative grid gap-4">
      {/* ── Header + Sub-tabs ── */}
      <div className="flex items-center justify-between gap-3">
        <h3 className="m-0 text-base font-semibold">Skills</h3>
        <div className="flex gap-1 rounded-lg bg-muted/40 p-1">
          {(["overview", "hub", "catalog", "governance"] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                activeSubtab === tab
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
              onClick={() => setActiveSubtab(tab)}
            >
              {tab === "hub" ? "Hub & Sync" : tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {/* Inline stats row */}
      <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
        <span><strong className="text-foreground">{skillCatalog.length}</strong> skills</span>
        <span><strong className="text-foreground">{managedSkills.length}</strong> managed</span>
        <span><strong className="text-foreground">{skillSourceIndices.length}</strong> sources</span>
        <span><strong className="text-foreground">{skillGovernanceTimeline.length}</strong> events</span>
      </div>

      {!skillsMutationsEnabled ? (
        <div className="rounded-lg border border-amber-300 bg-amber-50/80 px-4 py-2.5 text-sm leading-relaxed text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200">
          Select or create a session before managing skills so permission prompts can be
          routed to the active session.
        </div>
      ) : null}

      {activeSubtab === "overview" ? (
        <div className="grid gap-5">
          {/* Workspace info */}
          <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
            <div className="grid gap-2">
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Workspace Root</span>
                <span className="ml-2 text-foreground break-all">{workspaceRootPath || "--"}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Skill Root</span>
                <span className="ml-2 text-foreground break-all">{skillWorkspaceRoot}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Scope</span>
                <span className="ml-2 text-foreground">{selectedSessionId || "workspace authority"}</span>
              </div>
              <div className="mt-1 text-xs">
                Writes go through <code>/skill/manage</code> and land in the workspace authority at <code>{skillWorkspaceRoot}</code>.
                {selectedSessionId
                  ? " Catalog reads use session-aware scope through /skill/catalog?session_id=..."
                  : " Without an active session, the workspace authority is shown directly."}
              </div>
            </div>
          </div>

          {/* Create Skill */}
          <div>
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Create Skill</span>

            <div className="mt-3 flex flex-wrap gap-2">
              {(["methodology", "raw"] as SkillEditorMode[]).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  className={cn(
                    "rounded-full border px-3 py-1.5 text-xs font-semibold transition-colors",
                    newSkillEditorMode === mode
                      ? "border-border bg-accent text-foreground"
                      : "border-border/50 bg-background/60 text-muted-foreground hover:bg-accent/60",
                  )}
                  onClick={() => onNewSkillEditorModeChange(mode)}
                >
                  {mode === "methodology" ? "Methodology Form" : "Raw Markdown"}
                </button>
              ))}
            </div>

            <div className="mt-3 grid gap-3">
              <input
                type="text"
                placeholder="skill name"
                value={newSkillName}
                onChange={(event) => onNewSkillNameChange(event.target.value)}
              />
              <input
                type="text"
                placeholder="description"
                value={newSkillDescription}
                onChange={(event) => onNewSkillDescriptionChange(event.target.value)}
              />
              <input
                type="text"
                placeholder="category (optional)"
                value={newSkillCategory}
                onChange={(event) => onNewSkillCategoryChange(event.target.value)}
              />
              {newSkillEditorMode === "methodology" ? (
                <SkillMethodologyEditor
                  draft={newSkillMethodologyDraft}
                  onChange={onNewSkillMethodologyDraftChange}
                  previewBody={newSkillMethodologyPreview}
                  previewError={newSkillMethodologyPreviewError}
                />
              ) : (
                <textarea
                  className={editorTextareaClass}
                  placeholder="Skill body"
                  value={newSkillBody}
                  onChange={(event) => onNewSkillBodyChange(event.target.value)}
                  spellCheck={false}
                />
              )}
            </div>

            <div className="mt-3 flex items-center gap-2">
              <button
                className={primaryButtonClass}
                type="button"
                disabled={
                  !skillsMutationsEnabled ||
                  !newSkillName.trim() ||
                  !newSkillDescription.trim() ||
                  (newSkillEditorMode === "raw"
                    ? !newSkillBody.trim()
                    : Boolean(newSkillMethodologyPreviewError)) ||
                  busyKey === `skill:create:${newSkillName.trim() || "new"}`
                }
                onClick={onCreateSkill}
              >
                {busyKey === `skill:create:${newSkillName.trim() || "new"}`
                  ? "Creating..."
                  : "Create Skill"}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {activeSubtab === "hub" ? (
        <div className="grid gap-5">
          {/* Source config */}
          <div>
            <div className="flex items-center justify-between gap-3 mb-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Skill Hub / Sync</span>
              <span className="text-xs text-muted-foreground">
                {managedSkills.length} managed · {skillSourceIndices.length} sources
              </span>
            </div>

            <div className="grid gap-3">
              <input
                type="text"
                placeholder="source_id"
                value={skillSyncSourceId}
                onChange={(event) => onSkillSyncSourceIdChange(event.target.value)}
              />
              <select
                value={skillSyncSourceKind}
                onChange={(event) =>
                  onSkillSyncSourceKindChange(event.target.value as SkillSourceRefRecord["source_kind"])
                }
              >
                <option value="local_path">local_path</option>
                <option value="bundled">bundled</option>
                <option value="git">git</option>
                <option value="archive">archive</option>
                <option value="registry">registry</option>
              </select>
              <input
                type="text"
                placeholder="locator"
                value={skillSyncLocator}
                onChange={(event) => onSkillSyncLocatorChange(event.target.value)}
              />
              <input
                type="text"
                placeholder="revision (optional)"
                value={skillSyncRevision}
                onChange={(event) => onSkillSyncRevisionChange(event.target.value)}
              />
            </div>

            <div className="mt-3 flex flex-wrap items-center gap-2">
              <button
                className={primaryButtonClass}
                type="button"
                disabled={!selectedSourceId || !selectedSourceLocator || busyKey === `skill:sync:plan:${selectedSourceId}`}
                onClick={onPlanSkillSync}
              >
                {busyKey === `skill:sync:plan:${selectedSourceId}` ? "Planning..." : "Preview Sync Plan"}
              </button>
              <button
                className={secondaryButtonClass}
                type="button"
                disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || busyKey === `skill:sync:apply:${selectedSourceId}`}
                onClick={onApplySkillSync}
              >
                {busyKey === `skill:sync:apply:${selectedSourceId}` ? "Applying..." : "Apply Sync"}
              </button>
              <button
                className={secondaryButtonClass}
                type="button"
                disabled={!selectedSourceId || !selectedSourceLocator || busyKey === `skill:index:refresh:${selectedSourceId}`}
                onClick={onRefreshSkillSourceIndex}
              >
                {busyKey === `skill:index:refresh:${selectedSourceId}` ? "Refreshing Index..." : "Refresh Source Index"}
              </button>
              <button
                className={secondaryButtonClass}
                type="button"
                disabled={!selectedSourceId || !selectedSourceLocator || busyKey === `skill:guard:source ${selectedSourceId}`}
                onClick={onRunSelectedSourceGuard}
              >
                {busyKey === `skill:guard:source ${selectedSourceId}` ? "Scanning..." : "Run Source Guard"}
              </button>
            </div>

            {/* Selected source info */}
            {selectedSourceId || selectedSourceLocator ? (
              <div className="mt-3 border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
                <div className="grid gap-1">
                  <div><strong className="text-foreground">id:</strong> {skillSyncSourceId || "--"}</div>
                  <div><strong className="text-foreground">kind:</strong> {skillSyncSourceKind}</div>
                  <div className="break-all"><strong className="text-foreground">locator:</strong> {skillSyncLocator || "--"}</div>
                  <div><strong className="text-foreground">revision:</strong> {skillSyncRevision || "--"}</div>
                </div>
              </div>
            ) : (
              <div className="mt-3 text-sm text-muted-foreground">Select or type a source to populate sync, indexing, install, and guard actions.</div>
            )}
          </div>

          {/* Managed Skills */}
          <div>
            <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold mb-2">Managed Skills</div>
            {managedSkills.length ? (
              <div className="grid gap-2">
                {managedSkills.slice(0, 8).map((record) => (
                  <div key={record.skill_name} className="border-l-2 border-l-foreground/10 bg-muted/20 px-4 py-2 text-sm">
                    <div className="flex items-start justify-between gap-3">
                      <strong>{record.skill_name}</strong>
                      <span className="text-muted-foreground">{record.installed_revision || "--"}</span>
                    </div>
                    <div className="text-muted-foreground">{(record.source?.source_id ?? "unmanaged")} · {managedSkillStateLabel(record)}</div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="text-sm text-muted-foreground">No managed records yet.</div>
            )}
          </div>

          {/* Indexed Sources */}
          <div>
            <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold mb-2">Indexed Sources</div>
            {skillSourceIndices.length ? (
              <div className="grid gap-2">
                {skillSourceIndices.slice(0, 6).map((snapshot) => (
                  <button
                    key={snapshot.source.source_id}
                    type="button"
                    className="border-l-2 border-l-foreground/10 bg-muted/20 px-4 py-2 text-left text-sm transition-colors hover:bg-muted/40"
                    onClick={() => {
                      onSkillSyncSourceIdChange(snapshot.source.source_id);
                      onSkillSyncSourceKindChange(snapshot.source.source_kind);
                      onSkillSyncLocatorChange(snapshot.source.locator);
                      onSkillSyncRevisionChange(snapshot.source.revision ?? "");
                      onRemoteInstallSkillNameChange(snapshot.entries[0]?.skill_name ?? "");
                    }}
                  >
                    <strong>{snapshot.source.source_id}</strong>
                    <div className="text-muted-foreground">{snapshot.source.source_kind} · {snapshot.entries.length} skills</div>
                    <div className="break-all text-xs text-muted-foreground">{snapshot.source.locator}</div>
                  </button>
                ))}
              </div>
            ) : (
              <div className="text-sm text-muted-foreground">No source index cached yet.</div>
            )}
          </div>

          {/* Sync Plan */}
          {skillSyncPlan ? (
            <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 grid gap-3">
              <div className="flex items-center justify-between gap-3">
                <strong>Sync Plan · {skillSyncPlan.source_id}</strong>
                <span className="text-sm text-muted-foreground">{skillSyncPlan.entries.length} entries</span>
              </div>
              {skillSyncPlan.entries.length ? skillSyncPlan.entries.map((entry) => (
                <div key={`${entry.skill_name}:${entry.action}`} className="bg-muted/40 rounded px-3 py-2 text-sm">
                  <div className="flex items-start justify-between gap-3">
                    <strong>{entry.skill_name}</strong>
                    <span className="text-xs uppercase tracking-wide text-muted-foreground">{entry.action}</span>
                  </div>
                  <div className="mt-1 text-muted-foreground">{entry.reason}</div>
                </div>
              )) : (
                <div className="text-sm text-muted-foreground">This source currently produces an empty plan.</div>
              )}
            </div>
          ) : null}

          {/* Remote Install */}
          <div>
            <div className="flex items-center justify-between gap-3 mb-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Remote Install</span>
              <span className="text-xs text-muted-foreground">source {selectedHubSourceSnapshot?.source.source_id ?? "--"}</span>
            </div>

            <div className="grid gap-3">
              <input
                type="text"
                placeholder="remote skill name"
                value={remoteInstallSkillName}
                onChange={(event) => onRemoteInstallSkillNameChange(event.target.value)}
              />

              <div className="grid gap-2 sm:grid-cols-2">
                <button className={primaryButtonClass} type="button"
                  disabled={!selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:install:plan:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onPlanRemoteInstall}
                >
                  {busyKey === `skill:install:plan:${selectedSourceId}:${selectedRemoteSkillName}` ? "Planning..." : "Preview Install"}
                </button>
                <button className={secondaryButtonClass} type="button"
                  disabled={!selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:update:plan:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onPlanRemoteUpdate}
                >
                  {busyKey === `skill:update:plan:${selectedSourceId}:${selectedRemoteSkillName}` ? "Planning..." : "Preview Update"}
                </button>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                <button className={secondaryButtonClass} type="button"
                  disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:install:apply:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onApplyRemoteInstall}
                >
                  {busyKey === `skill:install:apply:${selectedSourceId}:${selectedRemoteSkillName}` ? "Installing..." : "Install To Workspace"}
                </button>
                <button className={secondaryButtonClass} type="button"
                  disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:update:apply:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onApplyRemoteUpdate}
                >
                  {busyKey === `skill:update:apply:${selectedSourceId}:${selectedRemoteSkillName}` ? "Updating..." : "Update Workspace"}
                </button>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                <button className="min-h-[36px] rounded-lg border border-amber-300 bg-amber-50/80 px-4 text-sm text-amber-950 inline-flex items-center justify-center cursor-pointer transition-colors disabled:cursor-not-allowed disabled:opacity-60 dark:border-amber-700 dark:bg-amber-950/40 dark:text-amber-200 dark:hover:bg-amber-950/60" type="button"
                  disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:detach:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onDetachManagedSkill}
                >
                  {busyKey === `skill:detach:${selectedSourceId}:${selectedRemoteSkillName}` ? "Detaching..." : "Detach Managed"}
                </button>
                <button className="min-h-[36px] rounded-lg border border-red-300 bg-red-50/80 px-4 text-sm text-red-900 inline-flex items-center justify-center cursor-pointer transition-colors disabled:cursor-not-allowed disabled:opacity-60 dark:border-red-700 dark:bg-red-950/40 dark:text-red-200 dark:hover:bg-red-950/60" type="button"
                  disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:remove:${selectedSourceId}:${selectedRemoteSkillName}`}
                  onClick={onRemoveManagedSkill}
                >
                  {busyKey === `skill:remove:${selectedSourceId}:${selectedRemoteSkillName}` ? "Removing..." : "Remove Managed"}
                </button>
              </div>

              <div className="text-xs text-muted-foreground">
                Update re-applies through the unified lifecycle pipeline. Detach drops managed ownership but keeps files. Remove clears managed state and deletes workspace copy when clean.
              </div>
            </div>

            {/* Selected source entry */}
            {selectedRemoteSourceEntry ? (
              <div className="mt-3 border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm">
                <div className="flex items-start justify-between gap-3">
                  <strong>{selectedRemoteSourceEntry.skill_name}</strong>
                  <span className="text-muted-foreground">{selectedRemoteSourceEntry.revision || "--"}</span>
                </div>
                <div className="mt-1 text-muted-foreground">
                  {selectedRemoteSourceEntry.category ? `${selectedRemoteSourceEntry.category} · ` : ""}
                  {selectedRemoteSourceEntry.description || "No remote description provided."}
                </div>
              </div>
            ) : (
              <div className="mt-3 text-sm text-muted-foreground">Type a skill name from the selected source index to preview or apply a remote install.</div>
            )}

            {/* Indexed entries */}
            {selectedHubSourceSnapshot?.entries.length ? (
              <div className="mt-3 grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Indexed Entries</span>
                  <span className="text-xs text-muted-foreground">{Math.min(selectedHubSourceSnapshot.entries.length, 12)} shown</span>
                </div>
                <div className="max-h-[24rem] overflow-y-auto pr-1 grid gap-2 sm:grid-cols-2">
                  {selectedHubSourceSnapshot.entries.slice(0, 12).map((entry) => {
                    const selected = entry.skill_name.trim().toLowerCase() === selectedRemoteSkillName.toLowerCase();
                    return (
                      <button key={entry.skill_name} type="button"
                        className={cn("border-l-2 px-3 py-2 text-left text-sm transition-colors", selected ? "border-l-foreground/40 bg-foreground/5" : "border-l-transparent bg-muted/20 hover:bg-muted/40")}
                        onClick={() => onRemoteInstallSkillNameChange(entry.skill_name)}
                      >
                        <strong>{entry.skill_name}</strong>
                        <div className="text-xs text-muted-foreground">{entry.category ? `${entry.category} · ` : ""}{entry.revision || "unversioned"}</div>
                        <div className="mt-1 text-xs text-muted-foreground">{entry.description || "No description"}</div>
                      </button>
                    );
                  })}
                </div>
              </div>
            ) : null}

            {/* Remote install plan */}
            {remoteInstallPlan ? (
              <div className="mt-3 border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <strong>Remote Install Plan · {remoteInstallPlan.entry.skill_name}</strong>
                  <span className={cn("rounded-full border px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wide", lifecycleStatusClass(remoteInstallPlan.entry.action))}>
                    {remoteInstallPlan.entry.action}
                  </span>
                </div>
                <div className="text-sm text-muted-foreground">{remoteInstallPlan.entry.reason}</div>
                <div className="grid gap-1 text-sm text-muted-foreground">
                  <div>source <code>{remoteInstallPlan.source_id}</code></div>
                  <div>distribution <code>{remoteInstallPlan.distribution.distribution_id}</code></div>
                  <div>artifact <code>{remoteInstallPlan.distribution.resolution.artifact.artifact_id}</code></div>
                  <div>locator <code>{remoteInstallPlan.distribution.resolution.artifact.locator}</code></div>
                </div>
              </div>
            ) : null}

            {/* Hub Policy */}
            <div className="mt-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Hub Policy</span>
              <div className="mt-2 text-sm text-muted-foreground">
                {skillHubPolicy ? (
                  <div className="grid gap-1">
                    <span>retention {formatHubDurationSeconds(skillHubPolicy.artifact_cache_retention_seconds)}</span>
                    <span>timeout {formatHubDurationMs(skillHubPolicy.fetch_timeout_ms)}</span>
                    <span>max download {formatHubBytes(skillHubPolicy.max_download_bytes)}</span>
                    <span>max extract {formatHubBytes(skillHubPolicy.max_extract_bytes)}</span>
                  </div>
                ) : "No hub policy loaded yet."}
              </div>
            </div>

            {/* Distribution Snapshot */}
            <div className="mt-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Distribution</span>
              {selectedRemoteDistribution ? (
                <div className="mt-2 text-sm text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <strong className="text-foreground">{selectedRemoteDistribution.skill_name}</strong>
                    <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteDistribution.lifecycle))}>
                      {selectedRemoteDistribution.lifecycle}
                    </span>
                  </div>
                  <span>release {selectedRemoteDistribution.release.version || "--"} · revision {selectedRemoteDistribution.release.revision || "--"}</span>
                  {selectedRemoteDistribution.installed ? (
                    <span className="block mt-1 text-xs">installed {unixTimeLabel(selectedRemoteDistribution.installed.installed_at)} · {selectedRemoteDistribution.installed.workspace_skill_path}</span>
                  ) : null}
                </div>
              ) : <div className="mt-2 text-sm text-muted-foreground">No resolved distribution recorded.</div>}
            </div>

            {/* Artifact Cache */}
            <div className="mt-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Artifact Cache</span>
              {selectedRemoteArtifactCache ? (
                <div className="mt-2 text-sm text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <strong className="text-foreground">{selectedRemoteArtifactCache.artifact.artifact_id}</strong>
                    <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteArtifactCache.status))}>
                      {selectedRemoteArtifactCache.status}
                    </span>
                  </div>
                  <span className="break-all">cached {unixTimeLabel(selectedRemoteArtifactCache.cached_at)} · {selectedRemoteArtifactCache.local_path}</span>
                  {selectedRemoteArtifactCache.error ? (
                    <div className="mt-1 text-xs text-red-700 dark:text-red-300">{selectedRemoteArtifactCache.error}</div>
                  ) : null}
                </div>
              ) : <div className="mt-2 text-sm text-muted-foreground">No artifact cache entry yet.</div>}
            </div>

            {/* Lifecycle State */}
            <div className="mt-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Lifecycle State</span>
              {selectedRemoteLifecycle ? (
                <div className="mt-2 text-sm text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <strong className="text-foreground">{selectedRemoteLifecycle.skill_name}</strong>
                    <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteLifecycle.state))}>
                      {selectedRemoteLifecycle.state}
                    </span>
                  </div>
                  <span>updated {unixTimeLabel(selectedRemoteLifecycle.updated_at)}</span>
                  {selectedRemoteLifecycle.error ? (
                    <div className="mt-1 text-xs text-red-700 dark:text-red-300">{selectedRemoteLifecycle.error}</div>
                  ) : null}
                </div>
              ) : <div className="mt-2 text-sm text-muted-foreground">No lifecycle record yet.</div>}
            </div>
          </div>
        </div>
      ) : null}

      {activeSubtab === "catalog" ? (
        <div className="relative">
          {/* List view */}
          <div className="grid gap-2 max-h-[28rem] overflow-y-auto pr-1">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                Catalog
              </span>
              <span className="text-xs text-muted-foreground">
                {skillCatalog.length} skills
              </span>
            </div>

            {skillCatalog.length ? (
              skillCatalog.map((skill) => {
                const selected = selectedSkillEntry?.name === skill.name;
                const managedRecord =
                  managedRecordBySkill.get(skill.name.trim().toLowerCase()) ?? null;
                const latestGuard =
                  latestGuardBySkill.get(skill.name.trim().toLowerCase()) ?? null;

                return (
                  <button
                    key={skill.name}
                    type="button"
                    className={cn(
                      "grid gap-1.5 rounded-lg border-l-2 px-4 py-3 text-left transition-colors",
                      selected
                        ? "border-l-foreground/40 bg-foreground/5"
                        : "border-l-transparent bg-muted/20 hover:bg-muted/40"
                    )}
                    onClick={() => onSelectedSkillNameChange(skill.name)}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <strong className="block truncate text-sm">{skill.name}</strong>
                        <p className="m-0 mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                          {skill.description || "No description"}
                        </p>
                      </div>
                      <span
                        className={cn(
                          "shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                          skill.writable
                            ? "border-green-300 bg-green-50 text-green-800 dark:border-green-700 dark:bg-green-950 dark:text-green-300"
                            : "border-border bg-muted text-muted-foreground",
                        )}
                      >
                        {skill.writable ? "workspace" : "read only"}
                      </span>
                    </div>
                    <div className="flex flex-wrap gap-1.5 text-[10px]">
                      <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                        {skill.supporting_files.length} files
                      </span>
                      {skill.category ? (
                        <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                          {skill.category}
                        </span>
                      ) : null}
                      {managedRecord ? (
                        <span
                          className={cn(
                            "rounded-full border px-2 py-0.5",
                            managedRecord.locally_modified || managedRecord.deleted_locally
                              ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200"
                              : "border-border/40 bg-muted text-muted-foreground",
                          )}
                        >
                          {managedSkillStateLabel(managedRecord)}
                        </span>
                      ) : null}
                      {latestGuard ? (
                        <span
                          className={cn(
                            "rounded-full border px-2 py-0.5",
                            latestGuard.status === "blocked"
                              ? "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950/60 dark:text-red-300"
                              : latestGuard.status === "warn"
                                ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200"
                                : "border-border/40 bg-muted text-muted-foreground",
                          )}
                        >
                          {latestGuardStatusLabel(latestGuard)}
                        </span>
                      ) : null}
                    </div>
                  </button>
                );
              })
            ) : (
              <div className={mutedCardClass}>No skills discovered yet.</div>
            )}
          </div>

          {/* Master-detail overlay */}
          {selectedSkillEntry && !skillDetailLoading ? (
            <div className="absolute inset-0 z-10 grid gap-4 overflow-y-auto rounded-lg bg-background p-4 shadow-lg">
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                  onClick={() => onSelectedSkillNameChange("")}
                >
                  ← Back to catalog
                </button>
                {selectedSkillEntry ? (
                  <span
                    className={cn(
                      "rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                      selectedSkillEntry.writable
                        ? "border-green-300 bg-green-50 text-green-800 dark:border-green-700 dark:bg-green-950 dark:text-green-300"
                        : "border-border bg-muted text-muted-foreground",
                    )}
                  >
                    {selectedSkillEntry.writable ? "workspace" : "read only"}
                  </span>
                ) : null}
              </div>

              {/* Skill header */}
              <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                <strong className="block text-base text-foreground">{selectedSkillEntry.name}</strong>
                <p className="m-0 mt-1 text-sm text-muted-foreground">
                  {selectedSkillEntry.description || "No description"}
                </p>
                <div className="mt-2 flex flex-wrap gap-1.5 text-[10px]">
                  {selectedManagedRecord ? (
                    <>
                      <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                        source {selectedManagedRecord.source?.source_id || "workspace-local"}
                      </span>
                      <span
                        className={cn(
                          "rounded-full border px-2 py-0.5",
                          selectedManagedRecord.locally_modified || selectedManagedRecord.deleted_locally
                            ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200"
                            : "border-border/40 bg-muted text-muted-foreground",
                        )}
                      >
                        {managedSkillStateLabel(selectedManagedRecord)}
                      </span>
                    </>
                  ) : null}
                  {selectedLatestGuard ? (
                    <span
                      className={cn(
                        "rounded-full border px-2 py-0.5",
                        selectedLatestGuard.status === "blocked"
                          ? "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950/60 dark:text-red-300"
                          : selectedLatestGuard.status === "warn"
                            ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200"
                            : "border-border/40 bg-muted text-muted-foreground",
                      )}
                    >
                      {latestGuardStatusLabel(selectedLatestGuard)} · {selectedLatestGuard.violations.length} violations
                    </span>
                  ) : null}
                </div>
              </div>

              {/* Metadata */}
              <div className="text-sm text-muted-foreground">
                <span>Location: {selectedSkillEntry.location}</span>
                <span className="mx-2">·</span>
                <span>Category: {selectedSkillEntry.category || "--"}</span>
                <span className="mx-2">·</span>
                <span>{selectedSkillEntry.supporting_files.length} files</span>
              </div>
              {!selectedSkillEntry.writable ? (
                <div className="text-xs text-amber-700 dark:text-amber-300">
                  Read-only skill outside workspace root <code>{skillWorkspaceRoot}</code>. Edits disabled.
                </div>
              ) : null}

              {/* Editor mode toggle */}
              <div className="flex flex-wrap gap-2">
                {(["methodology", "raw"] as SkillEditorMode[]).map((mode) => (
                  <button
                    key={mode}
                    type="button"
                    className={cn(
                      "rounded-full border px-3 py-1.5 text-xs font-semibold transition-colors",
                      editSkillEditorMode === mode
                        ? "border-border bg-accent text-foreground"
                        : "border-border/50 bg-background/60 text-muted-foreground hover:bg-accent/60",
                    )}
                    onClick={() => onEditSkillEditorModeChange(mode)}
                    disabled={!selectedSkillEntry.writable || skillDetailLoading}
                  >
                    {mode === "methodology" ? "Methodology Form" : "Raw Markdown"}
                  </button>
                ))}
              </div>

              {/* Editor body */}
              {editSkillEditorMode === "methodology" ? (
                <div className="grid gap-3">
                  <input
                    type="text"
                    placeholder="description"
                    value={editSkillDescription}
                    onChange={(event) => onEditSkillDescriptionChange(event.target.value)}
                    disabled={!selectedSkillEntry.writable}
                  />
                  {!editSkillMethodologyMatched ? (
                    <div className="rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200">
                      Current SKILL.md did not round-trip into the methodology template. Saving
                      in methodology mode will rewrite the body from this structured form.
                    </div>
                  ) : null}
                  <SkillMethodologyEditor
                    draft={editSkillMethodologyDraft}
                    onChange={onEditSkillMethodologyDraftChange}
                    previewBody={editSkillMethodologyPreview}
                    previewError={editSkillMethodologyPreviewError}
                    disabled={!selectedSkillEntry.writable}
                  />
                </div>
              ) : (
                <textarea
                  className="min-h-[24rem] w-full resize-y rounded-lg border border-border/40 bg-background p-3.5 text-sm leading-relaxed font-mono text-foreground"
                  value={skillEditorContent}
                  onChange={(event) => onSkillEditorContentChange(event.target.value)}
                  spellCheck={false}
                  readOnly={!selectedSkillEntry.writable}
                />
              )}

              {/* Actions */}
              <div className="flex items-center gap-2">
                <button
                  className={secondaryButtonClass}
                  type="button"
                  disabled={busyKey === `skill:guard:skill ${selectedSkillEntry.name}`}
                  onClick={onRunSelectedSkillGuard}
                >
                  {busyKey === `skill:guard:skill ${selectedSkillEntry.name}`
                    ? "Scanning..."
                    : "Run Guard Check"}
                </button>
                <button
                  className={primaryButtonClass}
                  type="button"
                  disabled={
                    !skillsMutationsEnabled ||
                    !selectedSkillEntry.writable ||
                    skillDetailLoading ||
                    (editSkillEditorMode === "methodology" &&
                      (!editSkillDescription.trim() ||
                        Boolean(editSkillMethodologyPreviewError))) ||
                    busyKey === `skill:edit:${selectedSkillEntry.name}`
                  }
                  onClick={onSaveSelectedSkill}
                >
                  {busyKey === `skill:edit:${selectedSkillEntry.name}` ? "Saving..." : "Save Skill"}
                </button>
                <button
                  className={secondaryButtonClass}
                  type="button"
                  disabled={
                    !skillsMutationsEnabled ||
                    !selectedSkillEntry.writable ||
                    busyKey === `skill:delete:${selectedSkillEntry.name}`
                  }
                  onClick={onDeleteSelectedSkill}
                >
                  {busyKey === `skill:delete:${selectedSkillEntry.name}`
                    ? "Deleting..."
                    : "Delete Skill"}
                </button>
              </div>
            </div>
          ) : null}

          {/* Loading overlay */}
          {selectedSkillEntry && skillDetailLoading ? (
            <div className="absolute inset-0 z-10 flex items-center justify-center rounded-lg bg-background/80">
              <span className="text-sm text-muted-foreground">Loading skill source...</span>
            </div>
          ) : null}
        </div>
      ) : null}

      {activeSubtab === "governance" ? (
        <div className="grid gap-5">
          <div>
            <div className="flex items-center justify-between gap-3 mb-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                Guard Summary
              </span>
              <span className="text-xs text-muted-foreground">
                {selectedSkillName ? `skill ${selectedSkillName}` : "all skills"}
              </span>
            </div>

            <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
              <span><strong className="text-foreground">{skillGuardReports.length}</strong> reports</span>
              <span><strong className="text-foreground">{blockedGuardCount}</strong> blocked</span>
              <span><strong className="text-foreground">{warnedGuardCount}</strong> warn</span>
              <span><strong className="text-foreground">{totalGuardViolations}</strong> violations</span>
            </div>

            {skillGuardTarget ? (
              <div className="mt-3 text-xs text-muted-foreground">
                Latest guard run targeted <code>{skillGuardTarget}</code> and returned{" "}
                {skillGuardReports.length} report{skillGuardReports.length === 1 ? "" : "s"}.
              </div>
            ) : null}
          </div>

          <div className="grid gap-2">
            <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
              Recent Guard Reports
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
                          ? "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950/60 dark:text-red-300"
                          : report.status === "warn"
                            ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200"
                            : "border-border/40 bg-muted text-muted-foreground",
                      )}
                    >
                      {latestGuardStatusLabel(report)}
                    </span>
                  </div>
                  <div className="mt-2 text-muted-foreground">
                    {report.violations.length
                      ? `${report.violations.length} violation${report.violations.length === 1 ? "" : "s"}`
                      : `${passedGuardCount ? "no violations" : "guard passed"}`}
                    {" · "}
                    scanned {unixTimeLabel(report.scanned_at)}
                  </div>
                </div>
              ))
            ) : (
              <div className="rounded-lg bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
                No guard reports recorded yet.
              </div>
            )}
          </div>

          <SkillGovernanceTimeline
            entries={skillGovernanceTimeline}
            selectedSkillName={selectedSkillName}
            selectedSourceId={selectedSourceId || null}
          />
        </div>
      ) : null}
    </div>
  );
}
