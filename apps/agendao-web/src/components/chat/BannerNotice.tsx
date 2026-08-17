import { AlertTriangleIcon, CheckCircleIcon, InfoIcon, OctagonAlertIcon, XIcon } from "lucide-react";
import { useEffect } from "react";
import { useI18n } from "../../i18n/I18nProvider";
import { useAgendaoStore } from "../../store";
import type { AppNotice } from "../../store/types";

const AUTO_DISMISS_MS = 6000;

const TONE_STYLES: Record<
  AppNotice["tone"],
  { border: string; icon: typeof InfoIcon; label: string }
> = {
  success: {
    border: "border-(--ds-success)/45 bg-(--ds-success)/8 text-foreground",
    icon: CheckCircleIcon,
    label: "text-(--ds-success)",
  },
  error: {
    border: "border-(--ds-error)/50 bg-(--ds-error)/8 text-foreground",
    icon: OctagonAlertIcon,
    label: "text-(--ds-error)",
  },
  warning: {
    border: "border-border bg-card/80 text-foreground",
    icon: AlertTriangleIcon,
    label: "text-(--ds-warning, text-foreground/80)",
  },
  info: {
    border: "border-border bg-card/80 text-foreground",
    icon: InfoIcon,
    label: "text-muted-foreground",
  },
};

function NoticeRow({ notice }: { notice: AppNotice }) {
  const { t } = useI18n();
  const dismissNotice = useAgendaoStore((s) => s.dismissNotice);
  const style = TONE_STYLES[notice.tone];
  const Icon = style.icon;

  // Success/info notices are transient; errors and warnings stay until the
  // user dismisses them so nothing important scrolls away.
  useEffect(() => {
    if (notice.tone !== "success" && notice.tone !== "info") return;
    const timer = window.setTimeout(() => dismissNotice(notice.id), AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [dismissNotice, notice.id, notice.tone]);

  return (
    <div
      className={`flex items-start gap-3 rounded-2xl border px-4 py-3 ${style.border}`}
      data-tone={notice.tone}
      data-testid="banner-notice"
    >
      <Icon className={`mt-0.5 size-4 shrink-0 ${style.label}`} />
      <p className="min-w-0 flex-1 text-sm leading-6 text-current/92">{notice.message}</p>
      <button
        type="button"
        className="roc-banner-dismiss shrink-0"
        aria-label={t("app.dismissStatusMessage")}
        onClick={() => dismissNotice(notice.id)}
      >
        <XIcon className="size-4" />
      </button>
    </div>
  );
}

export function BannerNotice() {
  const notices = useAgendaoStore((s) => s.notices);
  if (notices.length === 0) return null;

  return (
    <div className="mx-auto w-full max-w-[88rem] space-y-2 px-4 pt-3 md:px-5">
      {notices.map((notice) => (
        <NoticeRow key={notice.id} notice={notice} />
      ))}
    </div>
  );
}
