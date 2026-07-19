import { useMemo } from "react";
import { formatError, modeKey } from "../../lib/display";
import {
  flattenProviderModels,
  type ConnectProtocolOption,
  type KnownProviderEntry,
  type ProviderRecord,
} from "../../lib/provider";
import { THEMES, type ExecutionMode, type ThemeId } from "../../lib/webRuntime";
import type { RecentModelRecord } from "../../lib/workspace";
import type {
  ConnectFormActions,
  ConnectFormState,
} from "../../hooks/useProviderConnectForm";
import { SettingsDrawer } from "./SettingsDrawer";

interface SettingsPageProps {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  connectForm: ConnectFormState;
  connectFormActions: ConnectFormActions;
  connectProtocols: ConnectProtocolOption[];
  knownProviders: KnownProviderEntry[];
  modes: ExecutionMode[];
  onBanner: (message: string | null) => void;
  onClose: () => void;
  onModeChange: (mode: string) => void;
  onModelChange: (model: string) => void;
  onReloadCoreData: () => Promise<void>;
  onShowThinkingChange: (value: boolean) => void;
  onThemeChange: (theme: ThemeId) => void;
  providers: ProviderRecord[];
  recentModels: RecentModelRecord[];
  selectedMode: string;
  selectedModel: string;
  selectedSessionId: string | null;
  showThinking: boolean;
  theme: ThemeId;
  workspaceConfigDir: string | null;
  workspaceMode: "shared" | "isolated" | null;
  workspaceRootPath: string;
}

export function SettingsPage({
  api,
  apiJson,
  connectForm,
  connectFormActions,
  connectProtocols,
  knownProviders,
  modes,
  onBanner,
  onClose,
  onModeChange,
  onModelChange,
  onReloadCoreData,
  onShowThinkingChange,
  onThemeChange,
  providers,
  recentModels,
  selectedMode,
  selectedModel,
  selectedSessionId,
  showThinking,
  theme,
  workspaceConfigDir,
  workspaceMode,
  workspaceRootPath,
}: SettingsPageProps) {
  const modelOptions = useMemo(() => {
    const options = flattenProviderModels(providers);
    if (recentModels.length === 0) return options;
    const recentKeys = recentModels.map((entry) => `${entry.provider}/${entry.model}`);
    const recentSet = new Set(recentKeys);
    return [
      ...recentKeys
        .map((key) => options.find((option) => option.key === key))
        .filter((option): option is (typeof options)[number] => Boolean(option)),
      ...options.filter((option) => !recentSet.has(option.key)),
    ];
  }, [providers, recentModels]);
  const settingsModeOptions = useMemo(
    () =>
      modes.map((mode) => ({
        key: modeKey(mode),
        label: mode.kind === "agent" ? mode.name : `${mode.kind}:${mode.name}`,
      })),
    [modes],
  );

  const connectProvider = async () => {
    const providerId = connectForm.providerId.trim();
    const apiKey = connectForm.apiKey.trim();
    if (!providerId || !apiKey) {
      onBanner("provider_id and api_key are required");
      return;
    }

    const baseUrl = connectForm.baseUrl.trim();
    const defaultProtocol = connectProtocols[0]?.id || "openai";
    const protocol = connectForm.protocol.trim() || defaultProtocol;
    const suggestedDraft = connectForm.resolution?.draft ?? null;
    const suggestedBaseUrl = suggestedDraft?.base_url?.trim() ?? "";
    const suggestedProtocol = suggestedDraft?.protocol?.trim() || defaultProtocol;

    connectFormActions.setBusy(true);
    try {
      const useKnownQuickConnect =
        suggestedDraft?.mode === "known" &&
        suggestedDraft.provider_id.toLowerCase() === providerId.toLowerCase() &&
        ((baseUrl === suggestedBaseUrl && protocol === suggestedProtocol) || !baseUrl);
      if (!useKnownQuickConnect && !baseUrl) {
        onBanner("Custom or advanced provider connect requires a base URL.");
        return;
      }

      await api("/provider/connect", {
        method: "POST",
        body: JSON.stringify({
          provider_id: providerId,
          api_key: apiKey,
          base_url: useKnownQuickConnect ? undefined : baseUrl,
          protocol: useKnownQuickConnect ? undefined : protocol,
        }),
      });
      connectFormActions.setApiKey("");
      connectFormActions.setBaseUrl("");
      await onReloadCoreData();
      onBanner(`Connected provider ${providerId}`);
    } catch (error) {
      onBanner(`Provider connect failed: ${formatError(error)}`);
    } finally {
      connectFormActions.setBusy(false);
    }
  };

  return (
    <SettingsDrawer
      onClose={onClose}
      theme={theme}
      themes={THEMES}
      onThemeChange={(nextTheme) => onThemeChange(nextTheme as ThemeId)}
      workspaceMode={workspaceMode}
      workspaceRootPath={workspaceRootPath}
      workspaceConfigDir={workspaceConfigDir}
      selectedSessionId={selectedSessionId}
      modeOptions={settingsModeOptions}
      selectedMode={selectedMode}
      onModeChange={onModeChange}
      modelOptions={modelOptions}
      selectedModel={selectedModel}
      onModelChange={onModelChange}
      showThinking={showThinking}
      onShowThinkingChange={onShowThinkingChange}
      providers={providers}
      knownProviders={knownProviders}
      connectProtocols={connectProtocols}
      connectQuery={connectForm.query}
      onConnectQueryChange={connectFormActions.setQuery}
      connectResolution={connectForm.resolution}
      connectResolveBusy={connectForm.resolveBusy}
      connectResolveError={connectForm.resolveError}
      connectProviderId={connectForm.providerId}
      onConnectProviderIdChange={connectFormActions.setProviderId}
      connectProtocol={connectForm.protocol}
      onConnectProtocolChange={connectFormActions.setProtocol}
      connectApiKey={connectForm.apiKey}
      onConnectApiKeyChange={connectFormActions.setApiKey}
      connectBaseUrl={connectForm.baseUrl}
      onConnectBaseUrlChange={connectFormActions.setBaseUrl}
      connectBusy={connectForm.busy}
      onConnectProvider={connectProvider}
      api={api}
      apiJson={apiJson}
      onBanner={onBanner}
      onReloadCoreData={onReloadCoreData}
    />
  );
}
