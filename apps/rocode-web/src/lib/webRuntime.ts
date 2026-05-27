export type ThemeId = "daylight" | "sunset" | "cobalt";

export interface ExecutionMode {
  id: string;
  name: string;
  kind: string;
  hidden?: boolean;
  mode?: string;
}

export const THEMES: Array<{ id: ThemeId; label: string }> = [
  { id: "daylight", label: "Daylight" },
  { id: "sunset", label: "Sunset" },
  { id: "cobalt", label: "Cobalt" },
];

export const DEFAULT_WEB_MODE = "preset:auto";

export function applyPreferences(config: Record<string, unknown>) {
  const ui = (config.uiPreferences ?? config.ui_preferences ?? {}) as Record<string, unknown>;
  return {
    theme: String(ui.webTheme ?? ui.web_theme ?? "daylight") as ThemeId,
    mode: String(ui.webMode ?? ui.web_mode ?? ""),
    model: String(ui.webModel ?? ui.web_model ?? ""),
    showThinking: Boolean(ui.showThinking ?? ui.show_thinking ?? true),
  };
}
