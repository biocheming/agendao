import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  ManagedSkillRecord,
  SkillCatalogEntry,
  SkillGuardReportRecord,
} from "@/lib/skill";
import type { SkillEditorMode } from "../types";
import { SkillMethodologyEditor } from "../../settings/SkillMethodologyEditor";
import type { SkillMethodologyDraft } from "../../settings/SkillMethodologyEditor";
import { latestGuardStatusLabel, managedSkillStateLabel } from "./shared";

interface SkillsCatalogSectionStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  mutedCardClass: string;
}

export interface SkillsCatalogSectionProps {
  styles: SkillsCatalogSectionStyles;
  busyKey: string | null;
  skillsMutationsEnabled: boolean;
  skillWorkspaceRoot: string;
  skillCatalog: SkillCatalogEntry[];
  selectedSkillEntry: SkillCatalogEntry | null;
  onSelectedSkillNameChange: (value: string) => void;
  managedRecordBySkill: Map<string, ManagedSkillRecord>;
  latestGuardBySkill: Map<string, SkillGuardReportRecord>;
  selectedManagedRecord: ManagedSkillRecord | null;
  selectedLatestGuard: SkillGuardReportRecord | null;
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
  onRunSelectedSkillGuard: () => void;
  onSaveSelectedSkill: () => void;
  onDeleteSelectedSkill: () => void;
}

export function SkillsCatalogSection({
  styles,
  busyKey,
  skillsMutationsEnabled,
  skillWorkspaceRoot,
  skillCatalog,
  selectedSkillEntry,
  onSelectedSkillNameChange,
  managedRecordBySkill,
  latestGuardBySkill,
  selectedManagedRecord,
  selectedLatestGuard,
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
  onRunSelectedSkillGuard,
  onSaveSelectedSkill,
  onDeleteSelectedSkill,
}: SkillsCatalogSectionProps) {
  const { primaryButtonClass, secondaryButtonClass, mutedCardClass } = styles;
  const { t } = useI18n();

  return (
    <div className="relative" data-testid="settings-skills-catalog">
      {/* List view */}
      <div className="grid gap-2 max-h-[28rem] overflow-y-auto pr-1" data-testid="settings-skills-catalog-list">
        <div className="flex items-center justify-between gap-3 mb-2">
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
            {t("settings.skills.catalog.title")}
          </span>
          <span className="text-xs text-muted-foreground">
            {t("settings.skills.hub.skillsCount", { count: skillCatalog.length })}
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
                data-testid="settings-skills-catalog-item"
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
                      {skill.description || t("settings.skills.catalog.noDescription")}
                    </p>
                  </div>
                  <span
                    className={cn(
                      "shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                      skill.writable
                        ? "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)"
                        : "border-border bg-muted text-muted-foreground",
                    )}
                  >
                    {skill.writable ? t("settings.skills.catalog.writable") : t("settings.skills.catalog.readOnly")}
                  </span>
                </div>
                <div className="flex flex-wrap gap-1.5 text-[10px]">
                  <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                    {t("settings.skills.catalog.filesCount", { count: skill.supporting_files.length })}
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
                          ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                          : "border-border/40 bg-muted text-muted-foreground",
                      )}
                    >
                      {managedSkillStateLabel(managedRecord, t)}
                    </span>
                  ) : null}
                  {latestGuard ? (
                    <span
                      className={cn(
                        "rounded-full border px-2 py-0.5",
                        latestGuard.status === "blocked"
                          ? "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)"
                          : latestGuard.status === "warn"
                            ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                            : "border-border/40 bg-muted text-muted-foreground",
                      )}
                    >
                      {latestGuardStatusLabel(latestGuard, t)}
                    </span>
                  ) : null}
                </div>
              </button>
            );
          })
        ) : (
          <div className={mutedCardClass} data-testid="settings-skills-catalog-empty">{t("settings.skills.catalog.empty")}</div>
        )}
      </div>

      {/* Master-detail overlay */}
      {selectedSkillEntry && !skillDetailLoading ? (
        <div className="absolute inset-2 z-10 overflow-hidden rounded-4xl border border-border/60 bg-background/98 shadow-2xl backdrop-blur-sm">
          <div className="flex items-center gap-3 border-b border-border/60 px-4 py-3">
            <button
              type="button"
              className="text-xs text-muted-foreground transition-colors hover:text-foreground"
              onClick={() => onSelectedSkillNameChange("")}
            >
              {t("settings.skills.catalog.backToCatalog")}
            </button>
            {selectedSkillEntry ? (
              <span
                className={cn(
                  "rounded-full border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
                  selectedSkillEntry.writable
                    ? "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)"
                    : "border-border bg-muted text-muted-foreground",
                )}
              >
                {selectedSkillEntry.writable ? t("settings.skills.catalog.writable") : t("settings.skills.catalog.readOnly")}
              </span>
            ) : null}
          </div>
          <div className="grid max-h-full gap-4 overflow-y-auto px-4 py-4">
            <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
              <strong className="block text-base text-foreground">{selectedSkillEntry.name}</strong>
              <p className="m-0 mt-1 text-sm text-muted-foreground">
                {selectedSkillEntry.description || t("settings.skills.catalog.noDescription")}
              </p>
              <div className="mt-2 flex flex-wrap gap-1.5 text-[10px]">
                {selectedManagedRecord ? (
                  <>
                    <span className="rounded-full border border-border/40 bg-muted px-2 py-0.5 text-muted-foreground">
                      {t("settings.skills.sourcePrefix", { value: selectedManagedRecord.source?.source_id || t("settings.skills.catalog.workspaceLocal") })}
                    </span>
                    <span
                      className={cn(
                        "rounded-full border px-2 py-0.5",
                        selectedManagedRecord.locally_modified || selectedManagedRecord.deleted_locally
                          ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                          : "border-border/40 bg-muted text-muted-foreground",
                      )}
                    >
                      {managedSkillStateLabel(selectedManagedRecord, t)}
                    </span>
                  </>
                ) : null}
                {selectedLatestGuard ? (
                  <span
                    className={cn(
                      "rounded-full border px-2 py-0.5",
                      selectedLatestGuard.status === "blocked"
                        ? "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)"
                        : selectedLatestGuard.status === "warn"
                          ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                          : "border-border/40 bg-muted text-muted-foreground",
                    )}
                  >
                    {latestGuardStatusLabel(selectedLatestGuard, t)} · {t("settings.skills.catalog.violationsCount", { count: selectedLatestGuard.violations.length })}
                  </span>
                ) : null}
              </div>
            </div>

            <div className="text-sm text-muted-foreground">
              <span>{t("settings.skills.catalog.location", { value: selectedSkillEntry.location })}</span>
              <span className="mx-2">·</span>
              <span>{t("settings.skills.catalog.category", { value: selectedSkillEntry.category || "--" })}</span>
              <span className="mx-2">·</span>
              <span>{t("settings.skills.catalog.filesCount", { count: selectedSkillEntry.supporting_files.length })}</span>
            </div>
            {!selectedSkillEntry.writable ? (
              <div className="text-xs text-(--ds-warn)">
                {t("settings.skills.catalog.readOnlyNotice", { root: skillWorkspaceRoot })}
              </div>
            ) : null}

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
                  {mode === "methodology" ? t("settings.skills.editorMode.methodology") : t("settings.skills.editorMode.raw")}
                </button>
              ))}
            </div>

            {editSkillEditorMode === "methodology" ? (
              <div className="grid gap-3">
                <input
                  type="text"
                  placeholder={t("settings.skills.catalog.descriptionPlaceholder")}
                  value={editSkillDescription}
                  onChange={(event) => onEditSkillDescriptionChange(event.target.value)}
                  disabled={!selectedSkillEntry.writable}
                />
                {!editSkillMethodologyMatched ? (
                  <div className="rounded-lg border border-(--ds-warn)/40 bg-(--ds-warn)/12 px-4 py-3 text-sm text-(--ds-warn)">
                    {t("settings.skills.catalog.methodologyRoundtripWarning")}
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
                className="min-h-[24rem] w-full resize-y rounded-lg border border-border/40 bg-background p-3.5 font-mono text-sm leading-relaxed text-foreground"
                value={skillEditorContent}
                onChange={(event) => onSkillEditorContentChange(event.target.value)}
                spellCheck={false}
                readOnly={!selectedSkillEntry.writable}
              />
            )}

            <div className="flex items-center gap-2">
              <button
                className={secondaryButtonClass}
                type="button"
                disabled={busyKey === `skill:guard:skill ${selectedSkillEntry.name}`}
                onClick={onRunSelectedSkillGuard}
              >
                {busyKey === `skill:guard:skill ${selectedSkillEntry.name}`
                  ? t("settings.skills.catalog.scanning")
                  : t("settings.skills.catalog.runGuardCheck")}
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
                {busyKey === `skill:edit:${selectedSkillEntry.name}` ? t("settings.skills.catalog.saving") : t("settings.skills.catalog.saveSkill")}
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
                  ? t("settings.skills.catalog.deleting")
                  : t("settings.skills.catalog.deleteSkill")}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {selectedSkillEntry && skillDetailLoading ? (
        <div className="absolute inset-2 z-10 flex items-center justify-center rounded-4xl border border-border/50 bg-background/85 backdrop-blur-sm">
          <span className="text-sm text-muted-foreground">{t("settings.skills.catalog.loadingSkillSource")}</span>
        </div>
      ) : null}
    </div>
  );
}
