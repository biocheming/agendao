export type ThemeId = "daylight" | "sunset" | "cobalt";

export interface ExecutionMode {
  id: string;
  name: string;
  kind: "agent" | "scheduler";
  hidden?: boolean;
  mode?: string;
}

export type SchedulerTemplateId =
  | "direct"
  | "plan"
  | "coordinate"
  | "verify"
  | "autoresearch";

export type SchedulerChoiceRecord =
  | { kind: "auto" }
  | { kind: "template"; template: SchedulerTemplateId };

const SCHEDULER_TEMPLATE_IDS = new Set<SchedulerTemplateId>([
  "direct",
  "plan",
  "coordinate",
  "verify",
  "autoresearch",
]);

export function schedulerChoiceFromId(id: string): SchedulerChoiceRecord {
  const normalized = id.trim().toLowerCase();
  if (normalized === "auto") return { kind: "auto" };
  if (SCHEDULER_TEMPLATE_IDS.has(normalized as SchedulerTemplateId)) {
    return { kind: "template", template: normalized as SchedulerTemplateId };
  }
  throw new Error(`Unknown scheduler mode: ${id}`);
}

export const THEMES: Array<{ id: ThemeId; label: string }> = [
  { id: "daylight", label: "Daylight" },
  { id: "sunset", label: "Sunset" },
  { id: "cobalt", label: "Cobalt" },
];

export const DEFAULT_WEB_MODE = "scheduler:auto";

export function applyPreferences(config: Record<string, unknown>) {
  const ui = (config.uiPreferences ?? config.ui_preferences ?? {}) as Record<string, unknown>;
  return {
    theme: String(ui.webTheme ?? ui.web_theme ?? "daylight") as ThemeId,
    mode: String(ui.webMode ?? ui.web_mode ?? ""),
    model: String(ui.webModel ?? ui.web_model ?? ""),
    showThinking: Boolean(ui.showThinking ?? ui.show_thinking ?? true),
  };
}
