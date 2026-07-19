import { useI18n } from "@/i18n/I18nProvider";
import type { FormatterStatus, LspStatus } from "./types";

export interface LspTabProps {
  lspStatus: LspStatus | null;
  formatterStatus: FormatterStatus | null;
}

export function LspTab({ lspStatus, formatterStatus }: LspTabProps) {
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-lsp">
      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.lsp.servers")}</p>
          <span>{lspStatus?.servers.length ?? 0}</span>
        </div>
        {lspStatus?.servers.length ? (
          lspStatus.servers.map((server) => (
            <div
              key={server}
              className="rounded-lg border border-border/40 bg-card p-4 flex items-center justify-between gap-4"
              data-testid="settings-lsp-server"
            >
              <strong>{server}</strong>
            </div>
          ))
        ) : (
          <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8" data-testid="settings-lsp-empty">{t("settings.lsp.noServers")}</p>
        )}
      </div>

      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.lsp.formatters")}</p>
          <span>{formatterStatus?.formatters.length ?? 0}</span>
        </div>
        {formatterStatus?.formatters.length ? (
          formatterStatus.formatters.map((formatter) => (
            <div key={formatter} className="rounded-lg border border-border/40 bg-card p-4 flex items-center justify-between gap-4">
              <strong>{formatter}</strong>
            </div>
          ))
        ) : (
          <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">{t("settings.lsp.noFormatters")}</p>
        )}
      </div>
    </div>
  );
}
