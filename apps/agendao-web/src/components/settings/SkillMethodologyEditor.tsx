import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { SkillMethodologyTemplateRecord } from "@/lib/skill";

export interface SkillMethodologyDraft {
  whenToUse: string;
  whenNotToUse: string;
  prerequisites: string;
  coreSteps: string;
  successCriteria: string;
  validation: string;
  pitfalls: string;
  references: string;
}

interface SkillMethodologyEditorProps {
  draft: SkillMethodologyDraft;
  onChange: (next: SkillMethodologyDraft) => void;
  previewBody: string;
  previewError?: string | null;
  disabled?: boolean;
}

interface MethodologyFieldDescriptor {
  key: keyof SkillMethodologyDraft;
  label: string;
  hint: string;
  placeholder: string;
  minHeightClass?: string;
}

function methodologyFields(
  t: (key: string) => string,
): MethodologyFieldDescriptor[] {
  return [
    {
      key: "whenToUse",
      label: t("settings.skills.methodology.whenToUse.label"),
      hint: t("settings.skills.methodology.whenToUse.hint"),
      placeholder: t("settings.skills.methodology.whenToUse.placeholder"),
    },
    {
      key: "whenNotToUse",
      label: t("settings.skills.methodology.whenNotToUse.label"),
      hint: t("settings.skills.methodology.whenNotToUse.hint"),
      placeholder: t("settings.skills.methodology.whenNotToUse.placeholder"),
    },
    {
      key: "prerequisites",
      label: t("settings.skills.methodology.prerequisites.label"),
      hint: t("settings.skills.methodology.prerequisites.hint"),
      placeholder: t("settings.skills.methodology.prerequisites.placeholder"),
    },
    {
      key: "coreSteps",
      label: t("settings.skills.methodology.coreSteps.label"),
      hint: t("settings.skills.methodology.coreSteps.hint"),
      placeholder: t("settings.skills.methodology.coreSteps.placeholder"),
      minHeightClass: "min-h-[9rem]",
    },
    {
      key: "successCriteria",
      label: t("settings.skills.methodology.successCriteria.label"),
      hint: t("settings.skills.methodology.successCriteria.hint"),
      placeholder: t("settings.skills.methodology.successCriteria.placeholder"),
    },
    {
      key: "validation",
      label: t("settings.skills.methodology.validation.label"),
      hint: t("settings.skills.methodology.validation.hint"),
      placeholder: t("settings.skills.methodology.validation.placeholder"),
    },
    {
      key: "pitfalls",
      label: t("settings.skills.methodology.pitfalls.label"),
      hint: t("settings.skills.methodology.pitfalls.hint"),
      placeholder: t("settings.skills.methodology.pitfalls.placeholder"),
    },
    {
      key: "references",
      label: t("settings.skills.methodology.references.label"),
      hint: t("settings.skills.methodology.references.hint"),
      placeholder: t("settings.skills.methodology.references.placeholder"),
    },
  ];
}

export function emptySkillMethodologyDraft(): SkillMethodologyDraft {
  return {
    whenToUse: "",
    whenNotToUse: "",
    prerequisites: "",
    coreSteps: "",
    successCriteria: "",
    validation: "",
    pitfalls: "",
    references: "",
  };
}

export function methodologyDraftFromTemplate(
  template: SkillMethodologyTemplateRecord,
): SkillMethodologyDraft {
  return {
    whenToUse: template.when_to_use.join("\n"),
    whenNotToUse: template.when_not_to_use.join("\n"),
    prerequisites: template.prerequisites.join("\n"),
    coreSteps: template.core_steps
      .map((step) =>
        [step.title, step.action, step.outcome?.trim() || ""]
          .filter((value, index) => index < 2 || value)
          .join(" | "),
      )
      .join("\n"),
    successCriteria: template.success_criteria.join("\n"),
    validation: template.validation.join("\n"),
    pitfalls: template.pitfalls.join("\n"),
    references: template.references
      .map((reference) => `${reference.path} | ${reference.label}`)
      .join("\n"),
  };
}

export function buildMethodologyTemplateFromDraft(
  draft: SkillMethodologyDraft,
): SkillMethodologyTemplateRecord {
  return {
    when_to_use: splitNonEmptyLines(draft.whenToUse),
    when_not_to_use: splitNonEmptyLines(draft.whenNotToUse),
    prerequisites: splitNonEmptyLines(draft.prerequisites),
    core_steps: splitNonEmptyLines(draft.coreSteps).map((line) => {
      const [title = "", action = "", outcome = ""] = line.split("|").map((value) => value.trim());
      return {
        title,
        action,
        outcome: outcome || undefined,
      };
    }),
    success_criteria: splitNonEmptyLines(draft.successCriteria),
    validation: splitNonEmptyLines(draft.validation),
    pitfalls: splitNonEmptyLines(draft.pitfalls),
    references: splitNonEmptyLines(draft.references).map((line) => {
      const [path = "", label = ""] = line.split("|").map((value) => value.trim());
      return { path, label };
    }),
  };
}

function splitNonEmptyLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

export function SkillMethodologyEditor({
  draft,
  onChange,
  previewBody,
  previewError,
  disabled = false,
}: SkillMethodologyEditorProps) {
  const { t } = useI18n();
  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1.15fr)_minmax(19rem,0.85fr)]">
      <div className="grid gap-3">
        {methodologyFields(t).map((field) => (
          <label
            key={field.key}
            className="grid gap-1.5 rounded-xl border border-border/40 bg-background/55 p-3"
          >
            <span className="text-sm font-semibold text-foreground">{field.label}</span>
            <span className="text-xs text-muted-foreground">{field.hint}</span>
            <textarea
              className={cn(
                "w-full resize-y rounded-lg border border-border/45 bg-background/82 px-3 py-2 text-sm text-foreground leading-relaxed outline-none focus:border-primary/55",
                field.minHeightClass ?? "min-h-[5.5rem]",
              )}
              value={draft[field.key]}
              onChange={(event) =>
                onChange({
                  ...draft,
                  [field.key]: event.target.value,
                })
              }
              placeholder={field.placeholder}
              spellCheck={false}
              disabled={disabled}
            />
          </label>
        ))}
      </div>

      <div className="grid gap-3">
        <div className="rounded-xl border border-border/40 bg-background/55 p-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="m-0 text-sm font-semibold text-foreground">{t("settings.skills.methodology.preview")}</p>
              <p className="m-0 mt-1 text-xs text-muted-foreground">
                {t("settings.skills.methodology.previewNote")}
              </p>
            </div>
            {previewError ? (
              <span className="rounded-full border border-(--ds-warn)/40 bg-(--ds-warn)/12 px-2.5 py-1 text-xs font-semibold text-(--ds-warn)">
                {t("settings.skills.methodology.incomplete")}
              </span>
            ) : (
              <span className="rounded-full border border-border bg-card/80 px-2.5 py-1 text-xs font-semibold text-muted-foreground">
                {t("settings.skills.methodology.rendered")}
              </span>
            )}
          </div>
        </div>
        <pre className="min-h-[34rem] overflow-auto rounded-xl border border-border/45 bg-background/78 p-4 text-sm leading-relaxed text-foreground whitespace-pre-wrap">
          {previewError
            ? t("settings.skills.methodology.previewIncomplete", { error: previewError })
            : previewBody || t("settings.skills.methodology.previewPlaceholder")}
        </pre>
      </div>
    </div>
  );
}
