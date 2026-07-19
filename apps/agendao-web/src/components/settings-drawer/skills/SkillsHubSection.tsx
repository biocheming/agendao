import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  ManagedSkillRecord,
  SkillArtifactCacheEntryRecord,
  SkillDistributionRecord,
  SkillHubPolicyRecord,
  SkillManagedLifecycleRecord,
  SkillRemoteInstallPlanRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
  SkillSyncPlanRecord,
} from "@/lib/skill";
import type { SkillHubSearchResult } from "./shared";
import {
  formatHubBytes,
  formatHubDurationMs,
  formatHubDurationSeconds,
  lifecycleStatusClass,
  managedSkillStateLabel,
  unixTimeLabel,
} from "./shared";

interface SkillsHubSectionStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
}

export interface SkillsHubSectionProps {
  styles: SkillsHubSectionStyles;
  busyKey: string | null;
  skillsMutationsEnabled: boolean;
  hubSearchDraft: string;
  onHubSearchDraftChange: (value: string) => void;
  hubSearchResults: SkillHubSearchResult[];
  managedSkills: ManagedSkillRecord[];
  skillSourceIndices: SkillSourceIndexSnapshotRecord[];
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
  skillHubPolicy: SkillHubPolicyRecord | null;
  selectedHubSourceSnapshot: SkillSourceIndexSnapshotRecord | null;
  selectedRemoteSourceEntry: SkillSourceIndexSnapshotRecord["entries"][number] | null;
  selectedRemoteDistribution: SkillDistributionRecord | null;
  selectedRemoteArtifactCache: SkillArtifactCacheEntryRecord | null;
  selectedRemoteLifecycle: SkillManagedLifecycleRecord | null;
  selectedSourceId: string;
  selectedSourceLocator: string;
  selectedRemoteSkillName: string;
  onSelectHubRemoteSkill: (source: SkillSourceRefRecord, skillName: string) => void;
}

export function SkillsHubSection({
  styles,
  busyKey,
  skillsMutationsEnabled,
  hubSearchDraft,
  onHubSearchDraftChange,
  hubSearchResults,
  managedSkills,
  skillSourceIndices,
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
  skillHubPolicy,
  selectedHubSourceSnapshot,
  selectedRemoteSourceEntry,
  selectedRemoteDistribution,
  selectedRemoteArtifactCache,
  selectedRemoteLifecycle,
  selectedSourceId,
  selectedSourceLocator,
  selectedRemoteSkillName,
  onSelectHubRemoteSkill,
}: SkillsHubSectionProps) {
  const { primaryButtonClass, secondaryButtonClass } = styles;
  const { t } = useI18n();

  return (
    <div className="grid gap-5" data-testid="settings-skills-hub">
      {/* Search + select */}
      <div>
        <div className="flex items-center justify-between gap-3 mb-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.searchTitle")}</span>
          <span className="text-xs text-muted-foreground">
            {t("settings.skills.hub.searchCount", { shown: hubSearchResults.length, indexed: skillSourceIndices.reduce((count, source) => count + source.entries.length, 0) })}
          </span>
        </div>

        <div className="grid gap-3">
          <input
            type="search"
            data-testid="settings-skills-hub-search"
            placeholder={t("settings.skills.hub.searchPlaceholder")}
            value={hubSearchDraft}
            onChange={(event) => onHubSearchDraftChange(event.target.value)}
          />

          {skillSourceIndices.length ? (
            <div className="max-h-[22rem] overflow-y-auto pr-1 grid gap-2" data-testid="settings-skills-hub-results">
              {hubSearchResults.length ? hubSearchResults.map((result) => {
                const selected =
                  result.source.source_id === selectedSourceId &&
                  result.entry.skill_name.trim().toLowerCase() === selectedRemoteSkillName.toLowerCase();
                const lifecycleState = result.lifecycle?.state ?? result.distribution?.lifecycle ?? null;
                return (
                  <button
                    key={`${result.source.source_id}:${result.entry.skill_name}:${result.entry.revision ?? ""}`}
                    type="button"
                    data-testid="settings-skills-hub-result"
                    className={cn(
                      "grid gap-2 rounded-lg border-l-2 px-4 py-3 text-left text-sm transition-colors",
                      selected
                        ? "border-l-foreground/50 bg-foreground/5"
                        : "border-l-transparent bg-muted/20 hover:bg-muted/40",
                    )}
                    onClick={() => onSelectHubRemoteSkill(result.source, result.entry.skill_name)}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <strong className="block truncate text-foreground">{result.entry.skill_name}</strong>
                        <p className="m-0 mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                          {result.entry.description || t("settings.skills.hub.noDescription")}
                        </p>
                      </div>
                      <span className="shrink-0 rounded-full border border-border/40 bg-muted px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        {result.source.source_id}
                      </span>
                    </div>
                    <div className="flex flex-wrap gap-1.5 text-[10px]">
                      {result.entry.category ? (
                        <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                          {result.entry.category}
                        </span>
                      ) : null}
                      <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                        {result.entry.revision || result.source.revision || t("settings.skills.hub.unversioned")}
                      </span>
                      {result.installedRecord ? (
                        <span
                          className={cn(
                            "rounded-full border px-2 py-0.5",
                            result.installedRecord.locally_modified || result.installedRecord.deleted_locally
                              ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                              : "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)",
                          )}
                        >
                          {managedSkillStateLabel(result.installedRecord, t)}
                        </span>
                      ) : null}
                      {lifecycleState ? (
                        <span className={cn("rounded-full border px-2 py-0.5", lifecycleStatusClass(lifecycleState))}>
                          {lifecycleState}
                        </span>
                      ) : null}
                    </div>
                  </button>
                );
              }) : (
                <div className="rounded-lg border border-border/40 bg-muted/20 px-4 py-6 text-center text-sm text-muted-foreground" data-testid="settings-skills-hub-results-empty">
                  {t("settings.skills.hub.resultsEmpty")}
                </div>
              )}
            </div>
          ) : (
            <div className="rounded-lg border border-border/40 bg-muted/20 px-4 py-6 text-center text-sm text-muted-foreground" data-testid="settings-skills-hub-sources-empty">
              {t("settings.skills.hub.sourcesEmpty")}
            </div>
          )}
        </div>
      </div>

      {/* Source config */}
      <div>
        <div className="flex items-center justify-between gap-3 mb-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.syncTitle")}</span>
          <span className="text-xs text-muted-foreground">
            {t("settings.skills.hub.managedSourcesCount", { managed: managedSkills.length, sources: skillSourceIndices.length })}
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
            placeholder={t("settings.skills.hub.revisionPlaceholder")}
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
            {busyKey === `skill:sync:plan:${selectedSourceId}` ? t("settings.skills.hub.planning") : t("settings.skills.hub.previewSyncPlan")}
          </button>
          <button
            className={secondaryButtonClass}
            type="button"
            disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || busyKey === `skill:sync:apply:${selectedSourceId}`}
            onClick={onApplySkillSync}
          >
            {busyKey === `skill:sync:apply:${selectedSourceId}` ? t("settings.skills.hub.applying") : t("settings.skills.hub.applySync")}
          </button>
          <button
            className={secondaryButtonClass}
            type="button"
            disabled={!selectedSourceId || !selectedSourceLocator || busyKey === `skill:index:refresh:${selectedSourceId}`}
            onClick={onRefreshSkillSourceIndex}
          >
            {busyKey === `skill:index:refresh:${selectedSourceId}` ? t("settings.skills.hub.refreshingIndex") : t("settings.skills.hub.refreshSourceIndex")}
          </button>
          <button
            className={secondaryButtonClass}
            type="button"
            disabled={!selectedSourceId || !selectedSourceLocator || busyKey === `skill:guard:source ${selectedSourceId}`}
            onClick={onRunSelectedSourceGuard}
          >
            {busyKey === `skill:guard:source ${selectedSourceId}` ? t("settings.skills.hub.scanning") : t("settings.skills.hub.runSourceGuard")}
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
          <div className="mt-3 text-sm text-muted-foreground">{t("settings.skills.hub.selectSourceHint")}</div>
        )}
      </div>

      {/* Managed Skills */}
      <div>
        <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold mb-2">{t("settings.skills.hub.managedSkills")}</div>
        {managedSkills.length ? (
          <div className="grid gap-2" data-testid="settings-skills-managed-list">
            {managedSkills.slice(0, 8).map((record) => (
              <div key={record.skill_name} className="border-l-2 border-l-foreground/10 bg-muted/20 px-4 py-2 text-sm" data-testid="settings-skills-managed-item">
                <div className="flex items-start justify-between gap-3">
                  <strong>{record.skill_name}</strong>
                  <span className="text-muted-foreground">{record.installed_revision || "--"}</span>
                </div>
                <div className="text-muted-foreground">{(record.source?.source_id ?? t("settings.skills.hub.unmanaged"))} · {managedSkillStateLabel(record, t)}</div>
              </div>
            ))}
          </div>
        ) : (
          <div className="text-sm text-muted-foreground" data-testid="settings-skills-managed-empty">{t("settings.skills.hub.noManaged")}</div>
        )}
      </div>

      {/* Indexed Sources */}
      <div>
        <div className="text-xs tracking-widest uppercase text-muted-foreground font-semibold mb-2">{t("settings.skills.hub.indexedSources")}</div>
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
                <div className="text-muted-foreground">{snapshot.source.source_kind} · {t("settings.skills.hub.skillsCount", { count: snapshot.entries.length })}</div>
                <div className="break-all text-xs text-muted-foreground">{snapshot.source.locator}</div>
              </button>
            ))}
          </div>
        ) : (
          <div className="text-sm text-muted-foreground">{t("settings.skills.hub.noSourceIndex")}</div>
        )}
      </div>

      {/* Sync Plan */}
      {skillSyncPlan ? (
        <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 grid gap-3">
          <div className="flex items-center justify-between gap-3">
            <strong>{t("settings.skills.hub.syncPlanTitle", { source: skillSyncPlan.source_id })}</strong>
            <span className="text-sm text-muted-foreground">{t("settings.skills.hub.entriesCount", { count: skillSyncPlan.entries.length })}</span>
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
            <div className="text-sm text-muted-foreground">{t("settings.skills.hub.emptyPlan")}</div>
          )}
        </div>
      ) : null}

      {/* Remote Install */}
      <div>
        <div className="flex items-center justify-between gap-3 mb-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.remoteInstall")}</span>
          <span className="text-xs text-muted-foreground">{t("settings.skills.sourcePrefix", { value: selectedHubSourceSnapshot?.source.source_id ?? "--" })}</span>
        </div>

        <div className="grid gap-3">
          <input
            type="text"
            placeholder={t("settings.skills.hub.remoteSkillNamePlaceholder")}
            value={remoteInstallSkillName}
            onChange={(event) => onRemoteInstallSkillNameChange(event.target.value)}
          />

          <div className="grid gap-2 sm:grid-cols-2">
            <button className={primaryButtonClass} type="button"
              disabled={!selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:install:plan:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onPlanRemoteInstall}
            >
              {busyKey === `skill:install:plan:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.planning") : t("settings.skills.hub.previewInstall")}
            </button>
            <button className={secondaryButtonClass} type="button"
              disabled={!selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:update:plan:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onPlanRemoteUpdate}
            >
              {busyKey === `skill:update:plan:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.planning") : t("settings.skills.hub.previewUpdate")}
            </button>
          </div>
          <div className="grid gap-2 sm:grid-cols-2">
            <button className={secondaryButtonClass} type="button"
              disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:install:apply:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onApplyRemoteInstall}
            >
              {busyKey === `skill:install:apply:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.installing") : t("settings.skills.hub.installToWorkspace")}
            </button>
            <button className={secondaryButtonClass} type="button"
              disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:update:apply:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onApplyRemoteUpdate}
            >
              {busyKey === `skill:update:apply:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.updating") : t("settings.skills.hub.updateWorkspace")}
            </button>
          </div>
          <div className="grid gap-2 sm:grid-cols-2">
            <button className="min-h-[36px] rounded-lg border border-(--ds-warn)/40 bg-(--ds-warn)/12 px-4 text-sm text-(--ds-warn) inline-flex items-center justify-center cursor-pointer transition-colors disabled:cursor-not-allowed disabled:opacity-60 hover:bg-(--ds-warn)/18" type="button"
              disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:detach:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onDetachManagedSkill}
            >
              {busyKey === `skill:detach:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.detaching") : t("settings.skills.hub.detachManaged")}
            </button>
            <button className="min-h-[36px] rounded-lg border border-(--ds-error)/40 bg-(--ds-error)/12 px-4 text-sm text-(--ds-error) inline-flex items-center justify-center cursor-pointer transition-colors disabled:cursor-not-allowed disabled:opacity-60 hover:bg-(--ds-error)/18" type="button"
              disabled={!skillsMutationsEnabled || !selectedSourceId || !selectedSourceLocator || !selectedRemoteSkillName || busyKey === `skill:remove:${selectedSourceId}:${selectedRemoteSkillName}`}
              onClick={onRemoveManagedSkill}
            >
              {busyKey === `skill:remove:${selectedSourceId}:${selectedRemoteSkillName}` ? t("settings.skills.hub.removing") : t("settings.skills.hub.removeManaged")}
            </button>
          </div>

          <div className="text-xs text-muted-foreground">
            {t("settings.skills.hub.lifecycleNote")}
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
              {selectedRemoteSourceEntry.description || t("settings.skills.hub.noRemoteDescription")}
            </div>
          </div>
        ) : (
          <div className="mt-3 text-sm text-muted-foreground">{t("settings.skills.hub.typeSkillHint")}</div>
        )}

        {/* Indexed entries */}
        {selectedHubSourceSnapshot?.entries.length ? (
          <div className="mt-3 grid gap-3">
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.indexedEntries")}</span>
              <span className="text-xs text-muted-foreground">{t("settings.skills.hub.entriesShownCount", { count: Math.min(selectedHubSourceSnapshot.entries.length, 12) })}</span>
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
                    <div className="text-xs text-muted-foreground">{entry.category ? `${entry.category} · ` : ""}{entry.revision || t("settings.skills.hub.unversioned")}</div>
                    <div className="mt-1 text-xs text-muted-foreground">{entry.description || t("settings.skills.catalog.noDescription")}</div>
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
              <strong>{t("settings.skills.hub.remoteInstallPlanTitle", { name: remoteInstallPlan.entry.skill_name })}</strong>
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
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.hubPolicy")}</span>
          <div className="mt-2 text-sm text-muted-foreground">
            {skillHubPolicy ? (
              <div className="grid gap-1">
                <span>{t("settings.skills.hub.policyRetention", { value: formatHubDurationSeconds(skillHubPolicy.artifact_cache_retention_seconds) })}</span>
                <span>{t("settings.skills.hub.policyTimeout", { value: formatHubDurationMs(skillHubPolicy.fetch_timeout_ms) })}</span>
                <span>{t("settings.skills.hub.policyMaxDownload", { value: formatHubBytes(skillHubPolicy.max_download_bytes) })}</span>
                <span>{t("settings.skills.hub.policyMaxExtract", { value: formatHubBytes(skillHubPolicy.max_extract_bytes) })}</span>
              </div>
            ) : t("settings.skills.hub.noHubPolicy")}
          </div>
        </div>

        {/* Distribution Snapshot */}
        <div className="mt-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.distribution")}</span>
          {selectedRemoteDistribution ? (
            <div className="mt-2 text-sm text-muted-foreground">
              <div className="flex items-center gap-2">
                <strong className="text-foreground">{selectedRemoteDistribution.skill_name}</strong>
                <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteDistribution.lifecycle))}>
                  {selectedRemoteDistribution.lifecycle}
                </span>
              </div>
              <span>{t("settings.skills.hub.releaseRevision", { version: selectedRemoteDistribution.release.version || "--", revision: selectedRemoteDistribution.release.revision || "--" })}</span>
              {selectedRemoteDistribution.installed ? (
                <span className="block mt-1 text-xs">{t("settings.skills.hub.installedAt", { time: unixTimeLabel(selectedRemoteDistribution.installed.installed_at), path: selectedRemoteDistribution.installed.workspace_skill_path })}</span>
              ) : null}
            </div>
          ) : <div className="mt-2 text-sm text-muted-foreground">{t("settings.skills.hub.noDistribution")}</div>}
        </div>

        {/* Artifact Cache */}
        <div className="mt-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.artifactCache")}</span>
          {selectedRemoteArtifactCache ? (
            <div className="mt-2 text-sm text-muted-foreground">
              <div className="flex items-center gap-2">
                <strong className="text-foreground">{selectedRemoteArtifactCache.artifact.artifact_id}</strong>
                <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteArtifactCache.status))}>
                  {selectedRemoteArtifactCache.status}
                </span>
              </div>
              <span className="break-all">{t("settings.skills.hub.cachedAt", { time: unixTimeLabel(selectedRemoteArtifactCache.cached_at), path: selectedRemoteArtifactCache.local_path })}</span>
              {selectedRemoteArtifactCache.error ? (
                <div className="mt-1 text-xs text-(--ds-error)">{selectedRemoteArtifactCache.error}</div>
              ) : null}
            </div>
          ) : <div className="mt-2 text-sm text-muted-foreground">{t("settings.skills.hub.noArtifactCache")}</div>}
        </div>

        {/* Lifecycle State */}
        <div className="mt-3">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.hub.lifecycleState")}</span>
          {selectedRemoteLifecycle ? (
            <div className="mt-2 text-sm text-muted-foreground">
              <div className="flex items-center gap-2">
                <strong className="text-foreground">{selectedRemoteLifecycle.skill_name}</strong>
                <span className={cn("rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide", lifecycleStatusClass(selectedRemoteLifecycle.state))}>
                  {selectedRemoteLifecycle.state}
                </span>
              </div>
              <span>{t("settings.skills.hub.updatedAt", { time: unixTimeLabel(selectedRemoteLifecycle.updated_at) })}</span>
              {selectedRemoteLifecycle.error ? (
                <div className="mt-1 text-xs text-(--ds-error)">{selectedRemoteLifecycle.error}</div>
              ) : null}
            </div>
          ) : <div className="mt-2 text-sm text-muted-foreground">{t("settings.skills.hub.noLifecycle")}</div>}
        </div>
      </div>
    </div>
  );
}
