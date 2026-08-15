import { lazy } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import { GeneralTab } from "../settings-drawer/GeneralTab";
import {
  SettingsDrawerHeader,
  SettingsDrawerNav,
  SettingsTabSuspense,
} from "../settings-drawer/DrawerChrome";
import { useSettingsDrawerController } from "../settings-drawer/useSettingsDrawerController";
import type { SettingsDrawerProps } from "../settings-drawer/types";

const MemoryTab = lazy(async () => {
  const module = await import("../settings-drawer/MemoryTab");
  return { default: module.MemoryTab };
});

const SkillsTab = lazy(async () => {
  const module = await import("../settings-drawer/SkillsTab");
  return { default: module.SkillsTab };
});

const ProvidersTab = lazy(async () => {
  const module = await import("../settings-drawer/ProvidersTab");
  return { default: module.ProvidersTab };
});

const ValidationTab = lazy(async () => {
  const module = await import("../settings-drawer/ValidationTab");
  return { default: module.ValidationTab };
});

const McpTab = lazy(async () => {
  const module = await import("../settings-drawer/McpTab");
  return { default: module.McpTab };
});

const PluginsTab = lazy(async () => {
  const module = await import("../settings-drawer/PluginsTab");
  return { default: module.PluginsTab };
});

const LspTab = lazy(async () => {
  const module = await import("../settings-drawer/LspTab");
  return { default: module.LspTab };
});

export function SettingsDrawer(props: SettingsDrawerProps) {
  const view = useSettingsDrawerController(props);
  const { t } = useI18n();

  return (
    <div className="roc-app-shell flex h-dvh flex-col overflow-hidden bg-background text-foreground font-sans" data-testid="settings-page">
      <div className="mx-auto flex h-full w-full max-w-[110rem] flex-1 flex-col overflow-hidden px-4 py-6 md:px-6">
        <section
          className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-4xl border border-border/60 bg-background shadow-sm"
          data-testid="settings-drawer"
        >
        <SettingsDrawerHeader
          refreshing={view.refreshing}
          onRefresh={() => void view.reloadSettingsData()}
          onClose={props.onClose}
        />

        <SettingsDrawerNav activeTab={view.activeTab} onTabChange={view.onActiveTabChange} />

        <div className="flex flex-1 min-h-0 flex-col gap-6 overflow-y-auto px-5 pb-6 pt-5 sm:px-7 sm:pb-7 sm:pt-6">
          {view.feedback ? <div className="rounded-lg border border-(--ds-error)/40 bg-(--ds-error)/12 px-4 py-2.5 text-sm text-(--ds-error)" data-testid="settings-feedback">{view.feedback}</div> : null}
          {view.loading ? <div className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">{t("app.loadingSettings")}</div> : null}
          {!view.loading && view.isolatedNotice ? (
            <div className="rounded-lg border border-(--ds-warn)/40 bg-(--ds-warn)/12 px-4 py-2.5 text-sm leading-relaxed text-(--ds-warn)">
              {view.isolatedNotice}
            </div>
          ) : null}

          {!view.loading && view.activeTab === "general" ? (
            <GeneralTab {...view.generalTabProps} />
          ) : null}

          {!view.loading && view.activeTab === "memory" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-memory-loading"
              loadingLabel={t("settings.loading.memoryTools")}
            >
              <div data-testid="settings-panel-memory">
                <MemoryTab {...view.memoryTabProps} />
              </div>
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "providers" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-providers-loading"
              loadingLabel={t("settings.loading.providers")}
            >
              <ProvidersTab {...view.providersTabProps} />
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "validation" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-validation-loading"
              loadingLabel={t("settings.loading.validation")}
            >
              <ValidationTab {...view.validationTabProps} />
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "skills" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-skills-loading"
              loadingLabel={t("settings.loading.skills")}
            >
              <div data-testid="settings-panel-skills">
                <SkillsTab {...view.skillsTabProps} />
              </div>
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "mcp" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-mcp-loading"
              loadingLabel={t("settings.loading.mcp")}
            >
              <McpTab {...view.mcpTabProps} />
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "plugins" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-plugins-loading"
              loadingLabel={t("settings.loading.plugins")}
            >
              <PluginsTab {...view.pluginsTabProps} />
            </SettingsTabSuspense>
          ) : null}

          {!view.loading && view.activeTab === "lsp" ? (
            <SettingsTabSuspense
              loadingTestId="settings-panel-lsp-loading"
              loadingLabel={t("settings.loading.lsp")}
            >
              <LspTab {...view.lspTabProps} />
            </SettingsTabSuspense>
          ) : null}
        </div>
        </section>
      </div>
    </div>
  );
}
