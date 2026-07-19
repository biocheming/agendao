import { AlertTriangleIcon, XIcon } from "lucide-react";
import { useI18n } from "../../i18n/I18nProvider";
import { useAgendaoStore } from "../../store";

export function BannerNotice() {
  const { t } = useI18n();
  const banner = useAgendaoStore((s) => s.banner);
  const setBanner = useAgendaoStore((s) => s.setBanner);

  if (!banner) return null;

  return (
    <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
      <div className="roc-banner flex items-start gap-3" data-tone="warning">
        <div className="roc-status-orb mt-0.5 shrink-0" data-tone="loading">
          <AlertTriangleIcon className="size-4" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="roc-section-label">{t("app.attention")}</div>
          <p className="mt-1 text-sm leading-6 text-current/92">{banner}</p>
        </div>
        <button
          type="button"
          className="roc-banner-dismiss shrink-0"
          aria-label={t("app.dismissStatusMessage")}
          onClick={() => setBanner(null)}
        >
          <XIcon className="size-4" />
        </button>
      </div>
    </div>
  );
}
