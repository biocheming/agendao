import { useI18n } from "@/i18n/I18nProvider";
import type { SchedulerConfigResponse } from "./types";

interface SchedulerTabStyles {
  primaryButtonClass: string;
  formLabelClass: string;
  formHintClass: string;
  inputClass: string;
  editorTextareaClass: string;
}

export interface SchedulerTabProps {
  styles: SchedulerTabStyles;
  busyKey: string | null;
  schedulerConfig: SchedulerConfigResponse | null;
  schedulerPathDraft: string;
  onSchedulerPathDraftChange: (value: string) => void;
  schedulerContentDraft: string;
  onSchedulerContentDraftChange: (value: string) => void;
  onSaveScheduler: () => void;
}

export function SchedulerTab({
  styles,
  busyKey,
  schedulerConfig,
  schedulerPathDraft,
  onSchedulerPathDraftChange,
  schedulerContentDraft,
  onSchedulerContentDraftChange,
  onSaveScheduler,
}: SchedulerTabProps) {
  const {
    primaryButtonClass,
    formLabelClass,
    formHintClass,
    inputClass,
    editorTextareaClass,
  } = styles;
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-scheduler">
      <div className="roc-section">
        <label htmlFor="settings-scheduler-path" className={formLabelClass}>{t("settings.scheduler.configPath")}</label>
        <input
          id="settings-scheduler-path"
          className={inputClass}
          type="text"
          value={schedulerPathDraft}
          onChange={(event) => onSchedulerPathDraftChange(event.target.value)}
        />
        <div className={formHintClass}>
          {t("settings.scheduler.resolved", { path: schedulerConfig?.resolved_path || "--" })} · {schedulerConfig?.exists ? t("settings.scheduler.fileExists") : t("settings.scheduler.newFile")}
        </div>
      </div>

      <div className="roc-section">
        <label htmlFor="settings-scheduler-content" className={formLabelClass}>{t("settings.scheduler.config")}</label>
        <textarea
          id="settings-scheduler-content"
          className={editorTextareaClass}
          value={schedulerContentDraft}
          onChange={(event) => onSchedulerContentDraftChange(event.target.value)}
          spellCheck={false}
        />
        <div className="flex items-center gap-2">
          <button
            className={primaryButtonClass}
            type="button"
            data-testid="settings-scheduler-save"
            disabled={busyKey === "scheduler:save"}
            onClick={() => void onSaveScheduler()}
          >
            {busyKey === "scheduler:save" ? t("settings.scheduler.saving") : t("settings.scheduler.saveConfig")}
          </button>
        </div>
        {schedulerConfig?.parse_error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">{schedulerConfig.parse_error}</div>
        ) : null}
      </div>

      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.scheduler.profiles")}</p>
          <span>{t("settings.scheduler.defaultProfile", { value: schedulerConfig?.default_profile || "--" })}</span>
        </div>
        {schedulerConfig?.profiles.length ? (
          schedulerConfig.profiles.map((profile) => (
            <div key={profile.key} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4">
              <div>
                <strong>{profile.key}</strong>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {profile.orchestrator || t("settings.scheduler.noOrchestrator")}
                  {profile.description ? ` · ${profile.description}` : ""}
                </p>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {profile.stages.length ? profile.stages.join(" -> ") : t("settings.scheduler.noStages")}
                </p>
              </div>
            </div>
          ))
        ) : (
          <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">{t("settings.scheduler.emptyProfiles")}</p>
        )}
      </div>
    </div>
  );
}
