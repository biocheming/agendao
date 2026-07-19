import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { SkillEditorMode } from "../types";
import { SkillMethodologyEditor } from "../../settings/SkillMethodologyEditor";
import type { SkillMethodologyDraft } from "../../settings/SkillMethodologyEditor";

interface SkillsOverviewSectionStyles {
  primaryButtonClass: string;
  editorTextareaClass: string;
}

export interface SkillsOverviewSectionProps {
  workspaceRootPath: string;
  selectedSessionId: string | null;
  skillWorkspaceRoot: string;
  skillsMutationsEnabled: boolean;
  busyKey: string | null;
  styles: SkillsOverviewSectionStyles;
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
}

export function SkillsOverviewSection({
  workspaceRootPath,
  selectedSessionId,
  skillWorkspaceRoot,
  skillsMutationsEnabled,
  busyKey,
  styles,
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
}: SkillsOverviewSectionProps) {
  const { primaryButtonClass, editorTextareaClass } = styles;
  const { t } = useI18n();

  return (
    <div className="grid gap-5" data-testid="settings-skills-overview">
      {/* Workspace info */}
      <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
        <div className="grid gap-2">
          <div>
            <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.skills.overview.workspaceRoot")}</span>
            <span className="ml-2 text-foreground break-all">{workspaceRootPath || "--"}</span>
          </div>
          <div>
            <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.skills.overview.skillRoot")}</span>
            <span className="ml-2 text-foreground break-all">{skillWorkspaceRoot}</span>
          </div>
          <div>
            <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.skills.overview.scope")}</span>
            <span className="ml-2 text-foreground">{selectedSessionId || t("settings.skills.overview.workspaceAuthority")}</span>
          </div>
          <div className="mt-1 text-xs">
            {t("settings.skills.overview.writesNoteA")} <code>/skill/manage</code>{" "}
            {t("settings.skills.overview.writesNoteB", { root: skillWorkspaceRoot })}
            {selectedSessionId
              ? t("settings.skills.overview.catalogNoteSession")
              : t("settings.skills.overview.catalogNoteNoSession")}
          </div>
        </div>
      </div>

      {/* Create Skill */}
      <div>
        <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.skills.overview.createSkill")}</span>

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
              {mode === "methodology" ? t("settings.skills.editorMode.methodology") : t("settings.skills.editorMode.raw")}
            </button>
          ))}
        </div>

        <div className="mt-3 grid gap-3">
          <input
            type="text"
            data-testid="settings-skills-new-name"
            placeholder={t("settings.skills.overview.namePlaceholder")}
            value={newSkillName}
            onChange={(event) => onNewSkillNameChange(event.target.value)}
          />
          <input
            type="text"
            data-testid="settings-skills-new-description"
            placeholder={t("settings.skills.overview.descriptionPlaceholder")}
            value={newSkillDescription}
            onChange={(event) => onNewSkillDescriptionChange(event.target.value)}
          />
          <input
            type="text"
            data-testid="settings-skills-new-category"
            placeholder={t("settings.skills.overview.categoryPlaceholder")}
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
              data-testid="settings-skills-new-body"
              placeholder={t("settings.skills.overview.bodyPlaceholder")}
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
            data-testid="settings-skills-create"
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
              ? t("settings.skills.overview.creating")
              : t("settings.skills.overview.createSkill")}
          </button>
        </div>
      </div>
    </div>
  );
}
