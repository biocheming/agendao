import { cn } from "@/lib/utils";
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

const methodologyFields: MethodologyFieldDescriptor[] = [
  {
    key: "whenToUse",
    label: "When To Use",
    hint: "One trigger per line.",
    placeholder: "Use when provider metadata is stale",
  },
  {
    key: "whenNotToUse",
    label: "When Not To Use",
    hint: "One boundary per line.",
    placeholder: "Do not use for one-off API key edits",
  },
  {
    key: "prerequisites",
    label: "Prerequisites",
    hint: "One requirement per line.",
    placeholder: "Provider auth already configured",
  },
  {
    key: "coreSteps",
    label: "Core Steps",
    hint: "One step per line: Title | Action | Outcome(optional)",
    placeholder: "Refresh catalog | Run /providers refresh | Latest ids appear",
    minHeightClass: "min-h-[9rem]",
  },
  {
    key: "successCriteria",
    label: "Success Criteria",
    hint: "One checklist item per line.",
    placeholder: "Expected model ids appear in the catalog",
  },
  {
    key: "validation",
    label: "Validation",
    hint: "One verification item per line.",
    placeholder: "Reload the provider list and compare ids",
  },
  {
    key: "pitfalls",
    label: "Boundaries / Pitfalls",
    hint: "One pitfall per line.",
    placeholder: "Do not overwrite workspace-local overrides",
  },
  {
    key: "references",
    label: "References",
    hint: "One reference per line: path | label",
    placeholder: "docs/provider-index.md | Source contract",
  },
];

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
  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1.15fr)_minmax(19rem,0.85fr)]">
      <div className="grid gap-3">
        {methodologyFields.map((field) => (
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
              <p className="m-0 text-sm font-semibold text-foreground">Preview</p>
              <p className="m-0 mt-1 text-xs text-muted-foreground">
                This renders the structured methodology into the final skill body.
              </p>
            </div>
            {previewError ? (
              <span className="rounded-full border border-amber-300 bg-amber-50 px-2.5 py-1 text-xs font-semibold text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200">
                incomplete
              </span>
            ) : (
              <span className="rounded-full border border-border bg-card/80 px-2.5 py-1 text-xs font-semibold text-muted-foreground">
                rendered
              </span>
            )}
          </div>
        </div>
        <pre className="min-h-[34rem] overflow-auto rounded-xl border border-border/45 bg-background/78 p-4 text-sm leading-relaxed text-foreground whitespace-pre-wrap">
          {previewError
            ? `Methodology preview is incomplete.\n\n${previewError}`
            : previewBody || "Preview will appear here once the methodology becomes valid."}
        </pre>
      </div>
    </div>
  );
}
