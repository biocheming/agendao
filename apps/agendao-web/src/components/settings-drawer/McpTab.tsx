import type { Dispatch, SetStateAction } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import type { McpStatusInfo } from "./types";

interface McpTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  formLabelClass: string;
  inputClass: string;
  editorTextareaClass: string;
}

export interface McpTabProps {
  styles: McpTabStyles;
  busyKey: string | null;
  mcpStatus: Record<string, McpStatusInfo>;
  mcpConfigs: Record<string, unknown>;
  mcpDrafts: Record<string, string>;
  onMcpDraftsChange: Dispatch<SetStateAction<Record<string, string>>>;
  newMcpKey: string;
  onNewMcpKeyChange: (value: string) => void;
  newMcpDraft: string;
  onNewMcpDraftChange: (value: string) => void;
  onSaveMcpConfig: (key: string, raw: string) => void;
  onDeleteMcpConfig: (key: string) => void;
  onRunMcpAction: (name: string, action: "connect" | "disconnect" | "restart") => void;
}

export function McpTab({
  styles,
  busyKey,
  mcpStatus,
  mcpConfigs,
  mcpDrafts,
  onMcpDraftsChange,
  newMcpKey,
  onNewMcpKeyChange,
  newMcpDraft,
  onNewMcpDraftChange,
  onSaveMcpConfig,
  onDeleteMcpConfig,
  onRunMcpAction,
}: McpTabProps) {
  const {
    primaryButtonClass,
    secondaryButtonClass,
    formLabelClass,
    inputClass,
    editorTextareaClass,
  } = styles;
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-mcp">
      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.mcp.runtimeStatus")}</p>
          <span>{t("settings.mcp.serversCount", { count: Object.keys(mcpStatus).length })}</span>
        </div>
        {Object.values(mcpStatus).length ? (
          Object.values(mcpStatus).map((server) => (
            <div
              key={server.name}
              className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4"
              data-testid="settings-mcp-server"
            >
              <div>
                <strong>{server.name}</strong>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {t("settings.mcp.serverLine", { status: server.status, tools: server.tools, resources: server.resources })}
                </p>
                {server.error ? <p className="text-sm text-muted-foreground leading-relaxed">{server.error}</p> : null}
              </div>
              <div className="flex items-center gap-2">
                <button
                  className={secondaryButtonClass}
                  type="button"
                  disabled={busyKey === `mcp:connect:${server.name}`}
                  onClick={() => void onRunMcpAction(server.name, "connect")}
                >
                  {t("settings.mcp.connect")}
                </button>
                <button
                  className={secondaryButtonClass}
                  type="button"
                  disabled={busyKey === `mcp:disconnect:${server.name}`}
                  onClick={() => void onRunMcpAction(server.name, "disconnect")}
                >
                  {t("settings.mcp.disconnect")}
                </button>
                <button
                  className={secondaryButtonClass}
                  type="button"
                  disabled={busyKey === `mcp:restart:${server.name}`}
                  onClick={() => void onRunMcpAction(server.name, "restart")}
                >
                  {t("settings.mcp.restart")}
                </button>
              </div>
            </div>
          ))
        ) : (
          <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8" data-testid="settings-mcp-empty">{t("settings.mcp.empty")}</p>
        )}
      </div>

      {Object.entries(mcpConfigs).map(([key]) => (
        <div key={key} className="roc-section">
          <label className={formLabelClass}>{key}</label>
          <textarea
            className={editorTextareaClass}
            value={mcpDrafts[key] ?? ""}
            onChange={(event) => onMcpDraftsChange((current) => ({ ...current, [key]: event.target.value }))}
            spellCheck={false}
          />
          <div className="flex items-center gap-2">
            <button
              className={secondaryButtonClass}
              type="button"
              disabled={busyKey === `mcp:save:${key}`}
              onClick={() => void onSaveMcpConfig(key, mcpDrafts[key] ?? "")}
            >
              {t("settings.common.save")}
            </button>
            <button
              className={secondaryButtonClass}
              type="button"
              disabled={busyKey === `mcp:delete:${key}`}
              onClick={() => void onDeleteMcpConfig(key)}
            >
              {t("settings.common.delete")}
            </button>
          </div>
        </div>
      ))}

      <div className="roc-section">
        <label htmlFor="settings-new-mcp-key" className={formLabelClass}>{t("settings.mcp.newConfig")}</label>
        <input
          id="settings-new-mcp-key"
          className={inputClass}
          type="text"
          placeholder={t("settings.mcp.namePlaceholder")}
          value={newMcpKey}
          onChange={(event) => onNewMcpKeyChange(event.target.value)}
        />
        <textarea
          className={editorTextareaClass}
          value={newMcpDraft}
          onChange={(event) => onNewMcpDraftChange(event.target.value)}
          spellCheck={false}
        />
        <button
          className={primaryButtonClass}
          type="button"
          disabled={!newMcpKey.trim() || busyKey === `mcp:save:${newMcpKey.trim()}`}
          onClick={() => void onSaveMcpConfig(newMcpKey.trim(), newMcpDraft)}
        >
          {t("settings.mcp.addConfig")}
        </button>
      </div>
    </div>
  );
}
