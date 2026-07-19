import type { Dispatch, SetStateAction } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import type { PluginAuthProviderInfo } from "./types";

interface PluginsTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  formLabelClass: string;
  inputClass: string;
  editorTextareaClass: string;
}

export interface PluginsTabProps {
  styles: PluginsTabStyles;
  busyKey: string | null;
  pluginAuthProviders: PluginAuthProviderInfo[];
  pluginConfigs: Record<string, unknown>;
  pluginDrafts: Record<string, string>;
  onPluginDraftsChange: Dispatch<SetStateAction<Record<string, string>>>;
  newPluginKey: string;
  onNewPluginKeyChange: (value: string) => void;
  newPluginDraft: string;
  onNewPluginDraftChange: (value: string) => void;
  onSavePluginConfig: (key: string, raw: string) => void;
  onDeletePluginConfig: (key: string) => void;
}

export function PluginsTab({
  styles,
  busyKey,
  pluginAuthProviders,
  pluginConfigs,
  pluginDrafts,
  onPluginDraftsChange,
  newPluginKey,
  onNewPluginKeyChange,
  newPluginDraft,
  onNewPluginDraftChange,
  onSavePluginConfig,
  onDeletePluginConfig,
}: PluginsTabProps) {
  const {
    primaryButtonClass,
    secondaryButtonClass,
    formLabelClass,
    inputClass,
    editorTextareaClass,
  } = styles;
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-plugins">
      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.plugins.authBridges")}</p>
          <span>{t("settings.plugins.providersCount", { count: pluginAuthProviders.length })}</span>
        </div>
        {pluginAuthProviders.length ? (
          pluginAuthProviders.map((provider) => (
            <div
              key={provider.provider}
              className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4"
              data-testid="settings-plugin-provider"
            >
              <div>
                <strong>{provider.provider}</strong>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {provider.methods.length
                    ? provider.methods
                        .map((method) => method.label || method.type || "method")
                        .join(", ")
                    : t("settings.plugins.noAuthMethods")}
                </p>
              </div>
            </div>
          ))
        ) : (
          <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8" data-testid="settings-plugins-empty">{t("settings.plugins.empty")}</p>
        )}
      </div>

      {Object.entries(pluginConfigs).map(([key]) => (
        <div key={key} className="roc-section">
          <label className={formLabelClass}>{key}</label>
          <textarea
            className={editorTextareaClass}
            value={pluginDrafts[key] ?? ""}
            onChange={(event) => onPluginDraftsChange((current) => ({ ...current, [key]: event.target.value }))}
            spellCheck={false}
          />
          <div className="flex items-center gap-2">
            <button
              className={secondaryButtonClass}
              type="button"
              disabled={busyKey === `plugin:save:${key}`}
              onClick={() => void onSavePluginConfig(key, pluginDrafts[key] ?? "")}
            >
              {t("settings.common.save")}
            </button>
            <button
              className={secondaryButtonClass}
              type="button"
              disabled={busyKey === `plugin:delete:${key}`}
              onClick={() => void onDeletePluginConfig(key)}
            >
              {t("settings.common.delete")}
            </button>
          </div>
        </div>
      ))}

      <div className="roc-section">
        <label htmlFor="settings-new-plugin-key" className={formLabelClass}>{t("settings.plugins.newConfig")}</label>
        <input
          id="settings-new-plugin-key"
          className={inputClass}
          type="text"
          placeholder={t("settings.plugins.namePlaceholder")}
          value={newPluginKey}
          onChange={(event) => onNewPluginKeyChange(event.target.value)}
        />
        <textarea
          className={editorTextareaClass}
          value={newPluginDraft}
          onChange={(event) => onNewPluginDraftChange(event.target.value)}
          spellCheck={false}
        />
        <button
          className={primaryButtonClass}
          type="button"
          disabled={!newPluginKey.trim() || busyKey === `plugin:save:${newPluginKey.trim()}`}
          onClick={() => void onSavePluginConfig(newPluginKey.trim(), newPluginDraft)}
        >
          {t("settings.plugins.addConfig")}
        </button>
      </div>
    </div>
  );
}
