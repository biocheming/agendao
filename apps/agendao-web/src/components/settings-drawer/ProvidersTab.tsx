import { useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { Pencil } from "lucide-react";
import type {
  ConnectProtocolOption,
  KnownProviderEntry,
  ManagedModelOverrideInfoRecord,
  ManagedProviderInfoRecord,
  ProviderConnectionDescriptorCandidateRecord,
  ProviderProfileDescriptorViewRecord,
  ProviderRecord,
  ResolveProviderConnectResponseRecord,
  TestProviderConnectionResponseRecord,
  TestProviderModelResponseRecord,
} from "@/lib/provider";
import { displayProtocolLabel } from "@/lib/provider";
import { apiJson, formatError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import type { ModelOverrideDraft } from "./types";

interface ProvidersTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  formFieldClass: string;
  formLabelClass: string;
  formHintClass: string;
  inputClass: string;
  selectClass: string;
  checkboxRowClass: string;
  checkboxClass: string;
}

export interface ProvidersTabProps {
  styles: ProvidersTabStyles;
  busyKey: string | null;
  providers: ProviderRecord[];
  providerSummary: string;
  connectProtocols: ConnectProtocolOption[];
  connectQuery: string;
  onConnectQueryChange: (value: string) => void;
  connectResolution: ResolveProviderConnectResponseRecord | null;
  connectResolveBusy: boolean;
  connectResolveError: string | null;
  connectProviderId: string;
  onConnectProviderIdChange: (value: string) => void;
  connectProtocol: string;
  onConnectProtocolChange: (value: string) => void;
  connectApiKey: string;
  onConnectApiKeyChange: (value: string) => void;
  connectBaseUrl: string;
  onConnectBaseUrlChange: (value: string) => void;
  connectBusy: boolean;
  onConnectProvider: () => Promise<void>;
  onReloadSettingsData: () => Promise<void>;
  onRemoveProvider: (providerId: string) => void;
  onToggleProviderDisabled: (providerId: string, disabled: boolean) => void;
  onRenameProvider: (providerId: string, name: string) => void;
  onRefreshProviderCatalogue: () => void;
  managedProviders: ManagedProviderInfoRecord[];
  selectedManagedProviderId: string | null;
  onSelectedManagedProviderIdChange: (value: string) => void;
  providerDescriptorLoading: boolean;
  selectedProviderDescriptor: ProviderConnectionDescriptorCandidateRecord | null;
  selectedProviderDescriptorError: string | null;
  modelOverrideDraft: ModelOverrideDraft;
  onModelOverrideDraftChange: Dispatch<SetStateAction<ModelOverrideDraft>>;
  editingModelTarget: { providerId: string; modelKey: string } | null;
  modelOverrideProviderOptions: Array<{ id: string; label: string }>;
  configuredModelOverrides: Array<{
    providerId: string;
    providerName: string;
    override: ManagedModelOverrideInfoRecord;
  }>;
  onResetModelOverrideDraft: (providerId?: string) => void;
  onEditModelOverride: (providerId: string, record: ManagedModelOverrideInfoRecord) => void;
  onSaveModelOverride: () => void;
  onDeleteModelOverride: (providerId: string, modelKey: string) => void;
}

function statusTone(status: string | null | undefined) {
  switch ((status || "").toLowerCase()) {
    case "connected":
    case "done":
      return "ok";
    case "needs-auth":
    case "warning":
      return "warn";
    case "error":
    case "failed":
      return "danger";
    default:
      return "muted";
  }
}

export function ProvidersTab({
  styles,
  busyKey,
  providers,
  providerSummary,
  connectProtocols,
  connectQuery,
  onConnectQueryChange,
  connectResolution,
  connectResolveBusy,
  connectResolveError,
  connectProviderId,
  onConnectProviderIdChange,
  connectProtocol,
  onConnectProtocolChange,
  connectApiKey,
  onConnectApiKeyChange,
  connectBaseUrl,
  onConnectBaseUrlChange,
  connectBusy,
  onConnectProvider,
  onReloadSettingsData,
  onRemoveProvider,
  onToggleProviderDisabled,
  onRenameProvider,
  onRefreshProviderCatalogue,
  managedProviders,
  selectedManagedProviderId,
  onSelectedManagedProviderIdChange,
  providerDescriptorLoading,
  selectedProviderDescriptor,
  selectedProviderDescriptorError,
  modelOverrideDraft,
  onModelOverrideDraftChange,
  editingModelTarget,
  modelOverrideProviderOptions,
  configuredModelOverrides,
  onResetModelOverrideDraft,
  onEditModelOverride,
  onSaveModelOverride,
  onDeleteModelOverride,
}: ProvidersTabProps) {
  const {
    primaryButtonClass,
    secondaryButtonClass,
    formFieldClass,
    formLabelClass,
    formHintClass,
    inputClass,
    selectClass,
    checkboxRowClass,
    checkboxClass,
  } = styles;
  const { t } = useI18n();
  const [renamingProviderId, setRenamingProviderId] = useState<string | null>(null);
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null);
  const [testingModelKey, setTestingModelKey] = useState<string | null>(null);
  const [modelTestResults, setModelTestResults] = useState<
    Record<string, TestProviderModelResponseRecord>
  >({});
  const [providerTestState, setProviderTestState] = useState<{
    source: ManagedProviderInfoRecord[];
    results: Record<string, TestProviderConnectionResponseRecord>;
  }>({ source: managedProviders, results: {} });
  // Test results are per-load snapshots: reloadSettingsData swaps the
  // managedProviders reference, which drops stale results without an effect.
  const providerTestResults =
    providerTestState.source === managedProviders ? providerTestState.results : {};
  const connectMatches = connectResolution?.matches ?? [];
  const exactKnownProvider = connectResolution?.exact_match ? connectResolution.draft : null;
  const selectedManagedProvider = useMemo(
    () =>
      managedProviders.find(
        (provider) =>
          provider.id.trim().toLowerCase() === (selectedManagedProviderId ?? "").trim().toLowerCase(),
      ) ?? null,
    [managedProviders, selectedManagedProviderId],
  );
  const selectedProviderProfile = useMemo<ProviderProfileDescriptorViewRecord | null>(
    () => selectedProviderDescriptor?.profile ?? null,
    [selectedProviderDescriptor],
  );
  const chooseKnownProvider = (provider: KnownProviderEntry) => {
    onConnectQueryChange(provider.id);
  };
  const submitProviderRename = (providerId: string, value: string) => {
    const trimmed = value.trim();
    setRenamingProviderId(null);
    if (!trimmed) return;
    const current = managedProviders.find((provider) => provider.id === providerId);
    if (current && current.name === trimmed) return;
    onRenameProvider(providerId, trimmed);
  };
  const testProviderConnection = async (providerId: string) => {
    setTestingProviderId(providerId);
    const recordResult = (result: TestProviderConnectionResponseRecord) => {
      setProviderTestState((current) => ({
        source: managedProviders,
        results: {
          ...(current.source === managedProviders ? current.results : {}),
          [providerId]: result,
        },
      }));
    };
    try {
      recordResult(
        await apiJson<TestProviderConnectionResponseRecord>(
          `/provider/${encodeURIComponent(providerId)}/test`,
          { method: "POST" },
        ),
      );
    } catch (error) {
      recordResult({ ok: false, latency_ms: 0, error: formatError(error) });
    } finally {
      setTestingProviderId(null);
    }
  };
  const testProviderModel = async (providerId: string, modelId: string) => {
    const key = `${providerId}/${modelId}`;
    setTestingModelKey(key);
    try {
      const result = await apiJson<TestProviderModelResponseRecord>(
        `/provider/${encodeURIComponent(providerId)}/model-test`,
        { method: "POST", body: JSON.stringify({ model_id: modelId }) },
      );
      setModelTestResults((current) => ({ ...current, [key]: result }));
    } catch (error) {
      setModelTestResults((current) => ({
        ...current,
        [key]: {
          ok: false,
          model_id: modelId,
          latency_ms: 0,
          error: formatError(error),
        },
      }));
    } finally {
      setTestingModelKey(null);
    }
  };
  return (
    <div className="grid gap-6" data-testid="settings-panel-providers">
      <form
        className="roc-section"
        onSubmit={(event) => {
          event.preventDefault();
          void (async () => {
            await onConnectProvider();
            await onReloadSettingsData();
          })();
        }}
      >
        <label htmlFor="settings-provider-connect-query" className={formLabelClass}>{t("settings.providers.connectProvider")}</label>
        <input
          id="settings-provider-connect-query"
          data-testid="settings-provider-connect-query"
          className={inputClass}
          type="text"
          placeholder={t("settings.providers.searchPlaceholder")}
          value={connectQuery}
          onChange={(event) => onConnectQueryChange(event.target.value)}
        />
        {connectQuery.trim() ? (
          <div className="grid gap-2 rounded-[18px] border border-border/35 bg-muted/10 p-3">
            {connectResolveBusy ? (
              <p className={formHintClass}>
                {t("settings.providers.resolving")}
              </p>
            ) : connectResolveError ? (
              <p className="m-0 text-sm text-(--ds-error)">
                {t("settings.providers.resolveFailed", { error: connectResolveError })}
              </p>
            ) : connectMatches.length > 0 ? (
              connectMatches.map((provider) => (
                <button
                  key={provider.id}
                  type="button"
                  className="roc-item grid gap-1 text-left"
                  onClick={() => chooseKnownProvider(provider)}
                >
                  <strong>{provider.name}</strong>
                  <span className="block text-muted-foreground">
                    {provider.id}
                    {provider.protocol ? ` · ${displayProtocolLabel(provider.protocol)}` : ""}
                    {provider.base_url ? ` · ${provider.base_url}` : ""}
                  </span>
                </button>
              ))
            ) : (
              <p className={formHintClass}>
                {t("settings.providers.noMatchFallback")}{" "}
                <code>{connectResolution?.custom_draft.provider_id || connectQuery.trim()}</code>.
              </p>
            )}
          </div>
        ) : null}
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("settings.providers.providerId")}</span>
          <input
            data-testid="settings-provider-id"
            className={inputClass}
            type="text"
            placeholder={t("settings.providers.providerIdPlaceholder")}
            value={connectProviderId}
            onChange={(event) => onConnectProviderIdChange(event.target.value)}
          />
        </label>
        {exactKnownProvider ? (
          <p className={formHintClass}>
            {t("settings.providers.knownMatchBase")}
            {exactKnownProvider.env?.length
              ? t("settings.providers.knownMatchEnv", { env: exactKnownProvider.env.join(", ") })
              : ""}
            {exactKnownProvider.model_count
              ? t("settings.providers.knownMatchModels", { count: exactKnownProvider.model_count })
              : ""}
          </p>
        ) : connectResolution?.draft.mode === "custom" ? (
          <p className={formHintClass}>
            {t("settings.providers.customDraftHint")}
          </p>
        ) : null}
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("settings.providers.apiKey")}</span>
          <input
            data-testid="settings-provider-api-key"
            className={inputClass}
            type="password"
            placeholder={t("settings.providers.apiKeyPlaceholder")}
            value={connectApiKey}
            onChange={(event) => onConnectApiKeyChange(event.target.value)}
          />
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("settings.providers.baseUrl")}</span>
          <input
            data-testid="settings-provider-base-url"
            className={inputClass}
            type="url"
            placeholder={t("settings.providers.baseUrlPlaceholder")}
            value={connectBaseUrl}
            onChange={(event) => onConnectBaseUrlChange(event.target.value)}
          />
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("settings.providers.protocol")}</span>
          <select
            data-testid="settings-provider-protocol"
            className={selectClass}
            value={connectProtocol}
            onChange={(event) => onConnectProtocolChange(event.target.value)}
          >
            {connectProtocols.map((protocol) => (
              <option key={protocol.id} value={protocol.id}>
                {displayProtocolLabel(protocol.name)}
              </option>
            ))}
          </select>
        </label>
        <button className={primaryButtonClass} type="submit" disabled={connectBusy} data-testid="settings-provider-submit">
          {connectBusy ? t("settings.providers.connecting") : t("settings.providers.connect")}
        </button>
      </form>

      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.providers.configuredProviders")}</p>
          <div className="flex items-center gap-2">
            <span>{providerSummary}</span>
            <button
              className={secondaryButtonClass}
              type="button"
              data-testid="settings-provider-refresh"
              disabled={busyKey === "provider:refresh"}
              onClick={() => void onRefreshProviderCatalogue()}
            >
              {busyKey === "provider:refresh" ? t("settings.providers.refreshing") : t("settings.providers.refreshCatalogue")}
            </button>
          </div>
        </div>
        {providers.map((provider) => (
          <div key={provider.id} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4" data-testid={`settings-provider-row-${provider.id}`}>
            <div>
              <strong>{provider.name}</strong>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {provider.id} · {t("settings.providers.modelsCount", { count: (provider.models ?? []).length })}
              </p>
            </div>
            <button
              className={secondaryButtonClass}
              type="button"
              data-testid={`settings-provider-remove-${provider.id}`}
              disabled={busyKey === `provider:delete:${provider.id}`}
              onClick={() => void onRemoveProvider(provider.id)}
            >
              {t("settings.providers.remove")}
            </button>
          </div>
        ))}
      </div>

      <div className="grid gap-4 rounded-lg border border-border/35 bg-card/70 p-5">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="m-0 text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.providers.modelOverrides")}</p>
            <p className="m-0 text-sm text-muted-foreground">
              {t("settings.providers.modelOverridesDescription")}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span>{t("settings.providers.configuredCount", { count: configuredModelOverrides.length })}</span>
            <button
              className={secondaryButtonClass}
              type="button"
              onClick={() => onResetModelOverrideDraft()}
            >
              {editingModelTarget ? t("settings.providers.newOverride") : t("settings.providers.reset")}
            </button>
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.providerId")}</span>
            <select
              className={selectClass}
              value={modelOverrideDraft.providerId.trim()}
              disabled={modelOverrideProviderOptions.length === 0}
              data-testid="settings-model-override-provider"
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  providerId: event.target.value,
                }))
              }
            >
              <option value="" disabled>
                {modelOverrideProviderOptions.length
                  ? t("settings.providers.selectProvider")
                  : t("settings.providers.noProvidersAvailable")}
              </option>
              {modelOverrideProviderOptions.map(({ id, label }) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
            <span className={formHintClass}>
              {t("settings.providers.overrideHint")}
            </span>
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.modelKey")}</span>
            <input
              className={inputClass}
              value={modelOverrideDraft.modelKey}
              disabled={Boolean(editingModelTarget)}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  modelKey: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.upstreamModelId")}</span>
            <input
              className={inputClass}
              placeholder="gpt-4.1, qwen-max, claude-sonnet-4..."
              value={modelOverrideDraft.modelId}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  modelId: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.displayName")}</span>
            <input
              className={inputClass}
              value={modelOverrideDraft.name}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  name: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.baseUrl")}</span>
            <input
              className={inputClass}
              value={modelOverrideDraft.baseUrl}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  baseUrl: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.family")}</span>
            <input
              className={inputClass}
              value={modelOverrideDraft.family}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  family: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.status")}</span>
            <input
              className={inputClass}
              value={modelOverrideDraft.status}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  status: event.target.value,
                }))
              }
            />
          </label>
          <label className={formFieldClass}>
            <span className={formLabelClass}>{t("settings.providers.releaseDate")}</span>
            <input
              className={inputClass}
              placeholder="2026-04-24"
              value={modelOverrideDraft.releaseDate}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  releaseDate: event.target.value,
                }))
              }
            />
          </label>
        </div>

        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
          <label className={checkboxRowClass}>
            <input
              className={checkboxClass}
              type="checkbox"
              checked={modelOverrideDraft.reasoning}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  reasoning: event.target.checked,
                }))
              }
            />
            {t("settings.providers.capability.reasoning")}
          </label>
          <label className={checkboxRowClass}>
            <input
              className={checkboxClass}
              type="checkbox"
              checked={modelOverrideDraft.toolCall}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  toolCall: event.target.checked,
                }))
              }
            />
            {t("settings.providers.capability.toolCall")}
          </label>
          <label className={checkboxRowClass}>
            <input
              className={checkboxClass}
              type="checkbox"
              checked={modelOverrideDraft.attachment}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  attachment: event.target.checked,
                }))
              }
            />
            {t("settings.providers.capability.attachment")}
          </label>
          <label className={checkboxRowClass}>
            <input
              className={checkboxClass}
              type="checkbox"
              checked={modelOverrideDraft.temperature}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  temperature: event.target.checked,
                }))
              }
            />
            {t("settings.providers.capability.temperature")}
          </label>
          <label className={checkboxRowClass}>
            <input
              className={checkboxClass}
              type="checkbox"
              checked={modelOverrideDraft.experimental}
              onChange={(event) =>
                onModelOverrideDraftChange((current) => ({
                  ...current,
                  experimental: event.target.checked,
                }))
              }
            />
            {t("settings.providers.capability.experimental")}
          </label>
        </div>

        <div className="flex items-center gap-2">
          <button
            className={primaryButtonClass}
            type="button"
            disabled={
              !modelOverrideDraft.providerId.trim() ||
              !modelOverrideDraft.modelKey.trim() ||
              busyKey ===
                `provider:model:save:${modelOverrideDraft.providerId.trim()}:${modelOverrideDraft.modelKey.trim()}`
            }
            onClick={() => void onSaveModelOverride()}
          >
            {editingModelTarget ? t("settings.providers.saveOverride") : t("settings.providers.addOverride")}
          </button>
          {editingModelTarget ? (
            <button
              className={secondaryButtonClass}
              type="button"
              onClick={() => onResetModelOverrideDraft(modelOverrideDraft.providerId)}
            >
              {t("settings.providers.cancelEdit")}
            </button>
          ) : null}
        </div>

        <div className="grid gap-3">
          {configuredModelOverrides.length ? (
            configuredModelOverrides.map(({ providerId, providerName, override }) => (
              <div
                key={`${providerId}/${override.key}`}
                className="rounded-lg border border-border/35 bg-muted/20 p-4"
              >
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <strong>{providerId}/{override.key}</strong>
                    <p className="m-0 text-sm text-muted-foreground leading-relaxed">
                      {providerName}
                      {override.model ? ` · ${t("settings.providers.metaModel", { value: override.model })}` : ""}
                      {override.name ? ` · ${override.name}` : ""}
                    </p>
                    <p className="m-0 text-sm text-muted-foreground leading-relaxed">
                      {[
                        override.base_url ? `base ${override.base_url}` : null,
                        override.family ? `family ${override.family}` : null,
                        override.status ? `status ${override.status}` : null,
                        override.release_date ? `release ${override.release_date}` : null,
                      ]
                        .filter(Boolean)
                        .join(" · ") || t("settings.providers.noExtraMetadata")}
                    </p>
                    <p className="m-0 text-sm text-muted-foreground leading-relaxed">
                      {[
                        override.reasoning ? "reasoning" : null,
                        override.tool_call ? "tool-call" : null,
                        override.attachment ? "attachment" : null,
                        override.temperature ? "temperature" : null,
                        override.experimental ? "experimental" : null,
                      ]
                        .filter(Boolean)
                        .join(" · ") || t("settings.providers.noCapabilityFlags")}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      onClick={() => onEditModelOverride(providerId, override)}
                    >
                      {t("settings.common.edit")}
                    </button>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={
                        busyKey === `provider:model:delete:${providerId}:${override.key}`
                      }
                      onClick={() => void onDeleteModelOverride(providerId, override.key)}
                    >
                      {t("settings.common.delete")}
                    </button>
                  </div>
                </div>
              </div>
            ))
          ) : (
            <p className="m-0 text-sm text-muted-foreground">
              {t("settings.providers.noOverrides")}
            </p>
          )}
        </div>
      </div>

      <div className="grid gap-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.providers.managedProviders")}</p>
          <span>{t("settings.providers.itemsCount", { count: managedProviders.length })}</span>
        </div>
        {managedProviders.map((provider) => {
          const selected =
            provider.id.trim().toLowerCase() ===
            (selectedManagedProviderId ?? "").trim().toLowerCase();
          const isDisabled = provider.disabled === true;
          const renaming = renamingProviderId === provider.id;
          const testing = testingProviderId === provider.id;
          const testResult = providerTestResults[provider.id] ?? null;
          return (
            <div
              key={provider.id}
              className={cn(
                "rounded-lg border bg-card p-4 transition-colors flex items-start justify-between gap-4",
                selected
                  ? "border-primary/50 bg-primary/5"
                  : "border-border/40 hover:border-border/70",
                isDisabled && "opacity-60",
              )}
              data-testid={`settings-managed-provider-row-${provider.id}`}
              data-disabled={isDisabled ? "true" : "false"}
            >
              <div className="min-w-0 flex-1">
                {renaming ? (
                  <input
                    autoFocus
                    defaultValue={provider.name}
                    aria-label={t("settings.providers.renameProvider")}
                    className={inputClass}
                    data-testid={`settings-provider-rename-input-${provider.id}`}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        submitProviderRename(provider.id, event.currentTarget.value);
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        setRenamingProviderId(null);
                      }
                    }}
                  />
                ) : (
                  <button
                    type="button"
                    className="block w-full text-left"
                    onClick={() => onSelectedManagedProviderIdChange(provider.id)}
                  >
                    <strong>{provider.name}</strong>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {provider.id}
                    </p>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {t("settings.providers.statusLine", { status: provider.status })}
                      {provider.auth_type ? t("settings.providers.authSuffix", { value: provider.auth_type }) : ""}
                    </p>
                  </button>
                )}
                {(provider.models ?? []).length ? (
                  <div className="mt-2 grid gap-1">
                    {(provider.models ?? []).map((model) => {
                      const modelKey = `${provider.id}/${model.id}`;
                      const testingModel = testingModelKey === modelKey;
                      const modelTestResult = modelTestResults[modelKey];
                      return (
                        <div key={model.id} className="flex items-center gap-2 text-xs text-muted-foreground">
                          <span className="truncate">{model.id}</span>
                          <button
                            className={secondaryButtonClass}
                            type="button"
                            disabled={testingModel}
                            onClick={() => void testProviderModel(provider.id, model.id)}
                          >
                            {testingModel ? "测试中…" : "测试模型调用（真实请求）"}
                          </button>
                          {modelTestResult ? (
                            <span
                              className={cn(
                                "max-w-72 truncate",
                                modelTestResult.ok ? "text-(--ds-ok)" : "text-(--ds-error)",
                              )}
                              title={modelTestResult.error ?? modelTestResult.response_text ?? undefined}
                            >
                              {modelTestResult.ok
                                ? `✓ ${modelTestResult.latency_ms}ms`
                                : `✗ ${modelTestResult.error ?? "模型调用失败"}`}
                            </span>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                ) : null}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {isDisabled ? (
                  <span
                    className="rounded-full border border-border bg-muted px-3 py-1.5 text-xs font-semibold text-muted-foreground"
                    data-testid={`settings-provider-disabled-badge-${provider.id}`}
                  >
                    {t("settings.providers.disabledBadge")}
                  </span>
                ) : null}
                {provider.configured ? (
                  <>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      title={t("settings.providers.renameProvider")}
                      aria-label={t("settings.providers.renameProvider")}
                      data-testid={`settings-provider-rename-${provider.id}`}
                      onClick={() => setRenamingProviderId(provider.id)}
                    >
                      <Pencil className="h-3.5 w-3.5" />
                    </button>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      title="仅检查 Provider 的 /models 端点，不会调用模型"
                      data-testid={`settings-provider-test-${provider.id}`}
                      disabled={testing}
                      onClick={() => void testProviderConnection(provider.id)}
                    >
                      {testing
                        ? t("settings.providers.testingConnection")
                        : `${t("settings.providers.testConnection")}（/models 端点）`}
                    </button>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      data-testid={`settings-provider-toggle-${provider.id}`}
                      disabled={busyKey === `provider:disabled:${provider.id}`}
                      onClick={() => onToggleProviderDisabled(provider.id, !isDisabled)}
                    >
                      {isDisabled
                        ? t("settings.providers.enableProvider")
                        : t("settings.providers.disableProvider")}
                    </button>
                    {testResult ? (
                      <span
                        className={cn(
                          "max-w-56 truncate text-xs font-semibold",
                          testResult.ok ? "text-(--ds-ok)" : "text-(--ds-error)",
                        )}
                        title={testResult.ok ? undefined : (testResult.error ?? undefined)}
                        data-testid={`settings-provider-test-result-${provider.id}`}
                      >
                        {testResult.ok
                          ? `✓ ${t("settings.providers.testOk", { status: testResult.status ?? "—", ms: testResult.latency_ms })}`
                          : `✗ ${testResult.error ?? t("settings.providers.testFailed", { status: testResult.status ?? "?" })}`}
                      </span>
                    ) : null}
                  </>
                ) : null}
                <span className={cn("rounded-full border px-3 py-1.5 text-xs font-semibold", statusTone(provider.status) === "ok" ? "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)" : statusTone(provider.status) === "warn" ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)" : statusTone(provider.status) === "danger" ? "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)" : "border-border bg-muted text-muted-foreground")}>
                  {provider.status}
                </span>
              </div>
            </div>
          );
        })}
        {selectedManagedProvider ? (
          <div className="roc-section">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                  {t("settings.providers.inspection")}
                </p>
                <strong>
                  {selectedProviderDescriptor?.name ??
                    selectedManagedProvider.name}
                </strong>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {selectedManagedProvider.id}
                </p>
              </div>
              <span className="text-sm text-muted-foreground">
                {providerDescriptorLoading ? t("settings.common.loading") : t("settings.providers.readOnly")}
              </span>
            </div>
            {selectedProviderDescriptorError ? (
              <p className="m-0 text-sm text-(--ds-error)">
                {t("settings.providers.descriptorUnavailable", { error: selectedProviderDescriptorError })}
              </p>
            ) : null}
            {selectedProviderDescriptor ? (
              <div className="grid gap-3 md:grid-cols-2">
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.baseUrl")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderDescriptor.base_url || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.env")}</span>
                  <span className="text-sm text-foreground">
                    {(selectedProviderDescriptor.env ?? []).length
                      ? (selectedProviderDescriptor.env ?? []).join(", ")
                      : "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.profileSource")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.source || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.apiFamily")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.api_family || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.apiShape")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.api_shape || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.transport")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.transport || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.usageShape")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.usage_shape || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.cacheFamily")}</span>
                  <span className="text-sm text-foreground">
                    {selectedProviderProfile?.cache_family || "--"}
                  </span>
                </div>
                <div className="grid gap-1">
                  <span className={formLabelClass}>{t("settings.providers.quirks")}</span>
                  <span className="text-sm text-foreground">
                    {(selectedProviderProfile?.quirks ?? []).length
                      ? (selectedProviderProfile?.quirks ?? []).join(", ")
                      : "--"}
                  </span>
                </div>
              </div>
            ) : providerDescriptorLoading ? (
              <p className={formHintClass}>{t("settings.providers.loadingDescriptor")}</p>
            ) : (
              <p className={formHintClass}>
                {t("settings.providers.noDescriptor")}
              </p>
            )}
          </div>
        ) : null}
      </div>
    </div>
  );
}
