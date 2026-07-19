import { Suspense } from "react";
import type { ReactNode } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { SETTINGS_DRAWER_STYLES, SETTINGS_TABS } from "./shared";
import type { SettingsTabId } from "./shared";

interface SettingsDrawerHeaderProps {
  refreshing: boolean;
  onRefresh: () => void;
  onClose: () => void;
}

export function SettingsDrawerHeader({
  refreshing,
  onRefresh,
  onClose,
}: SettingsDrawerHeaderProps) {
  const { t } = useI18n();
  return (
    <header className="flex items-start justify-between gap-4 px-5 pb-5 pt-6 sm:px-7 sm:pb-6 sm:pt-7">
      <div>
        <p className="m-0 mb-1.5 text-xs tracking-widest uppercase text-(--ds-accent) font-bold">{t("app.settings")}</p>
        <h2 className="text-xl font-semibold tracking-tight">{t("settings.drawer.subtitle")}</h2>
      </div>
      <div className="flex items-center gap-2">
        <button
          className={SETTINGS_DRAWER_STYLES.secondaryButtonClass}
          type="button"
          data-testid="settings-refresh"
          onClick={onRefresh}
        >
          {refreshing ? t("settings.drawer.refreshing") : t("settings.drawer.refresh")}
        </button>
        <button className={SETTINGS_DRAWER_STYLES.secondaryButtonClass} type="button" data-testid="settings-close" onClick={onClose}>
          {t("settings.drawer.close")}
        </button>
      </div>
    </header>
  );
}

interface SettingsDrawerNavProps {
  activeTab: SettingsTabId;
  onTabChange: (tab: SettingsTabId) => void;
}

export function SettingsDrawerNav({ activeTab, onTabChange }: SettingsDrawerNavProps) {
  const { t } = useI18n();
  return (
    <nav className="border-b border-border/60 px-5 sm:px-7">
      <div className="flex flex-wrap gap-4 sm:gap-6">
        {SETTINGS_TABS.map((tab) => (
          <button
            key={tab.id}
            type="button"
            data-testid={`settings-tab-${tab.id}`}
            data-active={activeTab === tab.id ? "true" : "false"}
            className={cn(
              "relative py-2.5 text-sm font-medium cursor-pointer transition-colors",
              activeTab === tab.id ? "text-foreground" : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => onTabChange(tab.id)}
          >
            {t(`settings.tabs.${tab.id}`)}
            {activeTab === tab.id && (
              <span className="absolute bottom-0 left-0 right-0 h-0.5 bg-foreground rounded-t-sm" />
            )}
          </button>
        ))}
      </div>
    </nav>
  );
}

interface SettingsTabSuspenseProps {
  loadingTestId: string;
  loadingLabel: string;
  children: ReactNode;
}

export function SettingsTabSuspense({
  loadingTestId,
  loadingLabel,
  children,
}: SettingsTabSuspenseProps) {
  return (
    <Suspense
      fallback={
        <div
          className="rounded-xl border border-border/50 bg-card/50 px-4 py-8 text-sm text-muted-foreground"
          data-testid={loadingTestId}
        >
          {loadingLabel}
        </div>
      }
    >
      {children}
    </Suspense>
  );
}
