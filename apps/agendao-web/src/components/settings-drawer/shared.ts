import type { ConfigPolicyValidationItemRecord } from "@/lib/configPolicy";
import type { ManagedModelOverrideInfoRecord } from "@/lib/provider";
import type { ModelOverrideDraft } from "./types";

export type SettingsTabId =
  | "general"
  | "memory"
  | "providers"
  | "validation"
  | "skills"
  | "mcp"
  | "plugins"
  | "lsp";

export const SETTINGS_TABS: Array<{ id: SettingsTabId; label: string }> = [
  { id: "general", label: "General" },
  { id: "memory", label: "Memory" },
  { id: "providers", label: "Providers" },
  { id: "validation", label: "Validation" },
  { id: "skills", label: "Skills" },
  { id: "mcp", label: "MCP" },
  { id: "plugins", label: "Plugins" },
  { id: "lsp", label: "LSP" },
];

export const SETTINGS_DRAWER_STYLES = {
  secondaryButtonClass: "roc-action roc-action-pill px-4 text-foreground text-sm cursor-pointer",
  primaryButtonClass:
    "roc-action roc-action-pill border-foreground bg-foreground px-5 text-sm font-semibold text-background disabled:cursor-not-allowed disabled:opacity-60",
  summaryCardClass: "rounded-lg border border-border/30 bg-card p-4 grid gap-1",
  sectionCardClass: "grid gap-4 rounded-lg bg-muted/30 p-5",
  mutedCardClass:
    "rounded-lg bg-muted/40 px-4 py-3 text-sm leading-relaxed text-muted-foreground",
  insetCardClass: "rounded-lg border border-border/35 bg-card/80 p-4",
  disclosureCardClass: "rounded-lg border border-border/35 bg-card/80",
  editorTextareaClass: "roc-form-textarea min-h-40 font-mono",
  formFieldClass: "roc-form-field",
  formLabelClass: "roc-form-label",
  formHintClass: "roc-form-hint",
  inputClass: "roc-form-control",
  selectClass: "roc-form-select",
  checkboxRowClass: "roc-form-checkbox-row",
  checkboxClass: "roc-form-checkbox",
} as const;

export function isolatedWorkspaceNotice(
  tab: SettingsTabId,
  t: (key: string) => string,
): string | null {
  switch (tab) {
    case "general":
      return t("settings.isolated.general");
    case "providers":
      return t("settings.isolated.providers");
    case "memory":
      return t("settings.isolated.memory");
    case "validation":
      return t("settings.isolated.validation");
    case "skills":
      return t("settings.isolated.skills");
    case "mcp":
      return t("settings.isolated.mcp");
    case "plugins":
      return t("settings.isolated.plugins");
    default:
      return null;
  }
}

export function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error ?? "Unknown error");
}

export function arrayOrEmpty<T>(value: T[] | null | undefined): T[] {
  return Array.isArray(value) ? value : [];
}

export function stringifyJson(value: unknown) {
  return JSON.stringify(value ?? {}, null, 2);
}

export function objectRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  return value as Record<string, unknown>;
}

export function parseObjectJson(
  label: string,
  raw: string,
  t?: (key: string, params?: Record<string, string | number>) => string,
) {
  const trimmed = raw.trim();
  if (!trimmed) {
    throw new Error(t ? t("settings.feedback.jsonEmpty", { label }) : `${label} JSON cannot be empty`);
  }
  const parsed = JSON.parse(trimmed) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(t ? t("settings.feedback.jsonNotObject", { label }) : `${label} JSON must be an object`);
  }
  return parsed as Record<string, unknown>;
}

export function emptyModelOverrideDraft(providerId = ""): ModelOverrideDraft {
  return {
    providerId,
    modelKey: "",
    modelId: "",
    name: "",
    baseUrl: "",
    family: "",
    status: "",
    releaseDate: "",
    reasoning: false,
    reasoningEffort: "",
    toolCall: false,
    attachment: false,
    temperature: false,
    experimental: false,
  };
}

export function modelOverrideDraftFromRecord(
  providerId: string,
  record: ManagedModelOverrideInfoRecord,
): ModelOverrideDraft {
  return {
    providerId,
    modelKey: record.key,
    modelId: record.model ?? "",
    name: record.name ?? "",
    baseUrl: record.base_url ?? "",
    family: record.family ?? "",
    status: record.status ?? "",
    releaseDate: record.release_date ?? "",
    reasoning: Boolean(record.reasoning),
    reasoningEffort: record.reasoning_effort ?? "",
    toolCall: Boolean(record.tool_call),
    attachment: Boolean(record.attachment),
    temperature: Boolean(record.temperature),
    experimental: Boolean(record.experimental),
  };
}

export function validationJumpTarget(
  item: ConfigPolicyValidationItemRecord,
):
  | {
      tab: Extract<SettingsTabId, "providers">;
      label: string;
      providerId?: string;
    }
  | null {
  if (item.owner === "provider_profile") {
    return {
      tab: "providers",
      label: "settings.validation.openProviders",
      providerId: item.scope.subject_id ?? undefined,
    };
  }
  return null;
}
