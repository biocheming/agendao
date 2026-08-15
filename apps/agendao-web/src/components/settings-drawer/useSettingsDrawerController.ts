import { useCallback, useEffect, useMemo, useState } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import type {
  ConfigPolicyValidationItemRecord,
  ConfigPolicyValidationSnapshotRecord,
} from "@/lib/configPolicy";
import type { ManagedProviderInfoRecord } from "@/lib/provider";
import type {
  SkillCatalogEntry,
  SkillHubArtifactCacheResponseRecord,
  SkillHubAuditResponseRecord,
  SkillHubDistributionResponseRecord,
  SkillHubIndexResponseRecord,
  SkillHubLifecycleResponseRecord,
  SkillHubManagedResponseRecord,
  SkillHubNegativeEntropyResponseRecord,
  SkillHubPolicyResponseRecord,
  SkillHubSemanticConflictResponseRecord,
  SkillHubTimelineResponseRecord,
  SkillHubUsageLedgerResponseRecord,
} from "@/lib/skill";
import type { GeneralTabProps } from "./GeneralTab";
import type { MemoryTabProps } from "./MemoryTab";
import type { ProvidersTabProps } from "./ProvidersTab";
import type { SkillsTabProps } from "./SkillsTab";
import type { ValidationTabProps } from "./ValidationTab";
import type { McpTabProps } from "./McpTab";
import type { PluginsTabProps } from "./PluginsTab";
import type { LspTabProps } from "./LspTab";
import {
  SETTINGS_DRAWER_STYLES,
  formatError,
  isolatedWorkspaceNotice,
  objectRecord,
  stringifyJson,
  validationJumpTarget,
} from "./shared";
import type { SettingsTabId } from "./shared";
import type {
  AppConfigSnapshot,
  FormatterStatus,
  LspStatus,
  McpStatusInfo,
  PluginAuthProviderInfo,
  SettingsDrawerProps,
} from "./types";
import { useConfigTabSettingsMutations, useConfigTabSettingsState } from "./hooks/useConfigTabSettings";
import { useMemorySettingsActions, useMemorySettingsState } from "./hooks/useMemorySettings";
import {
  useProvidersSettingsActions,
  useProvidersSettingsState,
} from "./hooks/useProvidersSettings";
import { useSkillHubSettingsActions } from "./hooks/useSkillHubSettingsActions";
import { useSkillHubSettingsState } from "./hooks/useSkillHubSettingsState";

export interface SettingsDrawerView {
  activeTab: SettingsTabId;
  onActiveTabChange: (tab: SettingsTabId) => void;
  loading: boolean;
  refreshing: boolean;
  feedback: string | null;
  isolatedNotice: string | null;
  reloadSettingsData: () => Promise<void>;
  generalTabProps: GeneralTabProps;
  memoryTabProps: MemoryTabProps;
  providersTabProps: ProvidersTabProps;
  validationTabProps: ValidationTabProps;
  skillsTabProps: SkillsTabProps;
  mcpTabProps: McpTabProps;
  pluginsTabProps: PluginsTabProps;
  lspTabProps: LspTabProps;
}

export function useSettingsDrawerController({
  theme,
  themes,
  onThemeChange,
  workspaceMode,
  workspaceRootPath,
  workspaceConfigDir,
  selectedSessionId,
  modeOptions,
  selectedMode,
  onModeChange,
  modelOptions,
  selectedModel,
  onModelChange,
  showThinking,
  onShowThinkingChange,
  providers,
  knownProviders,
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
  api,
  apiJson,
  onBanner,
  onReloadCoreData,
}: SettingsDrawerProps): SettingsDrawerView {
  const [activeTab, setActiveTab] = useState<SettingsTabId>("general");
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [configSnapshot, setConfigSnapshot] = useState<AppConfigSnapshot | null>(null);
  const { t } = useI18n();

  const providersState = useProvidersSettingsState({ providers, knownProviders });
  const memoryState = useMemorySettingsState();
  const skillsState = useSkillHubSettingsState({ selectedSessionId, workspaceRootPath });
  const configTabsState = useConfigTabSettingsState();

  const {
    setManagedProviders,
    setSelectedManagedProviderId,
  } = providersState;
  const {
    setConfigValidation,
    setMcpStatus,
    setMcpDrafts,
    setPluginAuthProviders,
    setPluginDrafts,
    setLspStatus,
    setFormatterStatus,
  } = configTabsState;
  const {
    setSkillCatalog,
    setManagedSkills,
    setSkillUsageLedger,
    setSkillNegativeEntropy,
    setSkillSemanticConflicts,
    setSkillSourceIndices,
    setSkillDistributions,
    setSkillArtifactCache,
    setSkillHubPolicy,
    setSkillLifecycle,
    setSkillGovernanceTimeline,
  } = skillsState;

  const mcpConfigs = useMemo(
    () => objectRecord(configSnapshot?.mcp),
    [configSnapshot?.mcp],
  );
  const pluginConfigs = useMemo(
    () => objectRecord(configSnapshot?.plugin),
    [configSnapshot?.plugin],
  );
  const isolatedNotice = workspaceMode === "isolated" ? isolatedWorkspaceNotice(activeTab, t) : null;
  const providerSummary = t("settings.feedback.providerSummary", {
    connected: providers.length,
    known: knownProviders.length,
  });

  const reloadSettingsData = useCallback(async () => {
    setRefreshing(true);
    setFeedback(null);
    try {
      const skillCatalogPath = selectedSessionId
        ? `/skill/catalog?session_id=${encodeURIComponent(selectedSessionId)}`
        : "/skill/catalog";
      const [config, managed, validation, mcp, plugins, lsp, formatter, skills, skillHubManaged, skillHubUsage, skillHubNegativeEntropy, skillHubSemanticConflicts, skillHubIndex, skillHubDistributions, skillHubArtifactCache, skillHubPolicyResponse, skillHubLifecycle, _skillHubAudit, skillHubTimeline] =
        await Promise.all([
          apiJson<AppConfigSnapshot>("/config"),
          apiJson<{ providers: ManagedProviderInfoRecord[] }>("/provider/managed"),
          apiJson<ConfigPolicyValidationSnapshotRecord>("/config/validation").catch(() => null),
          apiJson<Record<string, McpStatusInfo>>("/mcp"),
          apiJson<PluginAuthProviderInfo[]>("/plugin/auth").catch(() => []),
          apiJson<LspStatus>("/lsp"),
          apiJson<FormatterStatus>("/formatter"),
          apiJson<SkillCatalogEntry[]>(skillCatalogPath),
          apiJson<SkillHubManagedResponseRecord>("/skill/hub/managed"),
          apiJson<SkillHubUsageLedgerResponseRecord>("/skill/hub/usage"),
          apiJson<SkillHubNegativeEntropyResponseRecord>("/skill/hub/negative-entropy"),
          apiJson<SkillHubSemanticConflictResponseRecord>("/skill/hub/semantic-conflicts"),
          apiJson<SkillHubIndexResponseRecord>("/skill/hub/index"),
          apiJson<SkillHubDistributionResponseRecord>("/skill/hub/distributions"),
          apiJson<SkillHubArtifactCacheResponseRecord>("/skill/hub/artifact-cache"),
          apiJson<SkillHubPolicyResponseRecord>("/skill/hub/policy"),
          apiJson<SkillHubLifecycleResponseRecord>("/skill/hub/lifecycle"),
          apiJson<SkillHubAuditResponseRecord>("/skill/hub/audit"),
          apiJson<SkillHubTimelineResponseRecord>("/skill/hub/timeline?limit=120"),
        ]);
      setConfigSnapshot(config);
      setManagedProviders(managed.providers ?? []);
      setConfigValidation(validation);
      setMcpStatus(mcp ?? {});
      setMcpDrafts(
        Object.fromEntries(
          Object.entries(objectRecord(config.mcp)).map(([key, value]) => [key, stringifyJson(value)]),
        ),
      );
      setPluginAuthProviders(plugins ?? []);
      setPluginDrafts(
        Object.fromEntries(
          Object.entries(objectRecord(config.plugin)).map(([key, value]) => [key, stringifyJson(value)]),
        ),
      );
      setLspStatus(lsp);
      setFormatterStatus(formatter);
      setSkillCatalog(skills ?? []);
      setManagedSkills(skillHubManaged.managed_skills ?? []);
      setSkillUsageLedger(skillHubUsage.entries ?? []);
      setSkillNegativeEntropy(skillHubNegativeEntropy.candidates ?? []);
      setSkillSemanticConflicts(skillHubSemanticConflicts.conflicts ?? []);
      setSkillSourceIndices(skillHubIndex.source_indices ?? []);
      setSkillDistributions(skillHubDistributions.distributions ?? []);
      setSkillArtifactCache(skillHubArtifactCache.artifact_cache ?? []);
      setSkillHubPolicy(skillHubPolicyResponse.policy ?? null);
      setSkillLifecycle(skillHubLifecycle.lifecycle ?? []);
      setSkillGovernanceTimeline(skillHubTimeline.entries ?? []);
    } catch (error) {
      const message = t("settings.feedback.loadFailed", { error: formatError(error) });
      setFeedback(message);
      onBanner(message);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [
    apiJson,
    onBanner,
    selectedSessionId,
    t,
    setManagedProviders,
    setConfigValidation,
    setMcpStatus,
    setMcpDrafts,
    setPluginAuthProviders,
    setPluginDrafts,
    setLspStatus,
    setFormatterStatus,
    setSkillCatalog,
    setManagedSkills,
    setSkillUsageLedger,
    setSkillNegativeEntropy,
    setSkillSemanticConflicts,
    setSkillSourceIndices,
    setSkillDistributions,
    setSkillArtifactCache,
    setSkillHubPolicy,
    setSkillLifecycle,
    setSkillGovernanceTimeline,
  ]);

  const runMutation = useCallback(
    async (key: string, action: () => Promise<string | void>, success: string) => {
      setBusyKey(key);
      setFeedback(null);
      try {
        const actionSuccess = await action();
        await Promise.all([reloadSettingsData(), onReloadCoreData()]);
        setFeedback(actionSuccess ?? success);
      } catch (error) {
        const message = formatError(error);
        setFeedback(message);
        onBanner(message);
      } finally {
        setBusyKey(null);
      }
    },
    [onBanner, onReloadCoreData, reloadSettingsData],
  );

  useEffect(() => {
    void reloadSettingsData();
  }, [reloadSettingsData]);

  const jumpToValidationTarget = useCallback(
    (item: ConfigPolicyValidationItemRecord) => {
      const target = validationJumpTarget(item);
      if (!target) {
        return;
      }
      if (target.tab === "providers" && target.providerId) {
        setSelectedManagedProviderId(target.providerId);
      }
      setActiveTab(target.tab);
    },
    [setSelectedManagedProviderId],
  );

  const providersActions = useProvidersSettingsActions({
    ...providersState,
    activeTab,
    api,
    apiJson,
    onBanner,
    onReloadCoreData,
    setFeedback,
    setBusyKey,
    reloadSettingsData,
    runMutation,
  });

  const memoryActions = useMemorySettingsActions({
    ...memoryState,
    activeTab,
    apiJson,
    onBanner,
    setFeedback,
    selectedSessionId,
  });

  const skillsActions = useSkillHubSettingsActions({
    ...skillsState,
    apiJson,
    onBanner,
    setFeedback,
    setBusyKey,
    selectedSessionId,
    reloadSettingsData,
    runMutation,
  });

  const configTabMutations = useConfigTabSettingsMutations({
    api,
    runMutation,
  });

  const generalTabProps: GeneralTabProps = {
    theme,
    themes,
    onThemeChange,
    modeOptions,
    selectedMode,
    onModeChange,
    modelOptions,
    selectedModel,
    onModelChange,
    showThinking,
    onShowThinkingChange,
    workspaceMode,
    workspaceRootPath,
    workspaceConfigDir,
    providerSummary,
    mcpConfigs,
    pluginConfigs,
    styles: {
      summaryCardClass: SETTINGS_DRAWER_STYLES.summaryCardClass,
      formFieldClass: SETTINGS_DRAWER_STYLES.formFieldClass,
      formLabelClass: SETTINGS_DRAWER_STYLES.formLabelClass,
      selectClass: SETTINGS_DRAWER_STYLES.selectClass,
      checkboxRowClass: SETTINGS_DRAWER_STYLES.checkboxRowClass,
      checkboxClass: SETTINGS_DRAWER_STYLES.checkboxClass,
    },
  };

  const memoryTabProps: MemoryTabProps = {
    selectedSessionId,
    styles: {
      primaryButtonClass: SETTINGS_DRAWER_STYLES.primaryButtonClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
      summaryCardClass: SETTINGS_DRAWER_STYLES.summaryCardClass,
      sectionCardClass: SETTINGS_DRAWER_STYLES.sectionCardClass,
      mutedCardClass: SETTINGS_DRAWER_STYLES.mutedCardClass,
      insetCardClass: SETTINGS_DRAWER_STYLES.insetCardClass,
      disclosureCardClass: SETTINGS_DRAWER_STYLES.disclosureCardClass,
    },
    memorySearchDraft: memoryState.memorySearchDraft,
    onMemorySearchDraftChange: memoryState.setMemorySearchDraft,
    memoryListLoading: memoryState.memoryListLoading,
    onLoadMemoryList: () => void memoryActions.loadMemoryList(),
    memoryPreviewLoading: memoryState.memoryPreviewLoading,
    onLoadMemoryPreview: () => void memoryActions.loadMemoryPreview(),
    memoryGovernanceLoading: memoryState.memoryGovernanceLoading,
    onLoadMemoryGovernance: () => void memoryActions.loadMemoryGovernance(),
    memoryConsolidateIncludeCandidates: memoryState.memoryConsolidateIncludeCandidates,
    onMemoryConsolidateIncludeCandidatesChange: memoryState.setMemoryConsolidateIncludeCandidates,
    memoryConsolidating: memoryState.memoryConsolidating,
    onRunMemoryConsolidation: () => void memoryActions.runMemoryConsolidation(),
    memoryListResponse: memoryState.memoryListResponse,
    selectedMemoryId: memoryState.selectedMemoryId,
    onSelectMemoryId: memoryState.setSelectedMemoryId,
    memoryDetailLoading: memoryState.memoryDetailLoading,
    memoryDetail: memoryState.memoryDetail,
    memoryValidationReport: memoryState.memoryValidationReport,
    memoryConflicts: memoryState.memoryConflicts,
    memoryPreview: memoryState.memoryPreview,
    memoryRulePacks: memoryState.memoryRulePacks,
    memoryRuleHits: memoryState.memoryRuleHits,
    memoryConsolidationRuns: memoryState.memoryConsolidationRuns,
    memoryConsolidationResult: memoryState.memoryConsolidationResult,
  };

  const providersTabProps: ProvidersTabProps = {
    styles: {
      primaryButtonClass: SETTINGS_DRAWER_STYLES.primaryButtonClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
      formFieldClass: SETTINGS_DRAWER_STYLES.formFieldClass,
      formLabelClass: SETTINGS_DRAWER_STYLES.formLabelClass,
      formHintClass: SETTINGS_DRAWER_STYLES.formHintClass,
      inputClass: SETTINGS_DRAWER_STYLES.inputClass,
      selectClass: SETTINGS_DRAWER_STYLES.selectClass,
      checkboxRowClass: SETTINGS_DRAWER_STYLES.checkboxRowClass,
      checkboxClass: SETTINGS_DRAWER_STYLES.checkboxClass,
    },
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
    onReloadSettingsData: reloadSettingsData,
    onRemoveProvider: (providerId) => void providersActions.removeProvider(providerId),
    onToggleProviderDisabled: (providerId, disabled) =>
      void providersActions.setProviderDisabled(providerId, disabled),
    onRenameProvider: (providerId, name) => void providersActions.renameProvider(providerId, name),
    onRefreshProviderCatalogue: () => void providersActions.refreshProviderCatalogue(),
    managedProviders: providersState.managedProviders,
    selectedManagedProviderId: providersState.selectedManagedProviderId,
    onSelectedManagedProviderIdChange: providersState.setSelectedManagedProviderId,
    providerDescriptorLoading: providersState.providerDescriptorLoading,
    selectedProviderDescriptor: providersState.selectedProviderDescriptor,
    selectedProviderDescriptorError: providersState.selectedProviderDescriptorError,
    modelOverrideDraft: providersState.modelOverrideDraft,
    onModelOverrideDraftChange: providersState.setModelOverrideDraft,
    editingModelTarget: providersState.editingModelTarget,
    modelOverrideProviderOptions: providersState.modelOverrideProviderOptions,
    configuredModelOverrides: providersState.configuredModelOverrides,
    onResetModelOverrideDraft: providersActions.resetModelOverrideDraft,
    onEditModelOverride: providersActions.editModelOverride,
    onSaveModelOverride: () => void providersActions.saveModelOverride(),
    onDeleteModelOverride: (providerId, modelKey) =>
      void providersActions.deleteModelOverride(providerId, modelKey),
  };

  const validationTabProps: ValidationTabProps = {
    styles: {
      summaryCardClass: SETTINGS_DRAWER_STYLES.summaryCardClass,
      mutedCardClass: SETTINGS_DRAWER_STYLES.mutedCardClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
    },
    configValidation: configTabsState.configValidation,
    validationReports: configTabsState.validationReports,
    validationGroups: configTabsState.validationGroups,
    validationErrorCount: configTabsState.validationErrorCount,
    validationWarningCount: configTabsState.validationWarningCount,
    onJumpToValidationTarget: jumpToValidationTarget,
  };

  const skillsTabProps: SkillsTabProps = {
    workspaceRootPath,
    selectedSessionId,
    skillWorkspaceRoot: skillsState.skillWorkspaceRoot,
    skillsMutationsEnabled: skillsState.skillsMutationsEnabled,
    styles: {
      primaryButtonClass: SETTINGS_DRAWER_STYLES.primaryButtonClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
      summaryCardClass: SETTINGS_DRAWER_STYLES.summaryCardClass,
      sectionCardClass: SETTINGS_DRAWER_STYLES.sectionCardClass,
      mutedCardClass: SETTINGS_DRAWER_STYLES.mutedCardClass,
      editorTextareaClass: SETTINGS_DRAWER_STYLES.editorTextareaClass,
    },
    busyKey,
    skillCatalog: skillsState.skillCatalog,
    managedSkills: skillsState.managedSkills,
    skillUsageLedger: skillsState.skillUsageLedger,
    skillNegativeEntropy: skillsState.skillNegativeEntropy,
    skillSemanticConflicts: skillsState.skillSemanticConflicts,
    skillSourceIndices: skillsState.skillSourceIndices,
    skillDistributions: skillsState.skillDistributions,
    skillArtifactCache: skillsState.skillArtifactCache,
    skillHubPolicy: skillsState.skillHubPolicy,
    skillLifecycle: skillsState.skillLifecycle,
    skillGovernanceTimeline: skillsState.skillGovernanceTimeline,
    skillSyncSourceId: skillsState.skillSyncSourceId,
    onSkillSyncSourceIdChange: skillsState.setSkillSyncSourceId,
    skillSyncSourceKind: skillsState.skillSyncSourceKind,
    onSkillSyncSourceKindChange: skillsState.setSkillSyncSourceKind,
    skillSyncLocator: skillsState.skillSyncLocator,
    onSkillSyncLocatorChange: skillsState.setSkillSyncLocator,
    skillSyncRevision: skillsState.skillSyncRevision,
    onSkillSyncRevisionChange: skillsState.setSkillSyncRevision,
    skillSyncPlan: skillsState.skillSyncPlan,
    onPlanSkillSync: () => void skillsActions.planSkillSync(),
    onApplySkillSync: () => void skillsActions.applySkillSync(),
    onRefreshSkillSourceIndex: () => void skillsActions.refreshSkillSourceIndex(),
    onRunSelectedSourceGuard: () => void skillsActions.runSelectedSourceGuard(),
    remoteInstallSkillName: skillsState.remoteInstallSkillName,
    onRemoteInstallSkillNameChange: skillsState.setRemoteInstallSkillName,
    remoteInstallPlan: skillsState.remoteInstallPlan,
    onPlanRemoteInstall: () => void skillsActions.planRemoteInstall(),
    onPlanRemoteUpdate: () => void skillsActions.planRemoteUpdate(),
    onApplyRemoteInstall: () => void skillsActions.applyRemoteInstall(),
    onApplyRemoteUpdate: () => void skillsActions.applyRemoteUpdate(),
    onDetachManagedSkill: () => void skillsActions.detachManagedSkill(),
    onRemoveManagedSkill: () => void skillsActions.removeManagedSkill(),
    skillGuardReports: skillsState.skillGuardReports,
    skillGuardTarget: skillsState.skillGuardTarget,
    selectedSkillName: skillsState.selectedSkillName,
    onSelectedSkillNameChange: skillsState.setSelectedSkillName,
    selectedSkillEntry: skillsState.selectedSkillEntry,
    skillDetail: skillsState.skillDetail,
    skillDetailLoading: skillsState.skillDetailLoading,
    skillEditorContent: skillsState.skillEditorContent,
    onSkillEditorContentChange: skillsState.setSkillEditorContent,
    editSkillEditorMode: skillsState.editSkillEditorMode,
    onEditSkillEditorModeChange: skillsState.setEditSkillEditorMode,
    editSkillDescription: skillsState.editSkillDescription,
    onEditSkillDescriptionChange: skillsState.setEditSkillDescription,
    editSkillMethodologyDraft: skillsState.editSkillMethodologyDraft,
    onEditSkillMethodologyDraftChange: skillsState.setEditSkillMethodologyDraft,
    editSkillMethodologyMatched: skillsState.editSkillMethodologyMatched,
    editSkillMethodologyPreview: skillsState.editSkillMethodologyPreview,
    editSkillMethodologyPreviewError: skillsState.editSkillMethodologyPreviewError,
    newSkillName: skillsState.newSkillName,
    onNewSkillNameChange: skillsState.setNewSkillName,
    newSkillDescription: skillsState.newSkillDescription,
    onNewSkillDescriptionChange: skillsState.setNewSkillDescription,
    newSkillCategory: skillsState.newSkillCategory,
    onNewSkillCategoryChange: skillsState.setNewSkillCategory,
    newSkillBody: skillsState.newSkillBody,
    onNewSkillBodyChange: skillsState.setNewSkillBody,
    newSkillEditorMode: skillsState.newSkillEditorMode,
    onNewSkillEditorModeChange: skillsState.setNewSkillEditorMode,
    newSkillMethodologyDraft: skillsState.newSkillMethodologyDraft,
    onNewSkillMethodologyDraftChange: skillsState.setNewSkillMethodologyDraft,
    newSkillMethodologyPreview: skillsState.newSkillMethodologyPreview,
    newSkillMethodologyPreviewError: skillsState.newSkillMethodologyPreviewError,
    onCreateSkill: () => void skillsActions.createSkill(),
    onRunSelectedSkillGuard: () => void skillsActions.runSelectedSkillGuard(),
    onSaveSelectedSkill: () => void skillsActions.saveSelectedSkill(),
    onDeleteSelectedSkill: () => void skillsActions.deleteSelectedSkill(),
    managedRecordBySkill: skillsState.managedRecordBySkill,
    latestGuardBySkill: skillsState.latestGuardBySkill,
    selectedHubSourceSnapshot: skillsState.selectedHubSourceSnapshot,
    selectedRemoteSourceEntry: skillsState.selectedRemoteSourceEntry,
    selectedRemoteDistribution: skillsState.selectedRemoteDistribution,
    selectedRemoteArtifactCache: skillsState.selectedRemoteArtifactCache,
    selectedRemoteLifecycle: skillsState.selectedRemoteLifecycle,
  };

  const mcpTabProps: McpTabProps = {
    styles: {
      primaryButtonClass: SETTINGS_DRAWER_STYLES.primaryButtonClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
      formLabelClass: SETTINGS_DRAWER_STYLES.formLabelClass,
      inputClass: SETTINGS_DRAWER_STYLES.inputClass,
      editorTextareaClass: SETTINGS_DRAWER_STYLES.editorTextareaClass,
    },
    busyKey,
    mcpStatus: configTabsState.mcpStatus,
    mcpConfigs,
    mcpDrafts: configTabsState.mcpDrafts,
    onMcpDraftsChange: configTabsState.setMcpDrafts,
    newMcpKey: configTabsState.newMcpKey,
    onNewMcpKeyChange: configTabsState.setNewMcpKey,
    newMcpDraft: configTabsState.newMcpDraft,
    onNewMcpDraftChange: configTabsState.setNewMcpDraft,
    onSaveMcpConfig: (key, raw) => void configTabMutations.saveMcpConfig(key, raw),
    onDeleteMcpConfig: (key) => void configTabMutations.deleteMcpConfig(key),
    onRunMcpAction: (name, action) => void configTabMutations.runMcpAction(name, action),
  };

  const pluginsTabProps: PluginsTabProps = {
    styles: {
      primaryButtonClass: SETTINGS_DRAWER_STYLES.primaryButtonClass,
      secondaryButtonClass: SETTINGS_DRAWER_STYLES.secondaryButtonClass,
      formLabelClass: SETTINGS_DRAWER_STYLES.formLabelClass,
      inputClass: SETTINGS_DRAWER_STYLES.inputClass,
      editorTextareaClass: SETTINGS_DRAWER_STYLES.editorTextareaClass,
    },
    busyKey,
    pluginAuthProviders: configTabsState.pluginAuthProviders,
    pluginConfigs,
    pluginDrafts: configTabsState.pluginDrafts,
    onPluginDraftsChange: configTabsState.setPluginDrafts,
    newPluginKey: configTabsState.newPluginKey,
    onNewPluginKeyChange: configTabsState.setNewPluginKey,
    newPluginDraft: configTabsState.newPluginDraft,
    onNewPluginDraftChange: configTabsState.setNewPluginDraft,
    onSavePluginConfig: (key, raw) => void configTabMutations.savePluginConfig(key, raw),
    onDeletePluginConfig: (key) => void configTabMutations.deletePluginConfig(key),
  };

  const lspTabProps: LspTabProps = {
    lspStatus: configTabsState.lspStatus,
    formatterStatus: configTabsState.formatterStatus,
  };

  return {
    activeTab,
    onActiveTabChange: setActiveTab,
    loading,
    refreshing,
    feedback,
    isolatedNotice,
    reloadSettingsData,
    generalTabProps,
    memoryTabProps,
    providersTabProps,
    validationTabProps,
    skillsTabProps,
    mcpTabProps,
    pluginsTabProps,
    lspTabProps,
  };
}
