import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  AppConfigSnapshot,
  ModeOption,
  ModelOption,
  SchedulerConfigResponse,
  ThemeOption,
} from "./types";

interface GeneralTabStyles {
  summaryCardClass: string;
  formFieldClass: string;
  formLabelClass: string;
  selectClass: string;
  checkboxRowClass: string;
  checkboxClass: string;
}

export interface GeneralTabProps {
  theme: string;
  themes: ThemeOption[];
  onThemeChange: (themeId: string) => void;
  modeOptions: ModeOption[];
  selectedMode: string;
  onModeChange: (mode: string) => void;
  modelOptions: ModelOption[];
  selectedModel: string;
  onModelChange: (model: string) => void;
  showThinking: boolean;
  onShowThinkingChange: (value: boolean) => void;
  workspaceMode: "shared" | "isolated" | null;
  workspaceRootPath: string;
  workspaceConfigDir?: string | null;
  providerSummary: string;
  schedulerConfig: SchedulerConfigResponse | null;
  configSnapshot: AppConfigSnapshot | null;
  mcpConfigs: Record<string, unknown>;
  pluginConfigs: Record<string, unknown>;
  styles: GeneralTabStyles;
}

export function GeneralTab({
  theme,
  themes,
  onThemeChange,
  modeOptions,
  selectedMode,
  onModeChange,
  modelOptions,
  selectedModel,
  onModelChange,
  showThinking,
  onShowThinkingChange,
  workspaceMode,
  workspaceRootPath,
  workspaceConfigDir,
  providerSummary,
  schedulerConfig,
  configSnapshot,
  mcpConfigs,
  pluginConfigs,
  styles,
}: GeneralTabProps) {
  const {
    summaryCardClass,
    formFieldClass,
    formLabelClass,
    selectClass,
    checkboxRowClass,
    checkboxClass,
  } = styles;
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-general">
      <div className="roc-section">
        <div className={formFieldClass}>
          <label className={formLabelClass}>{t("settings.general.theme")}</label>
        </div>
        <div className="flex flex-wrap gap-2">
          {themes.map((item) => (
            <button
              key={item.id}
              type="button"
              data-testid={`settings-theme-${item.id}`}
              data-active={theme === item.id ? "true" : "false"}
              className={
                theme === item.id
                  ? "roc-action roc-action-pill border-foreground bg-foreground px-4 text-sm font-medium text-background"
                  : "roc-action roc-action-pill px-4 text-sm text-muted-foreground"
              }
              onClick={() => onThemeChange(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>

      <div className="roc-section">
        <label htmlFor="settings-mode-select" className={formLabelClass}>{t("settings.general.executionMode")}</label>
        <select
          id="settings-mode-select"
          className={selectClass}
          value={selectedMode}
          onChange={(event) => onModeChange(event.target.value)}
        >
          <option value="">{t("settings.general.autoOption")}</option>
          {modeOptions.map((mode) => (
            <option key={mode.key} value={mode.key}>
              {mode.label}
            </option>
          ))}
        </select>
      </div>

      <div className="roc-section">
        <label htmlFor="settings-model-select" className={formLabelClass}>{t("settings.general.model")}</label>
        <select
          id="settings-model-select"
          className={selectClass}
          value={selectedModel}
          onChange={(event) => onModelChange(event.target.value)}
        >
          <option value="">{t("settings.general.autoOption")}</option>
          {modelOptions.map((model) => (
            <option key={model.key} value={model.key}>
              {model.label}
            </option>
          ))}
        </select>
      </div>

      <div className="roc-section">
        <label className={checkboxRowClass}>
          <input
            className={checkboxClass}
            type="checkbox"
            data-testid="settings-show-thinking"
            checked={showThinking}
            onChange={(event) => onShowThinkingChange(event.target.checked)}
          />
          {t("settings.general.showThinking")}
        </label>
      </div>

      <div className="grid gap-3">
        <label>{t("settings.general.workspaceAuthority")}</label>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className={summaryCardClass}>
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.workspaceMode")}</span>
            <strong>{workspaceMode === "isolated" ? t("settings.general.workspaceModeIsolated") : t("settings.general.workspaceModeShared")}</strong>
          </div>
          <div className={summaryCardClass}>
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.workspaceRoot")}</span>
            <strong className="break-all text-sm">{workspaceRootPath || "--"}</strong>
          </div>
        </div>
        {workspaceConfigDir ? (
          <p className="m-0 text-xs leading-relaxed text-muted-foreground">
            {t("settings.general.isolatedConfigDir")} <code>{workspaceConfigDir}</code>
          </p>
        ) : null}
        <div
          className={cn(
            "rounded-lg px-4 py-3 text-sm leading-relaxed",
            workspaceMode === "isolated"
              ? "bg-(--ds-warn)/12 text-(--ds-warn)"
              : "bg-muted/40 text-muted-foreground",
          )}
        >
          {workspaceMode === "isolated"
            ? t("settings.general.isolatedModeNotice")
            : t("settings.general.sharedModeNotice")}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.providers")}</span>
          <strong>{providerSummary}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.schedulerPath")}</span>
          <strong>{schedulerConfig?.raw_path || configSnapshot?.schedulerPath || "--"}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.mcpServers")}</span>
          <strong>{Object.keys(mcpConfigs).length}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.general.plugins")}</span>
          <strong>{Object.keys(pluginConfigs).length}</strong>
        </div>
      </div>
    </div>
  );
}
