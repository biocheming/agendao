import { useCallback, useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  KnownProviderEntry,
  ManagedModelOverrideInfoRecord,
  ManagedProviderInfoRecord,
  ProviderConnectionDescriptorCandidateRecord,
  ProviderDescriptorResponseRecord,
  ProviderRecord,
  RefreshProviderCatalogueResponseRecord,
} from "@/lib/provider";
import type { SettingsTabId } from "../shared";
import {
  emptyModelOverrideDraft,
  formatError,
  modelOverrideDraftFromRecord,
} from "../shared";
import type { ModelOverrideDraft } from "../types";

export interface ProvidersSettingsState {
  managedProviders: ManagedProviderInfoRecord[];
  setManagedProviders: Dispatch<SetStateAction<ManagedProviderInfoRecord[]>>;
  selectedManagedProviderId: string | null;
  setSelectedManagedProviderId: Dispatch<SetStateAction<string | null>>;
  providerDescriptorLoading: boolean;
  setProviderDescriptorLoading: Dispatch<SetStateAction<boolean>>;
  selectedProviderDescriptor: ProviderConnectionDescriptorCandidateRecord | null;
  setSelectedProviderDescriptor: Dispatch<
    SetStateAction<ProviderConnectionDescriptorCandidateRecord | null>
  >;
  selectedProviderDescriptorError: string | null;
  setSelectedProviderDescriptorError: Dispatch<SetStateAction<string | null>>;
  modelOverrideDraft: ModelOverrideDraft;
  setModelOverrideDraft: Dispatch<SetStateAction<ModelOverrideDraft>>;
  editingModelTarget: { providerId: string; modelKey: string } | null;
  setEditingModelTarget: Dispatch<
    SetStateAction<{ providerId: string; modelKey: string } | null>
  >;
  providerChoices: string[];
  modelOverrideProviderOptions: Array<{ id: string; label: string }>;
  configuredModelOverrides: Array<{
    providerId: string;
    providerName: string;
    override: ManagedModelOverrideInfoRecord;
  }>;
}

export function useProvidersSettingsState({
  providers,
  knownProviders,
}: {
  providers: ProviderRecord[];
  knownProviders: KnownProviderEntry[];
}): ProvidersSettingsState {
  const { t } = useI18n();
  const [managedProviders, setManagedProviders] = useState<ManagedProviderInfoRecord[]>([]);
  const [selectedManagedProviderId, setSelectedManagedProviderId] = useState<string | null>(null);
  const [providerDescriptorLoading, setProviderDescriptorLoading] = useState(false);
  const [selectedProviderDescriptor, setSelectedProviderDescriptor] =
    useState<ProviderConnectionDescriptorCandidateRecord | null>(null);
  const [selectedProviderDescriptorError, setSelectedProviderDescriptorError] =
    useState<string | null>(null);
  const [modelOverrideDraft, setModelOverrideDraft] = useState<ModelOverrideDraft>(
    emptyModelOverrideDraft(),
  );
  const [editingModelTarget, setEditingModelTarget] = useState<{
    providerId: string;
    modelKey: string;
  } | null>(null);

  const providerChoices = useMemo(() => {
    const seen = new Set<string>();
    const values = [
      ...managedProviders.map((provider) => provider.id),
      ...providers.map((provider) => provider.id),
      ...knownProviders.map((provider) => provider.id),
    ];
    return values.filter((value) => {
      const key = value.trim();
      if (!key || seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
  }, [knownProviders, managedProviders, providers]);
  const modelOverrideProviderOptions = useMemo(() => {
    const selectedProviderId = modelOverrideDraft.providerId.trim();
    if (!selectedProviderId || providerChoices.includes(selectedProviderId)) {
      return providerChoices.map((providerId) => ({
        id: providerId,
        label: providerId,
      }));
    }
    return [
      { id: selectedProviderId, label: t("settings.providers.customProviderSuffix", { id: selectedProviderId }) },
      ...providerChoices.map((providerId) => ({
        id: providerId,
        label: providerId,
      })),
    ];
  }, [modelOverrideDraft.providerId, providerChoices, t]);
  const configuredModelOverrides = useMemo(
    () =>
      managedProviders.flatMap((provider) =>
        (provider.model_overrides ?? []).map((override) => ({
          providerId: provider.id,
          providerName: provider.name,
          override,
        })),
      ),
    [managedProviders],
  );

  return {
    managedProviders,
    setManagedProviders,
    selectedManagedProviderId,
    setSelectedManagedProviderId,
    providerDescriptorLoading,
    setProviderDescriptorLoading,
    selectedProviderDescriptor,
    setSelectedProviderDescriptor,
    selectedProviderDescriptorError,
    setSelectedProviderDescriptorError,
    modelOverrideDraft,
    setModelOverrideDraft,
    editingModelTarget,
    setEditingModelTarget,
    providerChoices,
    modelOverrideProviderOptions,
    configuredModelOverrides,
  };
}

export interface ProvidersSettingsActionsDeps extends ProvidersSettingsState {
  activeTab: SettingsTabId;
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onBanner: (message: string) => void;
  onReloadCoreData: () => Promise<void>;
  setFeedback: Dispatch<SetStateAction<string | null>>;
  setBusyKey: Dispatch<SetStateAction<string | null>>;
  reloadSettingsData: () => Promise<void>;
  runMutation: (
    key: string,
    action: () => Promise<string | void>,
    success: string,
  ) => Promise<void>;
}

export interface ProvidersSettingsActions {
  removeProvider: (providerId: string) => Promise<void>;
  setProviderDisabled: (providerId: string, disabled: boolean) => Promise<void>;
  renameProvider: (providerId: string, name: string) => Promise<void>;
  resetModelOverrideDraft: (providerId?: string) => void;
  editModelOverride: (providerId: string, record: ManagedModelOverrideInfoRecord) => void;
  saveModelOverride: () => Promise<void>;
  deleteModelOverride: (providerId: string, modelKey: string) => Promise<void>;
  refreshProviderCatalogue: () => Promise<void>;
}

export function useProvidersSettingsActions({
  activeTab,
  api,
  apiJson,
  onBanner,
  onReloadCoreData,
  setFeedback,
  setBusyKey,
  reloadSettingsData,
  runMutation,
  managedProviders,
  selectedManagedProviderId,
  setSelectedManagedProviderId,
  setProviderDescriptorLoading,
  setSelectedProviderDescriptor,
  setSelectedProviderDescriptorError,
  modelOverrideDraft,
  setModelOverrideDraft,
  editingModelTarget,
  setEditingModelTarget,
  providerChoices,
}: ProvidersSettingsActionsDeps): ProvidersSettingsActions {
  const { t } = useI18n();
  useEffect(() => {
    if (modelOverrideDraft.providerId.trim() || providerChoices.length === 0) {
      return;
    }
    setModelOverrideDraft((current) => ({
      ...current,
      providerId: providerChoices[0],
    }));
  }, [modelOverrideDraft.providerId, providerChoices, setModelOverrideDraft]);

  useEffect(() => {
    if (managedProviders.length === 0) {
      setSelectedManagedProviderId(null);
      setSelectedProviderDescriptor(null);
      setSelectedProviderDescriptorError(null);
      setProviderDescriptorLoading(false);
      return;
    }

    const selected = (selectedManagedProviderId ?? "").trim().toLowerCase();
    const stillExists = managedProviders.some(
      (provider) => provider.id.trim().toLowerCase() === selected,
    );
    if (stillExists) {
      return;
    }
    setSelectedManagedProviderId(managedProviders[0].id);
  }, [
    managedProviders,
    selectedManagedProviderId,
    setProviderDescriptorLoading,
    setSelectedManagedProviderId,
    setSelectedProviderDescriptor,
    setSelectedProviderDescriptorError,
  ]);

  useEffect(() => {
    if (activeTab !== "providers" || !selectedManagedProviderId) {
      setProviderDescriptorLoading(false);
      return;
    }

    let cancelled = false;
    setProviderDescriptorLoading(true);
    setSelectedProviderDescriptor(null);
    setSelectedProviderDescriptorError(null);

    void (async () => {
      try {
        const response = await apiJson<ProviderDescriptorResponseRecord>(
          `/provider/${encodeURIComponent(selectedManagedProviderId)}/descriptor`,
        );
        if (cancelled) return;
        setSelectedProviderDescriptor(response.descriptor_candidate ?? null);
        setSelectedProviderDescriptorError(response.descriptor_candidate_error ?? null);
      } catch (error) {
        if (cancelled) return;
        setSelectedProviderDescriptor(null);
        setSelectedProviderDescriptorError(formatError(error));
      } finally {
        if (!cancelled) {
          setProviderDescriptorLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    activeTab,
    apiJson,
    selectedManagedProviderId,
    setProviderDescriptorLoading,
    setSelectedProviderDescriptor,
    setSelectedProviderDescriptorError,
  ]);

  const removeProvider = async (providerId: string) => {
    await runMutation(
      `provider:delete:${providerId}`,
      async () => {
        await api(`/provider/${encodeURIComponent(providerId)}`, { method: "DELETE" });
      },
      t("settings.feedback.providerRemoved", { id: providerId }),
    );
  };

  const setProviderDisabled = async (providerId: string, disabled: boolean) => {
    await runMutation(
      `provider:disabled:${providerId}`,
      async () => {
        await api(`/provider/${encodeURIComponent(providerId)}/disabled`, {
          method: "PUT",
          body: JSON.stringify({ disabled }),
        });
      },
      disabled
        ? t("settings.feedback.providerDisabled", { id: providerId })
        : t("settings.feedback.providerEnabled", { id: providerId }),
    );
  };

  const renameProvider = async (providerId: string, name: string) => {
    await runMutation(
      `provider:rename:${providerId}`,
      async () => {
        await api(`/provider/${encodeURIComponent(providerId)}`, {
          method: "PUT",
          body: JSON.stringify({ name }),
        });
      },
      t("settings.feedback.providerRenamed", { id: providerId }),
    );
  };

  const resetModelOverrideDraft = useCallback(
    (providerId?: string) => {
      setEditingModelTarget(null);
      setModelOverrideDraft(
        emptyModelOverrideDraft(
          providerId ?? modelOverrideDraft.providerId ?? providerChoices[0] ?? "",
        ),
      );
    },
    [modelOverrideDraft.providerId, providerChoices, setEditingModelTarget, setModelOverrideDraft],
  );

  const editModelOverride = useCallback(
    (providerId: string, record: ManagedModelOverrideInfoRecord) => {
      setEditingModelTarget({ providerId, modelKey: record.key });
      setModelOverrideDraft(modelOverrideDraftFromRecord(providerId, record));
    },
    [setEditingModelTarget, setModelOverrideDraft],
  );

  const saveModelOverride = async () => {
    const providerId = modelOverrideDraft.providerId.trim();
    const modelKey = modelOverrideDraft.modelKey.trim();
    if (!providerId) {
      throw new Error(t("settings.feedback.providerIdRequired"));
    }
    if (!modelKey) {
      throw new Error(t("settings.feedback.modelKeyRequired"));
    }

    await runMutation(
      `provider:model:save:${providerId}:${modelKey}`,
      async () => {
        await api(
          `/config/provider/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelKey)}`,
          {
            method: "PUT",
            body: JSON.stringify({
              model: modelOverrideDraft.modelId.trim() || undefined,
              name: modelOverrideDraft.name.trim() || undefined,
              base_url: modelOverrideDraft.baseUrl.trim() || undefined,
              family: modelOverrideDraft.family.trim() || undefined,
              status: modelOverrideDraft.status.trim() || undefined,
              release_date: modelOverrideDraft.releaseDate.trim() || undefined,
              reasoning: modelOverrideDraft.reasoning,
              tool_call: modelOverrideDraft.toolCall,
              attachment: modelOverrideDraft.attachment,
              temperature: modelOverrideDraft.temperature,
              experimental: modelOverrideDraft.experimental,
            }),
          },
        );
        resetModelOverrideDraft(providerId);
      },
      t("settings.feedback.modelOverrideSaved", { id: `${providerId}/${modelKey}` }),
    );
  };

  const deleteModelOverride = async (providerId: string, modelKey: string) => {
    await runMutation(
      `provider:model:delete:${providerId}:${modelKey}`,
      async () => {
        await api(
          `/config/provider/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelKey)}`,
          { method: "DELETE" },
        );
        if (
          editingModelTarget?.providerId === providerId &&
          editingModelTarget?.modelKey === modelKey
        ) {
          resetModelOverrideDraft(providerId);
        }
      },
      t("settings.feedback.modelOverrideRemoved", { id: `${providerId}/${modelKey}` }),
    );
  };

  const refreshProviderCatalogue = async () => {
    setBusyKey("provider:refresh");
    setFeedback(null);
    try {
      const response = await apiJson<RefreshProviderCatalogueResponseRecord>("/provider/refresh", {
        method: "POST",
      });
      await Promise.all([reloadSettingsData(), onReloadCoreData()]);
      const message =
        response.status === "updated"
          ? t("settings.feedback.catalogueUpdated", {
              before: response.generation_before,
              after: response.generation_after,
            })
          : response.status === "not_modified"
            ? t("settings.feedback.catalogueNotModified", { generation: response.generation_after })
            : t("settings.feedback.catalogueRefreshFailed", {
                error: response.error_message ?? t("settings.feedback.unknownRefreshFailure"),
              });
      setFeedback(message);
      if (response.status === "fallback_cached") {
        onBanner(message);
      }
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  return {
    removeProvider,
    setProviderDisabled,
    renameProvider,
    resetModelOverrideDraft,
    editModelOverride,
    saveModelOverride,
    deleteModelOverride,
    refreshProviderCatalogue,
  };
}
