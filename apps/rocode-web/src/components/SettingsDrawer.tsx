import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  ConfigPolicyValidationItemRecord,
  ConfigPolicyValidationOwnerRecord,
  ConfigPolicyValidationSnapshotRecord,
} from "@/lib/configPolicy";
import type {
  MemoryConsolidationResponseRecord,
  MemoryConsolidationRunListResponseRecord,
  MemoryConflictResponseRecord,
  MemoryDetailResponseRecord,
  MemoryListResponseRecord,
  MemoryRetrievalPreviewResponseRecord,
  MemoryRuleHitListResponseRecord,
  MemoryRulePackListResponseRecord,
  MemoryValidationReportResponseRecord,
} from "@/lib/memory";
import { memoryRecordIdValue } from "@/lib/memory";
import type {
  ManagedSkillRecord,
  SkillArtifactCacheEntryRecord,
  SkillCatalogEntry,
  SkillDetailResponseRecord,
  SkillDistributionRecord,
  SkillGovernanceTimelineEntryRecord,
  SkillGuardReportRecord,
  SkillHubArtifactCacheResponseRecord,
  SkillHubAuditResponseRecord,
  SkillHubDistributionResponseRecord,
  SkillHubGuardRunRequestRecord,
  SkillHubGuardRunResponseRecord,
  SkillHubIndexRefreshResponseRecord,
  SkillHubIndexResponseRecord,
  SkillHubLifecycleResponseRecord,
  SkillHubManagedDetachResponseRecord,
  SkillHubManagedRemoveResponseRecord,
  SkillHubManagedResponseRecord,
  SkillHubPolicyRecord,
  SkillHubPolicyResponseRecord,
  SkillHubSyncPlanResponseRecord,
  SkillHubTimelineResponseRecord,
  SkillManagedLifecycleRecord,
  SkillManageResponseRecord,
  SkillMethodologyExtractResponseRecord,
  SkillMethodologyPreviewResponseRecord,
  SkillMethodologyTemplateRecord,
  SkillRemoteInstallPlanRecord,
  SkillRemoteInstallResponseRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
  SkillSyncPlanRecord,
} from "@/lib/skill";
import { cn } from "@/lib/utils";
import type {
  ConnectProtocolOption,
  KnownProviderEntry,
  ManagedModelOverrideInfoRecord,
  ManagedProviderInfoRecord,
  ProviderConnectionDescriptorCandidateRecord,
  ProviderConnectDraft,
  ProviderDescriptorResponseRecord,
  ProviderRecord,
  ProviderProfileDescriptorViewRecord,
  RefreshProviderCatalogueResponseRecord,
  ResolveProviderConnectResponseRecord,
} from "@/lib/provider";
import { displayProtocolLabel } from "@/lib/provider";
import { MemoryTab } from "./settings-drawer/MemoryTab";
import { SkillsTab } from "./settings-drawer/SkillsTab";
import {
  buildMethodologyTemplateFromDraft,
  emptySkillMethodologyDraft,
  methodologyDraftFromTemplate,
  SkillMethodologyDraft,
} from "./SkillMethodologyEditor";

type SettingsTabId =
  | "general"
  | "memory"
  | "providers"
  | "scheduler"
  | "validation"
  | "skills"
  | "mcp"
  | "plugins"
  | "lsp";

interface ThemeOption {
  id: string;
  label: string;
}

interface ModeOption {
  key: string;
  label: string;
}

interface ModelOption {
  key: string;
  label: string;
}

interface SchedulerProfileSummary {
  key: string;
  orchestrator?: string | null;
  description?: string | null;
  stages: string[];
}

interface SchedulerConfigResponse {
  raw_path?: string | null;
  resolved_path?: string | null;
  exists: boolean;
  content: string;
  default_profile?: string | null;
  profiles: SchedulerProfileSummary[];
  parse_error?: string | null;
}

interface McpStatusInfo {
  name: string;
  status: string;
  tools: number;
  resources: number;
  error?: string | null;
}

interface PluginAuthProviderInfo {
  provider: string;
  methods: Array<{ type?: string; label?: string }>;
}

interface LspStatus {
  servers: string[];
}

interface FormatterStatus {
  formatters: string[];
}

type SkillEditorMode = "methodology" | "raw";

interface AppConfigSnapshot extends Record<string, unknown> {
  provider?: Record<string, unknown>;
  plugin?: Record<string, unknown>;
  mcp?: Record<string, unknown>;
  schedulerPath?: string | null;
}

interface ModelOverrideDraft {
  providerId: string;
  modelKey: string;
  modelId: string;
  name: string;
  baseUrl: string;
  family: string;
  status: string;
  releaseDate: string;
  reasoning: boolean;
  toolCall: boolean;
  attachment: boolean;
  temperature: boolean;
  experimental: boolean;
}

interface SettingsDrawerProps {
  onClose: () => void;
  theme: string;
  themes: ThemeOption[];
  onThemeChange: (themeId: string) => void;
  workspaceMode: "shared" | "isolated" | null;
  workspaceRootPath: string;
  workspaceConfigDir?: string | null;
  selectedSessionId: string | null;
  modeOptions: ModeOption[];
  selectedMode: string;
  onModeChange: (mode: string) => void;
  modelOptions: ModelOption[];
  selectedModel: string;
  onModelChange: (model: string) => void;
  showThinking: boolean;
  onShowThinkingChange: (value: boolean) => void;
  providers: ProviderRecord[];
  knownProviders: KnownProviderEntry[];
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
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onBanner: (message: string) => void;
  onReloadCoreData: () => Promise<void>;
}

const SETTINGS_TABS: Array<{ id: SettingsTabId; label: string }> = [
  { id: "general", label: "General" },
  { id: "memory", label: "Memory" },
  { id: "providers", label: "Providers" },
  { id: "scheduler", label: "Scheduler" },
  { id: "validation", label: "Validation" },
  { id: "skills", label: "Skills" },
  { id: "mcp", label: "MCP" },
  { id: "plugins", label: "Plugins" },
  { id: "lsp", label: "LSP" },
];

function isolatedWorkspaceNotice(tab: SettingsTabId): string | null {
  switch (tab) {
    case "general":
      return "These settings are still persisted to global config. In isolated mode, that persisted global copy does not become the current sandbox runtime unless you switch back to a shared workspace.";
    case "providers":
      return "Provider and model changes made here target global config or shared provider state. The current isolated sandbox will not inherit those global config changes unless the same intent is expressed inside this workspace's .rocode.";
    case "memory":
      return "Memory reads here come from the current workspace authority. In isolated mode, that means you are inspecting sandbox-local memory state rather than inherited global memory records.";
    case "scheduler":
      return "Scheduler edits here write global config. The active isolated sandbox will continue resolving scheduler behavior from its local workspace authority until you switch to shared mode or add matching workspace-local config.";
    case "validation":
      return "Validation here explains the persisted config snapshot behind this settings drawer. In isolated mode, the current sandbox runtime can still differ until matching workspace-local authority exists.";
    case "skills":
      return "Skill mutations here always target this workspace's .rocode/skills. In isolated mode, that means the current sandbox stays local and does not inherit or modify global skill config.";
    case "mcp":
      return "MCP config saved here is global. An isolated workspace does not automatically inherit that global config into its current sandbox runtime.";
    case "plugins":
      return "Plugin config saved here is global. The current isolated sandbox will not inherit those global config changes unless they are mirrored into this workspace's .rocode authority.";
    default:
      return null;
  }
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error ?? "Unknown error");
}

function arrayOrEmpty<T>(value: T[] | null | undefined): T[] {
  return Array.isArray(value) ? value : [];
}

function stringifyJson(value: unknown) {
  return JSON.stringify(value ?? {}, null, 2);
}

function objectRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  return value as Record<string, unknown>;
}

function parseObjectJson(label: string, raw: string) {
  const trimmed = raw.trim();
  if (!trimmed) {
    throw new Error(`${label} JSON cannot be empty`);
  }
  const parsed = JSON.parse(trimmed) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${label} JSON must be an object`);
  }
  return parsed as Record<string, unknown>;
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

function formatDateTime(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts).toLocaleString();
}

function validationSeverityTone(severity: string | null | undefined) {
  switch ((severity || "").toLowerCase()) {
    case "error":
      return "danger";
    case "warning":
      return "warn";
    default:
      return "muted";
  }
}

function validationOwnerLabel(owner: ConfigPolicyValidationOwnerRecord) {
  switch (owner) {
    case "scheduler":
      return "Scheduler";
    case "skill_tree":
      return "Skill Tree";
    case "provider_profile":
      return "Provider Profile";
    case "external_adapter":
      return "External Adapter";
  }
}

function validationEffectLabel(effect: string | null | undefined) {
  switch ((effect || "").toLowerCase()) {
    case "soft_fallback":
      return "soft fallback";
    case "fail_closed_bootstrap":
      return "fail-closed bootstrap";
    case "fail_closed_request_gate":
      return "fail-closed request gate";
    default:
      return effect || "--";
  }
}

function validationScopeLabel(scopeKind: string | null | undefined) {
  switch ((scopeKind || "").toLowerCase()) {
    case "scheduler_path":
      return "scheduler path";
    case "skill_tree":
      return "skill tree";
    case "provider":
      return "provider";
    case "external_adapter":
      return "external adapter";
    default:
      return scopeKind || "--";
  }
}

function validationJumpTarget(
  item: ConfigPolicyValidationItemRecord,
):
  | {
      tab: Extract<SettingsTabId, "providers" | "scheduler">;
      label: string;
      providerId?: string;
    }
  | null {
  if (item.owner === "provider_profile") {
    return {
      tab: "providers",
      label: "Open Providers",
      providerId: item.scope.subject_id ?? undefined,
    };
  }
  if (item.owner === "scheduler") {
    return {
      tab: "scheduler",
      label: "Open Scheduler",
    };
  }
  return null;
}

function emptyModelOverrideDraft(providerId = ""): ModelOverrideDraft {
  return {
    providerId,
    modelKey: "",
    modelId: "",
    name: "",
    baseUrl: "",
    family: "",
    status: "",
    releaseDate: "",
    reasoning: false,
    toolCall: false,
    attachment: false,
    temperature: false,
    experimental: false,
  };
}

function modelOverrideDraftFromRecord(
  providerId: string,
  record: ManagedModelOverrideInfoRecord,
): ModelOverrideDraft {
  return {
    providerId,
    modelKey: record.key,
    modelId: record.model ?? "",
    name: record.name ?? "",
    baseUrl: record.base_url ?? "",
    family: record.family ?? "",
    status: record.status ?? "",
    releaseDate: record.release_date ?? "",
    reasoning: Boolean(record.reasoning),
    toolCall: Boolean(record.tool_call),
    attachment: Boolean(record.attachment),
    temperature: Boolean(record.temperature),
    experimental: Boolean(record.experimental),
  };
}

export function SettingsDrawer({
  onClose,
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
}: SettingsDrawerProps) {
  const [activeTab, setActiveTab] = useState<SettingsTabId>("general");
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [configSnapshot, setConfigSnapshot] = useState<AppConfigSnapshot | null>(null);
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
  const [schedulerConfig, setSchedulerConfig] = useState<SchedulerConfigResponse | null>(null);
  const [schedulerPathDraft, setSchedulerPathDraft] = useState("");
  const [schedulerContentDraft, setSchedulerContentDraft] = useState("");
  const [configValidation, setConfigValidation] =
    useState<ConfigPolicyValidationSnapshotRecord | null>(null);
  const [memorySearchDraft, setMemorySearchDraft] = useState("");
  const [memoryListLoading, setMemoryListLoading] = useState(false);
  const [memoryListResponse, setMemoryListResponse] = useState<MemoryListResponseRecord | null>(null);
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);
  const [memoryDetailLoading, setMemoryDetailLoading] = useState(false);
  const [memoryDetail, setMemoryDetail] = useState<MemoryDetailResponseRecord | null>(null);
  const [memoryValidationReport, setMemoryValidationReport] =
    useState<MemoryValidationReportResponseRecord | null>(null);
  const [memoryConflicts, setMemoryConflicts] = useState<MemoryConflictResponseRecord | null>(null);
  const [memoryPreviewLoading, setMemoryPreviewLoading] = useState(false);
  const [memoryPreview, setMemoryPreview] = useState<MemoryRetrievalPreviewResponseRecord | null>(null);
  const [memoryRulePacks, setMemoryRulePacks] = useState<MemoryRulePackListResponseRecord | null>(null);
  const [memoryRuleHits, setMemoryRuleHits] = useState<MemoryRuleHitListResponseRecord | null>(null);
  const [memoryConsolidationRuns, setMemoryConsolidationRuns] =
    useState<MemoryConsolidationRunListResponseRecord | null>(null);
  const [memoryConsolidationResult, setMemoryConsolidationResult] =
    useState<MemoryConsolidationResponseRecord | null>(null);
  const [memoryGovernanceLoading, setMemoryGovernanceLoading] = useState(false);
  const [memoryConsolidating, setMemoryConsolidating] = useState(false);
  const [memoryConsolidateIncludeCandidates, setMemoryConsolidateIncludeCandidates] =
    useState(false);
  const [mcpStatus, setMcpStatus] = useState<Record<string, McpStatusInfo>>({});
  const [mcpDrafts, setMcpDrafts] = useState<Record<string, string>>({});
  const [newMcpKey, setNewMcpKey] = useState("");
  const [newMcpDraft, setNewMcpDraft] = useState("{\n  \"type\": \"local\",\n  \"command\": \"\"\n}");
  const [pluginAuthProviders, setPluginAuthProviders] = useState<PluginAuthProviderInfo[]>([]);
  const [pluginDrafts, setPluginDrafts] = useState<Record<string, string>>({});
  const [newPluginKey, setNewPluginKey] = useState("");
  const [newPluginDraft, setNewPluginDraft] = useState("{\n  \"command\": \"\",\n  \"args\": []\n}");
  const [lspStatus, setLspStatus] = useState<LspStatus | null>(null);
  const [formatterStatus, setFormatterStatus] = useState<FormatterStatus | null>(null);
  const [skillCatalog, setSkillCatalog] = useState<SkillCatalogEntry[]>([]);
  const [managedSkills, setManagedSkills] = useState<ManagedSkillRecord[]>([]);
  const [skillSourceIndices, setSkillSourceIndices] = useState<SkillSourceIndexSnapshotRecord[]>([]);
  const [skillDistributions, setSkillDistributions] = useState<SkillDistributionRecord[]>([]);
  const [skillArtifactCache, setSkillArtifactCache] = useState<SkillArtifactCacheEntryRecord[]>([]);
  const [skillHubPolicy, setSkillHubPolicy] = useState<SkillHubPolicyRecord | null>(null);
  const [skillLifecycle, setSkillLifecycle] = useState<SkillManagedLifecycleRecord[]>([]);
  const [skillGovernanceTimeline, setSkillGovernanceTimeline] = useState<SkillGovernanceTimelineEntryRecord[]>([]);
  const [skillSyncSourceId, setSkillSyncSourceId] = useState("");
  const [skillSyncSourceKind, setSkillSyncSourceKind] = useState<SkillSourceRefRecord["source_kind"]>("local_path");
  const [skillSyncLocator, setSkillSyncLocator] = useState("");
  const [skillSyncRevision, setSkillSyncRevision] = useState("");
  const [skillSyncPlan, setSkillSyncPlan] = useState<SkillSyncPlanRecord | null>(null);
  const [remoteInstallSkillName, setRemoteInstallSkillName] = useState("");
  const [remoteInstallPlan, setRemoteInstallPlan] = useState<SkillRemoteInstallPlanRecord | null>(null);
  const [skillGuardReports, setSkillGuardReports] = useState<SkillGuardReportRecord[]>([]);
  const [skillGuardTarget, setSkillGuardTarget] = useState<string | null>(null);
  const [selectedSkillName, setSelectedSkillName] = useState<string | null>(null);
  const [skillDetail, setSkillDetail] = useState<SkillDetailResponseRecord | null>(null);
  const [skillDetailLoading, setSkillDetailLoading] = useState(false);
  const [skillEditorContent, setSkillEditorContent] = useState("");
  const [editSkillEditorMode, setEditSkillEditorMode] = useState<SkillEditorMode>("raw");
  const [editSkillDescription, setEditSkillDescription] = useState("");
  const [editSkillMethodologyDraft, setEditSkillMethodologyDraft] =
    useState<SkillMethodologyDraft>(emptySkillMethodologyDraft);
  const [editSkillMethodologyMatched, setEditSkillMethodologyMatched] = useState(false);
  const [editSkillMethodologyPreview, setEditSkillMethodologyPreview] = useState("");
  const [editSkillMethodologyPreviewError, setEditSkillMethodologyPreviewError] =
    useState<string | null>(null);
  const [newSkillName, setNewSkillName] = useState("");
  const [newSkillDescription, setNewSkillDescription] = useState("");
  const [newSkillCategory, setNewSkillCategory] = useState("");
  const [newSkillBody, setNewSkillBody] = useState("");
  const [newSkillEditorMode, setNewSkillEditorMode] = useState<SkillEditorMode>("methodology");
  const [newSkillMethodologyDraft, setNewSkillMethodologyDraft] =
    useState<SkillMethodologyDraft>(emptySkillMethodologyDraft);
  const [newSkillMethodologyPreview, setNewSkillMethodologyPreview] = useState("");
  const [newSkillMethodologyPreviewError, setNewSkillMethodologyPreviewError] =
    useState<string | null>(null);

  const mcpConfigs = useMemo(
    () => objectRecord(configSnapshot?.mcp),
    [configSnapshot?.mcp],
  );
  const pluginConfigs = useMemo(
    () => objectRecord(configSnapshot?.plugin),
    [configSnapshot?.plugin],
  );
  const isolatedNotice = workspaceMode === "isolated" ? isolatedWorkspaceNotice(activeTab) : null;
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
  const selectedSkillEntry = useMemo(
    () =>
      skillCatalog.find(
        (skill) =>
          skill.name.trim().toLowerCase() === (selectedSkillName ?? "").trim().toLowerCase(),
      ) ?? null,
    [selectedSkillName, skillCatalog],
  );
  const managedRecordBySkill = useMemo(
    () =>
      new Map(
        managedSkills.map((record) => [record.skill_name.trim().toLowerCase(), record] as const),
      ),
    [managedSkills],
  );
  const selectedHubSourceSnapshot = useMemo(
    () =>
      skillSourceIndices.find(
        (snapshot) =>
          snapshot.source.source_id.trim().toLowerCase() ===
          skillSyncSourceId.trim().toLowerCase(),
      ) ?? null,
    [skillSourceIndices, skillSyncSourceId],
  );
  const selectedRemoteSourceEntries = useMemo(
    () => selectedHubSourceSnapshot?.entries ?? [],
    [selectedHubSourceSnapshot],
  );
  const selectedRemoteSourceEntry = useMemo(
    () =>
      selectedRemoteSourceEntries.find(
        (entry) =>
          entry.skill_name.trim().toLowerCase() ===
          remoteInstallSkillName.trim().toLowerCase(),
      ) ?? null,
    [remoteInstallSkillName, selectedRemoteSourceEntries],
  );
  const selectedRemoteDistribution = useMemo(() => {
    const matches = skillDistributions
      .filter(
        (record) =>
          record.source.source_id.trim().toLowerCase() ===
            skillSyncSourceId.trim().toLowerCase() &&
          record.skill_name.trim().toLowerCase() ===
            remoteInstallSkillName.trim().toLowerCase(),
      )
      .sort(
        (left, right) =>
          (right.resolution?.resolved_at ?? 0) - (left.resolution?.resolved_at ?? 0),
      );
    return matches[0] ?? null;
  }, [remoteInstallSkillName, skillDistributions, skillSyncSourceId]);
  const selectedRemoteLifecycle = useMemo(() => {
    if (selectedRemoteDistribution) {
      return (
        skillLifecycle.find(
          (record) => record.distribution_id === selectedRemoteDistribution.distribution_id,
        ) ?? null
      );
    }
    const matches = skillLifecycle
      .filter(
        (record) =>
          record.source_id.trim().toLowerCase() === skillSyncSourceId.trim().toLowerCase() &&
          record.skill_name.trim().toLowerCase() ===
            remoteInstallSkillName.trim().toLowerCase(),
      )
      .sort((left, right) => right.updated_at - left.updated_at);
    return matches[0] ?? null;
  }, [remoteInstallSkillName, selectedRemoteDistribution, skillLifecycle, skillSyncSourceId]);
  const selectedRemoteArtifactCache = useMemo(() => {
    if (!selectedRemoteDistribution) {
      return null;
    }
    return (
      skillArtifactCache.find(
        (entry) =>
          entry.artifact.artifact_id ===
          selectedRemoteDistribution.resolution.artifact.artifact_id,
      ) ?? null
    );
  }, [selectedRemoteDistribution, skillArtifactCache]);
  const latestGuardBySkill = useMemo(() => {
    const result = new Map<string, SkillGuardReportRecord>();
    for (const entry of skillGovernanceTimeline) {
      const key = entry.skill_name?.trim().toLowerCase();
      if (!key || result.has(key) || !entry.guard_report) {
        continue;
      }
      result.set(key, entry.guard_report);
    }
    return result;
  }, [skillGovernanceTimeline]);
  const skillWorkspaceRoot = useMemo(() => {
    const trimmed = workspaceRootPath.trim();
    if (!trimmed) return ".rocode/skills";
    return `${trimmed.replace(/\/+$/, "")}/.rocode/skills`;
  }, [workspaceRootPath]);
  const selectedMemoryCard = useMemo(
    () =>
      memoryListResponse?.items.find(
        (item) => memoryRecordIdValue(item.id) === (selectedMemoryId ?? ""),
      ) ?? null,
    [memoryListResponse?.items, selectedMemoryId],
  );
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
      { id: selectedProviderId, label: `${selectedProviderId} (custom)` },
      ...providerChoices.map((providerId) => ({
        id: providerId,
        label: providerId,
      })),
    ];
  }, [modelOverrideDraft.providerId, providerChoices]);
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
  const validationReports = configValidation?.reports ?? [];
  const validationGroups = useMemo(() => {
    const groups: Array<{
      owner: ConfigPolicyValidationOwnerRecord;
      items: ConfigPolicyValidationItemRecord[];
    }> = [];
    for (const item of validationReports) {
      const last = groups[groups.length - 1];
      if (last && last.owner === item.owner) {
        last.items.push(item);
      } else {
        groups.push({ owner: item.owner, items: [item] });
      }
    }
    return groups;
  }, [validationReports]);
  const validationErrorCount = useMemo(
    () => validationReports.filter((item) => item.severity === "error").length,
    [validationReports],
  );
  const validationWarningCount = useMemo(
    () => validationReports.filter((item) => item.severity === "warning").length,
    [validationReports],
  );
  const skillsMutationsEnabled = Boolean(selectedSessionId);
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
    [],
  );

  const reloadSettingsData = useCallback(async () => {
    setRefreshing(true);
    setFeedback(null);
    try {
      const skillCatalogPath = selectedSessionId
        ? `/skill/catalog?session_id=${encodeURIComponent(selectedSessionId)}`
        : "/skill/catalog";
      const [config, managed, scheduler, validation, mcp, plugins, lsp, formatter, skills, skillHubManaged, skillHubIndex, skillHubDistributions, skillHubArtifactCache, skillHubPolicyResponse, skillHubLifecycle, skillHubAudit, skillHubTimeline] =
        await Promise.all([
          apiJson<AppConfigSnapshot>("/config"),
          apiJson<{ providers: ManagedProviderInfoRecord[] }>("/provider/managed"),
          apiJson<SchedulerConfigResponse>("/config/scheduler"),
          apiJson<ConfigPolicyValidationSnapshotRecord>("/config/validation").catch(() => null),
          apiJson<Record<string, McpStatusInfo>>("/mcp"),
          apiJson<PluginAuthProviderInfo[]>("/plugin/auth").catch(() => []),
          apiJson<LspStatus>("/lsp"),
          apiJson<FormatterStatus>("/formatter"),
          apiJson<SkillCatalogEntry[]>(skillCatalogPath),
          apiJson<SkillHubManagedResponseRecord>("/skill/hub/managed"),
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
      setSchedulerConfig(scheduler);
      setSchedulerPathDraft(scheduler.raw_path ?? "");
      setSchedulerContentDraft(scheduler.content ?? "");
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
      setSkillSourceIndices(skillHubIndex.source_indices ?? []);
      setSkillDistributions(skillHubDistributions.distributions ?? []);
      setSkillArtifactCache(skillHubArtifactCache.artifact_cache ?? []);
      setSkillHubPolicy(skillHubPolicyResponse.policy ?? null);
      setSkillLifecycle(skillHubLifecycle.lifecycle ?? []);
      setSkillGovernanceTimeline(skillHubTimeline.entries ?? []);
    } catch (error) {
      const message = `Failed to load settings data: ${formatError(error)}`;
      setFeedback(message);
      onBanner(message);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [apiJson, onBanner, selectedSessionId]);

  const requestSkillMethodologyPreview = useCallback(
    async (
      skillName: string,
      draft: SkillMethodologyDraft,
      applyPreview: (body: string, error: string | null) => void,
    ) => {
      try {
        const response = await apiJson<SkillMethodologyPreviewResponseRecord>(
          "/skill/methodology/preview",
          {
            method: "POST",
            body: JSON.stringify({
              skill_name: skillName.trim() || "draft-skill",
              methodology: buildMethodologyTemplateFromDraft(draft),
            }),
          },
        );
        applyPreview(response.body, null);
      } catch (error) {
        applyPreview("", formatError(error));
      }
    },
    [apiJson],
  );

  const loadMemoryList = useCallback(async () => {
    setMemoryListLoading(true);
    try {
      const params = new URLSearchParams();
      if (memorySearchDraft.trim()) {
        params.set("search", memorySearchDraft.trim());
      }
      params.set("limit", "60");
      if (selectedSessionId) {
        params.set("source_session_id", selectedSessionId);
      }
      const route = memorySearchDraft.trim() ? "/memory/search" : "/memory/list";
      const path = `${route}${params.toString() ? `?${params.toString()}` : ""}`;
      const response = await apiJson<MemoryListResponseRecord>(path);
      response.items = arrayOrEmpty(response.items);
      if (response.contract) {
        response.contract.search_fields = arrayOrEmpty(response.contract.search_fields);
        response.contract.filter_query_parameters = arrayOrEmpty(
          response.contract.filter_query_parameters,
        );
        response.contract.non_search_fields = arrayOrEmpty(response.contract.non_search_fields);
      }
      setMemoryListResponse(response);
      setSelectedMemoryId((current) => {
        if (
          current &&
          response.items.some((item) => memoryRecordIdValue(item.id) === current)
        ) {
          return current;
        }
        return response.items[0] ? memoryRecordIdValue(response.items[0].id) : null;
      });
    } catch (error) {
      const message = `Failed to load memory list: ${formatError(error)}`;
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryListLoading(false);
    }
  }, [apiJson, memorySearchDraft, onBanner, selectedSessionId]);

  const loadMemoryPreview = useCallback(async () => {
    setMemoryPreviewLoading(true);
    try {
      const params = new URLSearchParams();
      if (memorySearchDraft.trim()) {
        params.set("query", memorySearchDraft.trim());
      }
      params.set("limit", "6");
      if (selectedSessionId) {
        params.set("session_id", selectedSessionId);
      }
      const path = `/memory/retrieval-preview?${params.toString()}`;
      const response = await apiJson<MemoryRetrievalPreviewResponseRecord>(path);
      response.packet.items = arrayOrEmpty(response.packet.items);
      response.packet.scopes = arrayOrEmpty(response.packet.scopes);
      if (response.contract) {
        response.contract.search_fields = arrayOrEmpty(response.contract.search_fields);
        response.contract.filter_query_parameters = arrayOrEmpty(
          response.contract.filter_query_parameters,
        );
        response.contract.non_search_fields = arrayOrEmpty(response.contract.non_search_fields);
      }
      setMemoryPreview(response);
    } catch (error) {
      const message = `Failed to load memory retrieval preview: ${formatError(error)}`;
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryPreviewLoading(false);
    }
  }, [apiJson, memorySearchDraft, onBanner, selectedSessionId]);

  const loadMemoryGovernance = useCallback(async () => {
    setMemoryGovernanceLoading(true);
    try {
      const [rulePacks, ruleHits, runs] = await Promise.all([
        apiJson<MemoryRulePackListResponseRecord>("/memory/rule-packs"),
        apiJson<MemoryRuleHitListResponseRecord>("/memory/rule-hits?limit=30"),
        apiJson<MemoryConsolidationRunListResponseRecord>("/memory/consolidation/runs?limit=20"),
      ]);
      rulePacks.items = arrayOrEmpty(rulePacks.items).map((pack) => ({
        ...pack,
        rules: arrayOrEmpty(pack.rules),
      }));
      ruleHits.items = arrayOrEmpty(ruleHits.items);
      runs.items = arrayOrEmpty(runs.items);
      setMemoryRulePacks(rulePacks);
      setMemoryRuleHits(ruleHits);
      setMemoryConsolidationRuns(runs);
    } catch (error) {
      const message = `Failed to load memory governance state: ${formatError(error)}`;
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryGovernanceLoading(false);
    }
  }, [apiJson, onBanner]);

  const runMemoryConsolidation = useCallback(async () => {
    setMemoryConsolidating(true);
    try {
      const response = await apiJson<MemoryConsolidationResponseRecord>("/memory/consolidate", {
        method: "POST",
        body: JSON.stringify({
          include_candidates: memoryConsolidateIncludeCandidates,
        }),
      });
      response.merged_record_ids = arrayOrEmpty(response.merged_record_ids);
      response.promoted_record_ids = arrayOrEmpty(response.promoted_record_ids);
      response.archived_record_ids = arrayOrEmpty(response.archived_record_ids);
      response.reflection_notes = arrayOrEmpty(response.reflection_notes);
      response.rule_hits = arrayOrEmpty(response.rule_hits);
      setMemoryConsolidationResult(response);
      await loadMemoryGovernance();
      await loadMemoryList();
      if (selectedMemoryId) {
        setSelectedMemoryId(selectedMemoryId);
      }
    } catch (error) {
      const message = `Failed to run memory consolidation: ${formatError(error)}`;
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryConsolidating(false);
    }
  }, [
    apiJson,
    loadMemoryGovernance,
    loadMemoryList,
    memoryConsolidateIncludeCandidates,
    onBanner,
    selectedMemoryId,
  ]);

  useEffect(() => {
    void reloadSettingsData();
  }, [reloadSettingsData]);

  useEffect(() => {
    if (modelOverrideDraft.providerId.trim() || providerChoices.length === 0) {
      return;
    }
    setModelOverrideDraft((current) => ({
      ...current,
      providerId: providerChoices[0],
    }));
  }, [modelOverrideDraft.providerId, providerChoices]);

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
  }, [managedProviders, selectedManagedProviderId]);

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
  }, [activeTab, apiJson, selectedManagedProviderId]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryList();
  }, [activeTab, loadMemoryList]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryPreview();
  }, [activeTab, loadMemoryPreview]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryGovernance();
  }, [activeTab, loadMemoryGovernance]);

  useEffect(() => {
    if (activeTab !== "memory" || !selectedMemoryId) {
      setMemoryDetail(null);
      setMemoryValidationReport(null);
      setMemoryConflicts(null);
      setMemoryDetailLoading(false);
      return;
    }

    let cancelled = false;
    setMemoryDetailLoading(true);

    void (async () => {
      try {
        const [detail, validation, conflicts] = await Promise.all([
          apiJson<MemoryDetailResponseRecord>(`/memory/${encodeURIComponent(selectedMemoryId)}`),
          apiJson<MemoryValidationReportResponseRecord>(
            `/memory/${encodeURIComponent(selectedMemoryId)}/validation-report`,
          ),
          apiJson<MemoryConflictResponseRecord>(
            `/memory/${encodeURIComponent(selectedMemoryId)}/conflicts`,
          ),
        ]);
        if (cancelled) return;
        detail.record.trigger_conditions = arrayOrEmpty(detail.record.trigger_conditions);
        detail.record.boundaries = arrayOrEmpty(detail.record.boundaries);
        detail.record.normalized_facts = arrayOrEmpty(detail.record.normalized_facts);
        detail.record.evidence_refs = arrayOrEmpty(detail.record.evidence_refs);
        if (validation.latest) {
          validation.latest.issues = arrayOrEmpty(validation.latest.issues);
        }
        conflicts.conflicts = arrayOrEmpty(conflicts.conflicts);
        setMemoryDetail(detail);
        setMemoryValidationReport(validation);
        setMemoryConflicts(conflicts);
      } catch (error) {
        if (cancelled) return;
        const message = `Failed to load memory detail: ${formatError(error)}`;
        setFeedback(message);
        onBanner(message);
        setMemoryDetail(null);
        setMemoryValidationReport(null);
        setMemoryConflicts(null);
      } finally {
        if (!cancelled) {
          setMemoryDetailLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeTab, apiJson, onBanner, selectedMemoryId]);

  useEffect(() => {
    if (skillCatalog.length === 0) {
      setSelectedSkillName(null);
      setSkillDetail(null);
      setSkillDetailLoading(false);
      setSkillEditorContent("");
      setEditSkillDescription("");
      setEditSkillEditorMode("raw");
      setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
      setEditSkillMethodologyMatched(false);
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    // Only auto-select first skill when the current selection is stale
    // (was removed from catalog). Do NOT auto-select when nothing is selected
    // so the catalog list is visible to the user.
    if (!selectedSkillName) {
      return;
    }

    const current = selectedSkillName.trim().toLowerCase();
    const matched = skillCatalog.find(
      (skill) => skill.name.trim().toLowerCase() === current,
    );

    if (matched) {
      return;
    }

    setSelectedSkillName(skillCatalog[0].name);
  }, [selectedSkillName, skillCatalog]);

  useEffect(() => {
    if (skillSyncSourceId.trim() || skillSourceIndices.length === 0) {
      return;
    }
    const firstSource = skillSourceIndices[0]?.source;
    if (!firstSource) {
      return;
    }
    setSkillSyncSourceId(firstSource.source_id);
    setSkillSyncSourceKind(firstSource.source_kind);
    setSkillSyncLocator(firstSource.locator);
    setSkillSyncRevision(firstSource.revision ?? "");
  }, [skillSourceIndices, skillSyncLocator, skillSyncRevision, skillSyncSourceId]);

  useEffect(() => {
    if (!selectedHubSourceSnapshot) {
      return;
    }
    const current = remoteInstallSkillName.trim().toLowerCase();
    const exactMatch = selectedHubSourceSnapshot.entries.some(
      (entry) => entry.skill_name.trim().toLowerCase() === current,
    );
    if (!current || !exactMatch) {
      setRemoteInstallSkillName(selectedHubSourceSnapshot.entries[0]?.skill_name ?? "");
    }
  }, [remoteInstallSkillName, selectedHubSourceSnapshot]);

  useEffect(() => {
    if (!selectedSkillName) {
      setSkillDetail(null);
      setSkillDetailLoading(false);
      setSkillEditorContent("");
      setEditSkillDescription("");
      setEditSkillEditorMode("raw");
      setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
      setEditSkillMethodologyMatched(false);
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    let cancelled = false;
    setSkillDetailLoading(true);

    void (async () => {
      try {
        const detailPath = selectedSessionId
          ? `/skill/detail?name=${encodeURIComponent(selectedSkillName)}&session_id=${encodeURIComponent(selectedSessionId)}`
          : `/skill/detail?name=${encodeURIComponent(selectedSkillName)}`;
        const detail = await apiJson<SkillDetailResponseRecord>(
          detailPath,
        );
        if (cancelled) return;
        let extractedMethodology: SkillMethodologyTemplateRecord | null = null;
        try {
          const extracted = await apiJson<SkillMethodologyExtractResponseRecord>(
            "/skill/methodology/extract",
            {
              method: "POST",
              body: JSON.stringify({
                content: detail.source ?? "",
              }),
            },
          );
          extractedMethodology = extracted.matched ? extracted.methodology ?? null : null;
        } catch {
          extractedMethodology = null;
        }
        if (cancelled) return;
        setSkillDetail(detail);
        setSkillEditorContent(detail.source ?? "");
        setEditSkillDescription(detail.skill.meta.description ?? "");
        setEditSkillMethodologyMatched(Boolean(extractedMethodology));
        setEditSkillMethodologyDraft(
          extractedMethodology
            ? methodologyDraftFromTemplate(extractedMethodology)
            : emptySkillMethodologyDraft(),
        );
        setEditSkillEditorMode(extractedMethodology ? "methodology" : "raw");
      } catch (error) {
        if (cancelled) return;
        const message = `Failed to load skill ${selectedSkillName}: ${formatError(error)}`;
        setSkillDetail(null);
        setSkillEditorContent("");
        setEditSkillDescription("");
        setEditSkillEditorMode("raw");
        setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
        setEditSkillMethodologyMatched(false);
        setEditSkillMethodologyPreview("");
        setEditSkillMethodologyPreviewError(null);
        setFeedback(message);
        onBanner(message);
      } finally {
        if (!cancelled) {
          setSkillDetailLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [apiJson, onBanner, selectedSessionId, selectedSkillName]);

  useEffect(() => {
    if (newSkillEditorMode !== "methodology") {
      setNewSkillMethodologyPreview("");
      setNewSkillMethodologyPreviewError(null);
      return;
    }

    const timer = window.setTimeout(() => {
      void requestSkillMethodologyPreview(
        newSkillName,
        newSkillMethodologyDraft,
        (body, error) => {
          setNewSkillMethodologyPreview(body);
          setNewSkillMethodologyPreviewError(error);
        },
      );
    }, 120);

    return () => window.clearTimeout(timer);
  }, [
    newSkillEditorMode,
    newSkillMethodologyDraft,
    newSkillName,
    requestSkillMethodologyPreview,
  ]);

  useEffect(() => {
    if (editSkillEditorMode !== "methodology" || !selectedSkillName) {
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    const timer = window.setTimeout(() => {
      void requestSkillMethodologyPreview(
        selectedSkillName,
        editSkillMethodologyDraft,
        (body, error) => {
          setEditSkillMethodologyPreview(body);
          setEditSkillMethodologyPreviewError(error);
        },
      );
    }, 120);

    return () => window.clearTimeout(timer);
  }, [
    editSkillEditorMode,
    editSkillMethodologyDraft,
    requestSkillMethodologyPreview,
    selectedSkillName,
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

  const saveScheduler = async () => {
    await runMutation(
      "scheduler:save",
      async () => {
        const response = await apiJson<SchedulerConfigResponse>("/config/scheduler", {
          method: "PUT",
          body: JSON.stringify({
            path: schedulerPathDraft.trim() || undefined,
            content: schedulerContentDraft,
          }),
        });
        setSchedulerConfig(response);
        setSchedulerPathDraft(response.raw_path ?? "");
        setSchedulerContentDraft(response.content ?? "");
      },
      "Scheduler config saved.",
    );
  };

  const saveMcpConfig = async (key: string, raw: string) => {
    await runMutation(
      `mcp:save:${key}`,
      async () => {
        await api(`/config/mcp/${encodeURIComponent(key)}`, {
          method: "PUT",
          body: JSON.stringify(parseObjectJson(`MCP ${key}`, raw)),
        });
      },
      `Saved MCP config ${key}.`,
    );
  };

  const deleteMcpConfig = async (key: string) => {
    await runMutation(
      `mcp:delete:${key}`,
      async () => {
        await api(`/config/mcp/${encodeURIComponent(key)}`, { method: "DELETE" });
      },
      `Removed MCP config ${key}.`,
    );
  };

  const savePluginConfig = async (key: string, raw: string) => {
    await runMutation(
      `plugin:save:${key}`,
      async () => {
        await api(`/config/plugin/${encodeURIComponent(key)}`, {
          method: "PUT",
          body: JSON.stringify(parseObjectJson(`Plugin ${key}`, raw)),
        });
      },
      `Saved plugin config ${key}.`,
    );
  };

  const deletePluginConfig = async (key: string) => {
    await runMutation(
      `plugin:delete:${key}`,
      async () => {
        await api(`/config/plugin/${encodeURIComponent(key)}`, { method: "DELETE" });
      },
      `Removed plugin config ${key}.`,
    );
  };

  const removeProvider = async (providerId: string) => {
    await runMutation(
      `provider:delete:${providerId}`,
      async () => {
        await api(`/provider/${encodeURIComponent(providerId)}`, { method: "DELETE" });
      },
      `Removed provider ${providerId}.`,
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
    [modelOverrideDraft.providerId, providerChoices],
  );

  const editModelOverride = useCallback(
    (providerId: string, record: ManagedModelOverrideInfoRecord) => {
      setEditingModelTarget({ providerId, modelKey: record.key });
      setModelOverrideDraft(modelOverrideDraftFromRecord(providerId, record));
    },
    [],
  );

  const saveModelOverride = async () => {
    const providerId = modelOverrideDraft.providerId.trim();
    const modelKey = modelOverrideDraft.modelKey.trim();
    if (!providerId) {
      throw new Error("Provider ID is required.");
    }
    if (!modelKey) {
      throw new Error("Model key is required.");
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
      `Saved model override ${providerId}/${modelKey}.`,
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
      `Removed model override ${providerId}/${modelKey}.`,
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
          ? `Provider catalogue refreshed (generation ${response.generation_before} -> ${response.generation_after}).`
          : response.status === "not_modified"
            ? `Provider catalogue checked; no changes (generation ${response.generation_after}).`
            : `Provider catalogue refresh failed; using cached snapshot: ${response.error_message ?? "Unknown refresh failure"}`;
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

  const runMcpAction = async (name: string, action: "connect" | "disconnect" | "restart") => {
    await runMutation(
      `mcp:${action}:${name}`,
      async () => {
        await api(`/mcp/${encodeURIComponent(name)}/${action}`, { method: "POST" });
      },
      `MCP ${name} ${action} complete.`,
    );
  };

  const buildSkillSyncSource = useCallback((): SkillSourceRefRecord => {
    if (!skillSyncSourceId.trim()) {
      throw new Error("Skill hub source_id is required.");
    }
    if (!skillSyncLocator.trim()) {
      throw new Error("Skill hub locator is required.");
    }
    return {
      source_id: skillSyncSourceId.trim(),
      source_kind: skillSyncSourceKind,
      locator: skillSyncLocator.trim(),
      revision: skillSyncRevision.trim() || undefined,
    };
  }, [skillSyncLocator, skillSyncRevision, skillSyncSourceId, skillSyncSourceKind]);

  const planSkillSync = async () => {
    const source = buildSkillSyncSource();
    setBusyKey(`skill:sync:plan:${source.source_id}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillHubSyncPlanResponseRecord>("/skill/hub/sync/plan", {
        method: "POST",
        body: JSON.stringify({ source }),
      });
      setSkillSyncPlan(response.plan);
      await reloadSettingsData();
      setFeedback(`Built skill sync plan for ${source.source_id}.`);
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const refreshSkillSourceIndex = async () => {
    const source = buildSkillSyncSource();
    setBusyKey(`skill:index:refresh:${source.source_id}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillHubIndexRefreshResponseRecord>("/skill/hub/index/refresh", {
        method: "POST",
        body: JSON.stringify({ source }),
      });
      await reloadSettingsData();
      setFeedback(
        `Refreshed source index for ${response.snapshot.source.source_id} (${response.snapshot.entries.length} entries).`,
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const runGuard = async (request: SkillHubGuardRunRequestRecord, targetLabel: string) => {
    await runMutation(
      `skill:guard:${targetLabel}`,
      async () => {
        const response = await apiJson<SkillHubGuardRunResponseRecord>("/skill/hub/guard/run", {
          method: "POST",
          body: JSON.stringify(request),
        });
        setSkillGuardTarget(targetLabel);
        setSkillGuardReports(response.reports);
        const violationCount = response.reports.reduce(
          (total, report) => total + report.violations.length,
          0,
        );
        return `Guard scanned ${targetLabel} (${response.reports.length} report${response.reports.length === 1 ? "" : "s"}, ${violationCount} total violations).`;
      },
      `Guard scanned ${targetLabel}.`,
    );
  };

  const applySkillSync = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    await runMutation(
      `skill:sync:apply:${source.source_id}`,
      async () => {
        const response = await apiJson<SkillHubSyncPlanResponseRecord>("/skill/hub/sync/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
          }),
        });
        setSkillSyncPlan(response.plan);
        if ((response.guard_reports?.length ?? 0) > 0) {
          return `Applied skill sync for ${source.source_id} with ${response.guard_reports?.length ?? 0} guard warnings.`;
        }
      },
      `Applied skill sync for ${source.source_id}.`,
    );
  };

  const planRemoteInstall = async () => {
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    setBusyKey(`skill:install:plan:${source.source_id}:${skillName}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillRemoteInstallPlanRecord>("/skill/hub/install/plan", {
        method: "POST",
        body: JSON.stringify({
          source,
          skill_name: skillName,
        }),
      });
      setRemoteInstallPlan(response);
      await reloadSettingsData();
      setFeedback(
        `Built remote install plan for ${response.entry.skill_name} from ${source.source_id} (${response.entry.action}).`,
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const applyRemoteInstall = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    await runMutation(
      `skill:install:apply:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillRemoteInstallResponseRecord>("/skill/hub/install/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        setRemoteInstallPlan(response.plan);
        const violationCount = response.guard_report?.violations.length ?? 0;
        if (violationCount > 0) {
          return `Applied remote ${response.plan.entry.action} for ${response.result.skill_name} with ${violationCount} guard warnings.`;
        }
        return `Applied remote ${response.plan.entry.action} for ${response.result.skill_name}.`;
      },
      `Applied remote install for ${skillName}.`,
    );
  };

  const planRemoteUpdate = async () => {
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    setBusyKey(`skill:update:plan:${source.source_id}:${skillName}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillRemoteInstallPlanRecord>("/skill/hub/update/plan", {
        method: "POST",
        body: JSON.stringify({
          source,
          skill_name: skillName,
        }),
      });
      setRemoteInstallPlan(response);
      await reloadSettingsData();
      setFeedback(
        `Built remote update plan for ${response.entry.skill_name} from ${source.source_id} (${response.entry.action}).`,
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const applyRemoteUpdate = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    await runMutation(
      `skill:update:apply:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillRemoteInstallResponseRecord>("/skill/hub/update/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        setRemoteInstallPlan(response.plan);
        const violationCount = response.guard_report?.violations.length ?? 0;
        if (violationCount > 0) {
          return `Applied remote ${response.plan.entry.action} for ${response.result.skill_name} with ${violationCount} guard warnings.`;
        }
        return `Applied remote ${response.plan.entry.action} for ${response.result.skill_name}.`;
      },
      `Applied remote update for ${skillName}.`,
    );
  };

  const detachManagedSkill = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    await runMutation(
      `skill:detach:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillHubManagedDetachResponseRecord>("/skill/hub/detach", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        return `Detached managed skill ${response.lifecycle.skill_name}; workspace content was preserved.`;
      },
      `Detached managed skill ${skillName}.`,
    );
  };

  const removeManagedSkill = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error("Select or type a remote skill name first.");
    }
    await runMutation(
      `skill:remove:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillHubManagedRemoveResponseRecord>("/skill/hub/remove", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        if (response.deleted_from_workspace) {
          return `Removed managed skill ${response.lifecycle.skill_name} and deleted the clean workspace copy.`;
        }
        return `Removed managed skill ${response.lifecycle.skill_name} without deleting the workspace copy.`;
      },
      `Removed managed skill ${skillName}.`,
    );
  };

  const createSkill = async () => {
    if (!selectedSessionId) return;
    await runMutation(
      `skill:create:${newSkillName.trim() || "new"}`,
      async () => {
        const methodology =
          newSkillEditorMode === "methodology"
            ? buildMethodologyTemplateFromDraft(newSkillMethodologyDraft)
            : undefined;
        const response = await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: "create",
            name: newSkillName,
            description: newSkillDescription,
            category: newSkillCategory.trim() || undefined,
            body: newSkillEditorMode === "raw" ? newSkillBody : undefined,
            methodology,
          }),
        });
        setSelectedSkillName(response.result.skill_name);
        setNewSkillName("");
        setNewSkillDescription("");
        setNewSkillCategory("");
        setNewSkillBody("");
        setNewSkillEditorMode("methodology");
        setNewSkillMethodologyDraft(emptySkillMethodologyDraft());
        setNewSkillMethodologyPreview("");
        setNewSkillMethodologyPreviewError(null);
        if (response.guard_report) {
          return `Created skill ${response.result.skill_name} with ${response.guard_report.violations.length} guard warnings.`;
        }
      },
      `Created skill ${newSkillName.trim()}.`,
    );
  };

  const saveSelectedSkill = async () => {
    if (!selectedSessionId || !selectedSkillName) return;
    await runMutation(
      `skill:edit:${selectedSkillName}`,
      async () => {
        const structuredMode = editSkillEditorMode === "methodology";
        const response = await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: structuredMode ? "patch" : "edit",
            name: selectedSkillName,
            description: structuredMode ? editSkillDescription : undefined,
            methodology: structuredMode
              ? buildMethodologyTemplateFromDraft(editSkillMethodologyDraft)
              : undefined,
            content: structuredMode ? undefined : skillEditorContent,
          }),
        });
        if (response.guard_report) {
          return `Saved skill ${selectedSkillName} with ${response.guard_report.violations.length} guard warnings.`;
        }
      },
      `Saved skill ${selectedSkillName}.`,
    );
  };

  const deleteSelectedSkill = async () => {
    if (!selectedSessionId || !selectedSkillName) return;
    const deletedSkillName = selectedSkillName;
    await runMutation(
      `skill:delete:${deletedSkillName}`,
      async () => {
        await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: "delete",
            name: deletedSkillName,
          }),
        });
        setSelectedSkillName(null);
      },
      `Deleted skill ${deletedSkillName}.`,
    );
  };

  const runSelectedSkillGuard = async () => {
    if (!selectedSkillName) return;
    await runGuard({ skill_name: selectedSkillName }, `skill ${selectedSkillName}`);
  };

  const runSelectedSourceGuard = async () => {
    const source = buildSkillSyncSource();
    await runGuard({ source }, `source ${source.source_id}`);
  };

  const providerSummary = `${providers.length} connected / ${knownProviders.length} known`;
  const chooseKnownProvider = (provider: KnownProviderEntry) => {
    onConnectQueryChange(provider.id);
  };
  const secondaryButtonClass =
    "roc-action roc-action-pill px-4 text-foreground text-sm cursor-pointer";
  const primaryButtonClass =
    "roc-action roc-action-pill border-foreground bg-foreground px-5 text-sm font-semibold text-background disabled:cursor-not-allowed disabled:opacity-60";
  const summaryCardClass = "rounded-lg border border-border/30 bg-card p-4 grid gap-1";
  const sectionCardClass = "grid gap-4 rounded-lg bg-muted/30 p-5";
  const mutedCardClass = "rounded-lg bg-muted/40 px-4 py-3 text-sm leading-relaxed text-muted-foreground";
  const insetCardClass = "rounded-lg border border-border/35 bg-card/80 p-4";
  const disclosureCardClass = "rounded-lg border border-border/35 bg-card/80";
  const editorTextareaClass =
    "roc-form-textarea min-h-40 font-mono";
  const formFieldClass = "roc-form-field";
  const formLabelClass = "roc-form-label";
  const formHintClass = "roc-form-hint";
  const inputClass = "roc-form-control";
  const selectClass = "roc-form-select";
  const checkboxRowClass = "roc-form-checkbox-row";
  const checkboxClass = "roc-form-checkbox";

  return (
    <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-start justify-end" data-testid="settings-overlay" onClick={onClose}>
      <section
        className="h-full w-full max-w-3xl bg-background border-l border-border/60 overflow-y-auto flex flex-col"
        data-testid="settings-drawer"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex items-start justify-between gap-4 px-8 pt-8 pb-6">
          <div>
            <p className="m-0 mb-1.5 text-xs tracking-widest uppercase text-amber-700 font-bold">Settings</p>
            <h2 className="text-xl font-semibold tracking-tight">General, providers, scheduler, validation, skills, MCP, plugins, LSP</h2>
          </div>
          <div className="flex items-center gap-2">
            <button
              className={secondaryButtonClass}
              type="button"
              data-testid="settings-refresh"
              onClick={() => void reloadSettingsData()}
            >
              {refreshing ? "Refreshing..." : "Refresh"}
            </button>
            <button className={secondaryButtonClass} type="button" data-testid="settings-close" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <nav className="px-8 border-b border-border/60">
          <div className="flex flex-wrap gap-6">
            {SETTINGS_TABS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                data-testid={`settings-tab-${tab.id}`}
                className={cn(
                  "relative py-2.5 text-sm font-medium cursor-pointer transition-colors",
                  activeTab === tab.id ? "text-foreground" : "text-muted-foreground hover:text-foreground"
                )}
                onClick={() => setActiveTab(tab.id)}
              >
                {tab.label}
                {activeTab === tab.id && (
                  <span className="absolute bottom-0 left-0 right-0 h-0.5 bg-foreground rounded-t-sm" />
                )}
              </button>
            ))}
          </div>
        </nav>

        <div className="flex flex-col gap-6 flex-1 min-h-0 px-8 pb-8 pt-6">
          {feedback ? <div className="rounded-lg border border-amber-300 bg-amber-50/80 px-4 py-2.5 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200">{feedback}</div> : null}
          {loading ? <div className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">Loading settings...</div> : null}
          {!loading && isolatedNotice ? (
            <div className="rounded-lg border border-amber-300 bg-amber-50/80 px-4 py-2.5 text-sm leading-relaxed text-amber-900 dark:border-amber-700 dark:bg-amber-950/60 dark:text-amber-200">
              {isolatedNotice}
            </div>
          ) : null}

          {!loading && activeTab === "general" ? (
            <div className="grid gap-6">
              <div className="roc-section">
                <div className={formFieldClass}>
                  <label className={formLabelClass}>Theme</label>
                </div>
                <div className="flex flex-wrap gap-2">
                  {themes.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={
                        theme === item.id
                          ? "roc-action roc-action-pill border-foreground bg-foreground px-4 text-sm font-medium text-background"
                          : "roc-action roc-action-pill px-4 text-sm text-muted-foreground"
                      }
                      onClick={() => onThemeChange(item.id)}
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
              </div>

              <div className="roc-section">
                <label htmlFor="settings-mode-select" className={formLabelClass}>Execution Mode</label>
                <select
                  id="settings-mode-select"
                  className={selectClass}
                  value={selectedMode}
                  onChange={(event) => onModeChange(event.target.value)}
                >
                  <option value="">auto</option>
                  {modeOptions.map((mode) => (
                    <option key={mode.key} value={mode.key}>
                      {mode.label}
                    </option>
                  ))}
                </select>
              </div>

              <div className="roc-section">
                <label htmlFor="settings-model-select" className={formLabelClass}>Model</label>
                <select
                  id="settings-model-select"
                  className={selectClass}
                  value={selectedModel}
                  onChange={(event) => onModelChange(event.target.value)}
                >
                  <option value="">auto</option>
                  {modelOptions.map((model) => (
                    <option key={model.key} value={model.key}>
                      {model.label}
                    </option>
                  ))}
                </select>
              </div>

              <div className="roc-section">
                <label className={checkboxRowClass}>
                  <input
                    className={checkboxClass}
                    type="checkbox"
                    checked={showThinking}
                    onChange={(event) => onShowThinkingChange(event.target.checked)}
                  />
                  Show reasoning blocks
                </label>
              </div>

              <div className="grid gap-3">
                <label>Workspace Authority</label>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className={summaryCardClass}>
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Workspace Mode</span>
                    <strong>{workspaceMode === "isolated" ? "isolated sandbox" : "shared workspace"}</strong>
                  </div>
                  <div className={summaryCardClass}>
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Workspace Root</span>
                    <strong className="break-all text-sm">{workspaceRootPath || "--"}</strong>
                  </div>
                </div>
                {workspaceConfigDir ? (
                  <p className="m-0 text-xs leading-relaxed text-muted-foreground">
                    Isolated config dir: <code>{workspaceConfigDir}</code>
                  </p>
                ) : null}
                <div
                  className={cn(
                    "rounded-lg px-4 py-3 text-sm leading-relaxed",
                    workspaceMode === "isolated"
                      ? "bg-amber-50/80 text-amber-900 dark:bg-amber-950/60 dark:text-amber-200"
                      : "bg-muted/40 text-muted-foreground",
                  )}
                >
                  {workspaceMode === "isolated"
                    ? "This workspace runs as an isolated sandbox. It will not inherit global config, managed home config, or shared workspace overrides outside this .rocode root."
                    : "This workspace runs in shared mode. Global config can still participate in the resolved runtime context alongside workspace-local settings."}
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Providers</span>
                  <strong>{providerSummary}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Scheduler Path</span>
                  <strong>{schedulerConfig?.raw_path || configSnapshot?.schedulerPath || "--"}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">MCP Servers</span>
                  <strong>{Object.keys(mcpConfigs).length}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Plugins</span>
                  <strong>{Object.keys(pluginConfigs).length}</strong>
                </div>
              </div>
            </div>
          ) : null}

          {!loading && activeTab === "memory" ? (
            <MemoryTab
              selectedSessionId={selectedSessionId}
              styles={{
                primaryButtonClass,
                secondaryButtonClass,
                summaryCardClass,
                sectionCardClass,
                mutedCardClass,
                insetCardClass,
                disclosureCardClass,
              }}
              memorySearchDraft={memorySearchDraft}
              onMemorySearchDraftChange={setMemorySearchDraft}
              memoryListLoading={memoryListLoading}
              onLoadMemoryList={() => void loadMemoryList()}
              memoryPreviewLoading={memoryPreviewLoading}
              onLoadMemoryPreview={() => void loadMemoryPreview()}
              memoryGovernanceLoading={memoryGovernanceLoading}
              onLoadMemoryGovernance={() => void loadMemoryGovernance()}
              memoryConsolidateIncludeCandidates={memoryConsolidateIncludeCandidates}
              onMemoryConsolidateIncludeCandidatesChange={setMemoryConsolidateIncludeCandidates}
              memoryConsolidating={memoryConsolidating}
              onRunMemoryConsolidation={() => void runMemoryConsolidation()}
              memoryListResponse={memoryListResponse}
              selectedMemoryId={selectedMemoryId}
              selectedMemoryCardIdLabel={
                selectedMemoryCard ? memoryRecordIdValue(selectedMemoryCard.id) : null
              }
              onSelectMemoryId={setSelectedMemoryId}
              memoryDetailLoading={memoryDetailLoading}
              memoryDetail={memoryDetail}
              memoryValidationReport={memoryValidationReport}
              memoryConflicts={memoryConflicts}
              memoryPreview={memoryPreview}
              memoryRulePacks={memoryRulePacks}
              memoryRuleHits={memoryRuleHits}
              memoryConsolidationRuns={memoryConsolidationRuns}
              memoryConsolidationResult={memoryConsolidationResult}
            />
          ) : null}

          {!loading && activeTab === "providers" ? (
            <div className="grid gap-6">
              <form
                className="roc-section"
                onSubmit={(event) => {
                  event.preventDefault();
                  void (async () => {
                    await onConnectProvider();
                    await reloadSettingsData();
                  })();
                }}
              >
                <label htmlFor="settings-provider-connect-query" className={formLabelClass}>Connect Provider</label>
                <input
                  id="settings-provider-connect-query"
                  className={inputClass}
                  type="text"
                  placeholder="Search provider or enter custom id"
                  value={connectQuery}
                  onChange={(event) => onConnectQueryChange(event.target.value)}
                />
                {connectQuery.trim() ? (
                  <div className="grid gap-2 rounded-[18px] border border-border/35 bg-muted/10 p-3">
                    {connectResolveBusy ? (
                      <p className={formHintClass}>
                        Resolving provider defaults...
                      </p>
                    ) : connectResolveError ? (
                      <p className="m-0 text-sm text-red-600 dark:text-red-300">
                        Failed to resolve provider defaults: {connectResolveError}
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
                        No known provider match. This query will fall back to custom provider id{" "}
                        <code>{connectResolution?.custom_draft.provider_id || connectQuery.trim()}</code>.
                      </p>
                    )}
                  </div>
                ) : null}
                <label className={formFieldClass}>
                  <span className={formLabelClass}>Provider ID</span>
                  <input
                    className={inputClass}
                    type="text"
                    placeholder="Provider id"
                    value={connectProviderId}
                    onChange={(event) => onConnectProviderIdChange(event.target.value)}
                  />
                </label>
                {exactKnownProvider ? (
                  <p className={formHintClass}>
                    Known provider match.
                    {exactKnownProvider.env?.length
                      ? ` Expected env: ${exactKnownProvider.env.join(", ")}.`
                      : ""}
                    {exactKnownProvider.model_count
                      ? ` ${exactKnownProvider.model_count} models in catalogue.`
                      : ""}
                  </p>
                ) : connectResolution?.draft.mode === "custom" ? (
                  <p className={formHintClass}>
                    No known provider matched. You can keep the suggested custom draft or edit the
                    provider id, base URL and protocol below.
                  </p>
                ) : null}
                <label className={formFieldClass}>
                  <span className={formLabelClass}>API Key</span>
                  <input
                    className={inputClass}
                    type="password"
                    placeholder="API key"
                    value={connectApiKey}
                    onChange={(event) => onConnectApiKeyChange(event.target.value)}
                  />
                </label>
                <label className={formFieldClass}>
                  <span className={formLabelClass}>Base URL</span>
                  <input
                    className={inputClass}
                    type="url"
                    placeholder="Custom base URL (optional)"
                    value={connectBaseUrl}
                    onChange={(event) => onConnectBaseUrlChange(event.target.value)}
                  />
                </label>
                <label className={formFieldClass}>
                  <span className={formLabelClass}>Protocol</span>
                  <select
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
                <button className={primaryButtonClass} type="submit" disabled={connectBusy}>
                  {connectBusy ? "Connecting..." : "Connect"}
                </button>
              </form>

              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Configured Providers</p>
                  <div className="flex items-center gap-2">
                    <span>{providerSummary}</span>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === "provider:refresh"}
                      onClick={() => void refreshProviderCatalogue()}
                    >
                      {busyKey === "provider:refresh" ? "Refreshing..." : "Refresh Catalogue"}
                    </button>
                  </div>
                </div>
                {providers.map((provider) => (
                  <div key={provider.id} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4">
                    <div>
                      <strong>{provider.name}</strong>
                      <p className="text-sm text-muted-foreground leading-relaxed">
                        {provider.id} · {(provider.models ?? []).length} models
                      </p>
                    </div>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === `provider:delete:${provider.id}`}
                      onClick={() => void removeProvider(provider.id)}
                    >
                      Remove
                    </button>
                  </div>
                ))}
              </div>

              <div className="grid gap-4 rounded-lg border border-border/35 bg-card/70 p-5">
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <p className="m-0 text-xs tracking-widest uppercase text-muted-foreground font-semibold">Model Overrides</p>
                    <p className="m-0 text-sm text-muted-foreground">
                      Add or edit provider-scoped model entries used by web, CLI and TUI.
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <span>{configuredModelOverrides.length} configured</span>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      onClick={() => resetModelOverrideDraft()}
                    >
                      {editingModelTarget ? "New Override" : "Reset"}
                    </button>
                  </div>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Provider ID</span>
                    <select
                      className={selectClass}
                      value={modelOverrideDraft.providerId.trim()}
                      disabled={modelOverrideProviderOptions.length === 0}
                      data-testid="settings-model-override-provider"
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          providerId: event.target.value,
                        }))
                      }
                    >
                      <option value="" disabled>
                        {modelOverrideProviderOptions.length
                          ? "Select a provider"
                          : "No providers available"}
                      </option>
                      {modelOverrideProviderOptions.map(({ id, label }) => (
                        <option key={id} value={id}>
                          {label}
                        </option>
                      ))}
                    </select>
                    <span className={formHintClass}>
                      Model overrides attach to a concrete provider and are shared by Web, CLI, and TUI.
                    </span>
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Model Key</span>
                    <input
                      className={inputClass}
                      value={modelOverrideDraft.modelKey}
                      disabled={Boolean(editingModelTarget)}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          modelKey: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Upstream Model ID</span>
                    <input
                      className={inputClass}
                      placeholder="gpt-4.1, qwen-max, claude-sonnet-4..."
                      value={modelOverrideDraft.modelId}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          modelId: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Display Name</span>
                    <input
                      className={inputClass}
                      value={modelOverrideDraft.name}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          name: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Base URL</span>
                    <input
                      className={inputClass}
                      value={modelOverrideDraft.baseUrl}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          baseUrl: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Family</span>
                    <input
                      className={inputClass}
                      value={modelOverrideDraft.family}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          family: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Status</span>
                    <input
                      className={inputClass}
                      value={modelOverrideDraft.status}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          status: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label className={formFieldClass}>
                    <span className={formLabelClass}>Release Date</span>
                    <input
                      className={inputClass}
                      placeholder="2026-04-24"
                      value={modelOverrideDraft.releaseDate}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
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
                        setModelOverrideDraft((current) => ({
                          ...current,
                          reasoning: event.target.checked,
                        }))
                      }
                    />
                    Reasoning
                  </label>
                  <label className={checkboxRowClass}>
                    <input
                      className={checkboxClass}
                      type="checkbox"
                      checked={modelOverrideDraft.toolCall}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          toolCall: event.target.checked,
                        }))
                      }
                    />
                    Tool Call
                  </label>
                  <label className={checkboxRowClass}>
                    <input
                      className={checkboxClass}
                      type="checkbox"
                      checked={modelOverrideDraft.attachment}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          attachment: event.target.checked,
                        }))
                      }
                    />
                    Attachment
                  </label>
                  <label className={checkboxRowClass}>
                    <input
                      className={checkboxClass}
                      type="checkbox"
                      checked={modelOverrideDraft.temperature}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          temperature: event.target.checked,
                        }))
                      }
                    />
                    Temperature
                  </label>
                  <label className={checkboxRowClass}>
                    <input
                      className={checkboxClass}
                      type="checkbox"
                      checked={modelOverrideDraft.experimental}
                      onChange={(event) =>
                        setModelOverrideDraft((current) => ({
                          ...current,
                          experimental: event.target.checked,
                        }))
                      }
                    />
                    Experimental
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
                    onClick={() => void saveModelOverride()}
                  >
                    {editingModelTarget ? "Save Override" : "Add Override"}
                  </button>
                  {editingModelTarget ? (
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      onClick={() => resetModelOverrideDraft(modelOverrideDraft.providerId)}
                    >
                      Cancel Edit
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
                              {override.model ? ` · model ${override.model}` : ""}
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
                                .join(" · ") || "No extra metadata"}
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
                                .join(" · ") || "No capability flags set"}
                            </p>
                          </div>
                          <div className="flex items-center gap-2">
                            <button
                              className={secondaryButtonClass}
                              type="button"
                              onClick={() => editModelOverride(providerId, override)}
                            >
                              Edit
                            </button>
                            <button
                              className={secondaryButtonClass}
                              type="button"
                              disabled={
                                busyKey === `provider:model:delete:${providerId}:${override.key}`
                              }
                              onClick={() => void deleteModelOverride(providerId, override.key)}
                            >
                              Delete
                            </button>
                          </div>
                        </div>
                      </div>
                    ))
                  ) : (
                    <p className="m-0 text-sm text-muted-foreground">
                      No model overrides are configured yet.
                    </p>
                  )}
                </div>
              </div>

              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Managed Providers</p>
                  <span>{managedProviders.length} items</span>
                </div>
                {managedProviders.map((provider) => {
                  const selected =
                    provider.id.trim().toLowerCase() ===
                    (selectedManagedProviderId ?? "").trim().toLowerCase();
                  return (
                    <button
                      key={provider.id}
                      type="button"
                      className={cn(
                        "rounded-lg border bg-card p-4 text-left transition-colors flex items-start justify-between gap-4",
                        selected
                          ? "border-primary/50 bg-primary/5"
                          : "border-border/40 hover:border-border/70",
                      )}
                      onClick={() => setSelectedManagedProviderId(provider.id)}
                    >
                      <div>
                        <strong>{provider.name}</strong>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {provider.id}
                        </p>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          status {provider.status}
                          {provider.auth_type ? ` · auth ${provider.auth_type}` : ""}
                        </p>
                      </div>
                      <span className={cn("rounded-full border px-3 py-1.5 text-xs font-semibold", statusTone(provider.status) === "ok" ? "border-green-300 bg-green-50 text-green-800 dark:border-green-700 dark:bg-green-950 dark:text-green-300" : statusTone(provider.status) === "warn" ? "border-amber-300 bg-amber-50 text-amber-800 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-300" : statusTone(provider.status) === "danger" ? "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950 dark:text-red-300" : "border-border bg-muted text-muted-foreground")}>
                        {provider.status}
                      </span>
                    </button>
                  );
                })}
                {selectedManagedProvider ? (
                  <div className="roc-section">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                          Provider Inspection
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
                        {providerDescriptorLoading ? "Loading..." : "Read-only"}
                      </span>
                    </div>
                    {selectedProviderDescriptorError ? (
                      <p className="m-0 text-sm text-red-600 dark:text-red-300">
                        Descriptor unavailable: {selectedProviderDescriptorError}
                      </p>
                    ) : null}
                    {selectedProviderDescriptor ? (
                      <div className="grid gap-3 md:grid-cols-2">
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Base URL</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderDescriptor.base_url || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Env</span>
                          <span className="text-sm text-foreground">
                            {(selectedProviderDescriptor.env ?? []).length
                              ? (selectedProviderDescriptor.env ?? []).join(", ")
                              : "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Profile Source</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.source || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>API Family</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.api_family || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>API Shape</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.api_shape || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Transport</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.transport || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Usage Shape</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.usage_shape || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Cache Family</span>
                          <span className="text-sm text-foreground">
                            {selectedProviderProfile?.cache_family || "--"}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <span className={formLabelClass}>Quirks</span>
                          <span className="text-sm text-foreground">
                            {(selectedProviderProfile?.quirks ?? []).length
                              ? (selectedProviderProfile?.quirks ?? []).join(", ")
                              : "--"}
                          </span>
                        </div>
                      </div>
                    ) : providerDescriptorLoading ? (
                      <p className={formHintClass}>Loading descriptor...</p>
                    ) : (
                      <p className={formHintClass}>
                        No descriptor candidate available for this provider.
                      </p>
                    )}
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}

          {!loading && activeTab === "scheduler" ? (
            <div className="grid gap-6">
              <div className="roc-section">
                <label htmlFor="settings-scheduler-path" className={formLabelClass}>Scheduler Config Path</label>
                <input
                  id="settings-scheduler-path"
                  className={inputClass}
                  type="text"
                  value={schedulerPathDraft}
                  onChange={(event) => setSchedulerPathDraft(event.target.value)}
                />
                <div className={formHintClass}>
                  resolved {schedulerConfig?.resolved_path || "--"} · {schedulerConfig?.exists ? "exists" : "new file"}
                </div>
              </div>

              <div className="roc-section">
                <label htmlFor="settings-scheduler-content" className={formLabelClass}>Scheduler Config</label>
                <textarea
                  id="settings-scheduler-content"
                  className={editorTextareaClass}
                  value={schedulerContentDraft}
                  onChange={(event) => setSchedulerContentDraft(event.target.value)}
                  spellCheck={false}
                />
                <div className="flex items-center gap-2">
                  <button
                    className={primaryButtonClass}
                    type="button"
                    disabled={busyKey === "scheduler:save"}
                    onClick={() => void saveScheduler()}
                  >
                    {busyKey === "scheduler:save" ? "Saving..." : "Save Scheduler Config"}
                  </button>
                </div>
                {schedulerConfig?.parse_error ? (
                  <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">{schedulerConfig.parse_error}</div>
                ) : null}
              </div>

              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Profiles</p>
                  <span>default {schedulerConfig?.default_profile || "--"}</span>
                </div>
                {schedulerConfig?.profiles.length ? (
                  schedulerConfig.profiles.map((profile) => (
                    <div key={profile.key} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4">
                      <div>
                        <strong>{profile.key}</strong>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {profile.orchestrator || "no orchestrator"}
                          {profile.description ? ` · ${profile.description}` : ""}
                        </p>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {profile.stages.length ? profile.stages.join(" -> ") : "no stages"}
                        </p>
                      </div>
                    </div>
                  ))
                ) : (
                  <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">No scheduler profiles parsed yet.</p>
                )}
              </div>
            </div>
          ) : null}

          {!loading && activeTab === "validation" ? (
            <div className="grid gap-6">
              <div className="grid gap-3 sm:grid-cols-4">
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Config Revision</span>
                  <strong>{configValidation?.revision ?? "--"}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Generated</span>
                  <strong className="text-sm">{formatDateTime(configValidation?.generated_at_ms)}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Errors</span>
                  <strong>{configValidation ? validationErrorCount : "--"}</strong>
                </div>
                <div className={summaryCardClass}>
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Warnings</span>
                  <strong>{configValidation ? validationWarningCount : "--"}</strong>
                </div>
              </div>

              {!configValidation ? (
                <div className={mutedCardClass}>
                  Validation snapshot was unavailable during this refresh. The rest of the settings surface still reflects current config and owner-local inspections.
                </div>
              ) : null}

              {configValidation && validationReports.length === 0 ? (
                <div className={mutedCardClass}>
                  No validation findings are present in the current config snapshot.
                </div>
              ) : null}

              {configValidation && validationGroups.length > 0 ? (
                validationGroups.map((group) => (
                  <div key={group.owner} className="roc-section">
                    <div className="flex items-center justify-between gap-3">
                      <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                        {validationOwnerLabel(group.owner)}
                      </p>
                      <span className="text-sm text-muted-foreground">
                        {group.items.length} finding{group.items.length === 1 ? "" : "s"}
                      </span>
                    </div>
                    <div className="grid gap-3">
                      {group.items.map((item) => {
                        const jumpTarget = validationJumpTarget(item);
                        return (
                          <div
                            key={`${item.owner}:${item.path}:${item.code}:${item.scope.subject_id ?? ""}`}
                            className="rounded-lg border border-border/35 bg-card p-4 grid gap-3"
                          >
                            <div className="flex flex-wrap items-start justify-between gap-3">
                              <div className="grid gap-1">
                                <strong>{item.code}</strong>
                                <p className="m-0 text-sm text-muted-foreground leading-relaxed break-all">
                                  <code>{item.path}</code>
                                </p>
                              </div>
                              <div className="flex flex-wrap items-center justify-end gap-2">
                                <span
                                  className={cn(
                                    "rounded-full border px-3 py-1.5 text-xs font-semibold",
                                    validationSeverityTone(item.severity) === "danger"
                                      ? "border-red-300 bg-red-50 text-red-800 dark:border-red-700 dark:bg-red-950 dark:text-red-300"
                                      : validationSeverityTone(item.severity) === "warn"
                                        ? "border-amber-300 bg-amber-50 text-amber-800 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-300"
                                        : "border-border bg-muted text-muted-foreground",
                                  )}
                                >
                                  {item.severity}
                                </span>
                                <span className="rounded-full border border-border bg-muted px-3 py-1.5 text-xs font-semibold text-muted-foreground">
                                  {validationEffectLabel(item.effect)}
                                </span>
                                {jumpTarget ? (
                                  <button
                                    className={secondaryButtonClass}
                                    type="button"
                                    onClick={() => jumpToValidationTarget(item)}
                                  >
                                    {jumpTarget.label}
                                  </button>
                                ) : null}
                              </div>
                            </div>
                            <p className="m-0 text-sm leading-relaxed text-foreground">
                              {item.message}
                            </p>
                            <div className="grid gap-1 text-sm text-muted-foreground">
                              <p className="m-0">
                                Scope: {validationScopeLabel(item.scope.kind)}
                                {item.scope.subject_id ? ` · ${item.scope.subject_id}` : ""}
                              </p>
                              {item.fallback ? (
                                <p className="m-0">Fallback: {item.fallback}</p>
                              ) : null}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ))
              ) : null}
            </div>
          ) : null}

          {!loading && activeTab === "skills" ? (
            <SkillsTab
              workspaceRootPath={workspaceRootPath}
              selectedSessionId={selectedSessionId}
              skillWorkspaceRoot={skillWorkspaceRoot}
              skillsMutationsEnabled={skillsMutationsEnabled}
              styles={{
                primaryButtonClass,
                secondaryButtonClass,
                summaryCardClass,
                sectionCardClass,
                mutedCardClass,
                editorTextareaClass,
              }}
              busyKey={busyKey}
              skillCatalog={skillCatalog}
              managedSkills={managedSkills}
              skillSourceIndices={skillSourceIndices}
              skillDistributions={skillDistributions}
              skillArtifactCache={skillArtifactCache}
              skillHubPolicy={skillHubPolicy}
              skillLifecycle={skillLifecycle}
              skillGovernanceTimeline={skillGovernanceTimeline}
              skillSyncSourceId={skillSyncSourceId}
              onSkillSyncSourceIdChange={setSkillSyncSourceId}
              skillSyncSourceKind={skillSyncSourceKind}
              onSkillSyncSourceKindChange={setSkillSyncSourceKind}
              skillSyncLocator={skillSyncLocator}
              onSkillSyncLocatorChange={setSkillSyncLocator}
              skillSyncRevision={skillSyncRevision}
              onSkillSyncRevisionChange={setSkillSyncRevision}
              skillSyncPlan={skillSyncPlan}
              onPlanSkillSync={() => void planSkillSync()}
              onApplySkillSync={() => void applySkillSync()}
              onRefreshSkillSourceIndex={() => void refreshSkillSourceIndex()}
              onRunSelectedSourceGuard={() => void runSelectedSourceGuard()}
              remoteInstallSkillName={remoteInstallSkillName}
              onRemoteInstallSkillNameChange={setRemoteInstallSkillName}
              remoteInstallPlan={remoteInstallPlan}
              onPlanRemoteInstall={() => void planRemoteInstall()}
              onPlanRemoteUpdate={() => void planRemoteUpdate()}
              onApplyRemoteInstall={() => void applyRemoteInstall()}
              onApplyRemoteUpdate={() => void applyRemoteUpdate()}
              onDetachManagedSkill={() => void detachManagedSkill()}
              onRemoveManagedSkill={() => void removeManagedSkill()}
              skillGuardReports={skillGuardReports}
              skillGuardTarget={skillGuardTarget}
              selectedSkillName={selectedSkillName}
              onSelectedSkillNameChange={setSelectedSkillName}
              selectedSkillEntry={selectedSkillEntry}
              skillDetail={skillDetail}
              skillDetailLoading={skillDetailLoading}
              skillEditorContent={skillEditorContent}
              onSkillEditorContentChange={setSkillEditorContent}
              editSkillEditorMode={editSkillEditorMode}
              onEditSkillEditorModeChange={setEditSkillEditorMode}
              editSkillDescription={editSkillDescription}
              onEditSkillDescriptionChange={setEditSkillDescription}
              editSkillMethodologyDraft={editSkillMethodologyDraft}
              onEditSkillMethodologyDraftChange={setEditSkillMethodologyDraft}
              editSkillMethodologyMatched={editSkillMethodologyMatched}
              editSkillMethodologyPreview={editSkillMethodologyPreview}
              editSkillMethodologyPreviewError={editSkillMethodologyPreviewError}
              newSkillName={newSkillName}
              onNewSkillNameChange={setNewSkillName}
              newSkillDescription={newSkillDescription}
              onNewSkillDescriptionChange={setNewSkillDescription}
              newSkillCategory={newSkillCategory}
              onNewSkillCategoryChange={setNewSkillCategory}
              newSkillBody={newSkillBody}
              onNewSkillBodyChange={setNewSkillBody}
              newSkillEditorMode={newSkillEditorMode}
              onNewSkillEditorModeChange={setNewSkillEditorMode}
              newSkillMethodologyDraft={newSkillMethodologyDraft}
              onNewSkillMethodologyDraftChange={setNewSkillMethodologyDraft}
              newSkillMethodologyPreview={newSkillMethodologyPreview}
              newSkillMethodologyPreviewError={newSkillMethodologyPreviewError}
              onCreateSkill={() => void createSkill()}
              onRunSelectedSkillGuard={() => void runSelectedSkillGuard()}
              onSaveSelectedSkill={() => void saveSelectedSkill()}
              onDeleteSelectedSkill={() => void deleteSelectedSkill()}
              managedRecordBySkill={managedRecordBySkill}
              latestGuardBySkill={latestGuardBySkill}
              selectedHubSourceSnapshot={selectedHubSourceSnapshot}
              selectedRemoteSourceEntry={selectedRemoteSourceEntry}
              selectedRemoteDistribution={selectedRemoteDistribution}
              selectedRemoteArtifactCache={selectedRemoteArtifactCache}
              selectedRemoteLifecycle={selectedRemoteLifecycle}
            />
          ) : null}

          {!loading && activeTab === "mcp" ? (
            <div className="grid gap-6">
              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Runtime Status</p>
                  <span>{Object.keys(mcpStatus).length} servers</span>
                </div>
                {Object.values(mcpStatus).length ? (
                  Object.values(mcpStatus).map((server) => (
                    <div key={server.name} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4">
                      <div>
                        <strong>{server.name}</strong>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          status {server.status} · tools {server.tools} · resources {server.resources}
                        </p>
                        {server.error ? <p className="text-sm text-muted-foreground leading-relaxed">{server.error}</p> : null}
                      </div>
                      <div className="flex items-center gap-2">
                        <button
                          className={secondaryButtonClass}
                          type="button"
                          disabled={busyKey === `mcp:connect:${server.name}`}
                          onClick={() => void runMcpAction(server.name, "connect")}
                        >
                          Connect
                        </button>
                        <button
                          className={secondaryButtonClass}
                          type="button"
                          disabled={busyKey === `mcp:disconnect:${server.name}`}
                          onClick={() => void runMcpAction(server.name, "disconnect")}
                        >
                          Disconnect
                        </button>
                        <button
                          className={secondaryButtonClass}
                          type="button"
                          disabled={busyKey === `mcp:restart:${server.name}`}
                          onClick={() => void runMcpAction(server.name, "restart")}
                        >
                          Restart
                        </button>
                      </div>
                    </div>
                  ))
                ) : (
                  <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">No MCP runtime entries reported yet.</p>
                )}
              </div>

              {Object.entries(mcpConfigs).map(([key]) => (
                <div key={key} className="roc-section">
                  <label className={formLabelClass}>{key}</label>
                  <textarea
                    className={editorTextareaClass}
                    value={mcpDrafts[key] ?? ""}
                    onChange={(event) => setMcpDrafts((current) => ({ ...current, [key]: event.target.value }))}
                    spellCheck={false}
                  />
                  <div className="flex items-center gap-2">
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === `mcp:save:${key}`}
                      onClick={() => void saveMcpConfig(key, mcpDrafts[key] ?? "")}
                    >
                      Save
                    </button>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === `mcp:delete:${key}`}
                      onClick={() => void deleteMcpConfig(key)}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))}

              <div className="roc-section">
                <label htmlFor="settings-new-mcp-key" className={formLabelClass}>New MCP Config</label>
                <input
                  id="settings-new-mcp-key"
                  className={inputClass}
                  type="text"
                  placeholder="server name"
                  value={newMcpKey}
                  onChange={(event) => setNewMcpKey(event.target.value)}
                />
                <textarea
                  className={editorTextareaClass}
                  value={newMcpDraft}
                  onChange={(event) => setNewMcpDraft(event.target.value)}
                  spellCheck={false}
                />
                <button
                  className={primaryButtonClass}
                  type="button"
                  disabled={!newMcpKey.trim() || busyKey === `mcp:save:${newMcpKey.trim()}`}
                  onClick={() => void saveMcpConfig(newMcpKey.trim(), newMcpDraft)}
                >
                  Add MCP Config
                </button>
              </div>
            </div>
          ) : null}

          {!loading && activeTab === "plugins" ? (
            <div className="grid gap-6">
              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Auth Bridges</p>
                  <span>{pluginAuthProviders.length} providers</span>
                </div>
                {pluginAuthProviders.length ? (
                  pluginAuthProviders.map((provider) => (
                    <div key={provider.provider} className="rounded-lg border border-border/40 bg-card p-4 flex items-start justify-between gap-4">
                      <div>
                        <strong>{provider.provider}</strong>
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {provider.methods.length
                            ? provider.methods
                                .map((method) => method.label || method.type || "method")
                                .join(", ")
                            : "no auth methods"}
                        </p>
                      </div>
                    </div>
                  ))
                ) : (
                  <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">No plugin auth bridges reported.</p>
                )}
              </div>

              {Object.entries(pluginConfigs).map(([key]) => (
                <div key={key} className="roc-section">
                  <label className={formLabelClass}>{key}</label>
                  <textarea
                    className={editorTextareaClass}
                    value={pluginDrafts[key] ?? ""}
                    onChange={(event) => setPluginDrafts((current) => ({ ...current, [key]: event.target.value }))}
                    spellCheck={false}
                  />
                  <div className="flex items-center gap-2">
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === `plugin:save:${key}`}
                      onClick={() => void savePluginConfig(key, pluginDrafts[key] ?? "")}
                    >
                      Save
                    </button>
                    <button
                      className={secondaryButtonClass}
                      type="button"
                      disabled={busyKey === `plugin:delete:${key}`}
                      onClick={() => void deletePluginConfig(key)}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))}

              <div className="roc-section">
                <label htmlFor="settings-new-plugin-key" className={formLabelClass}>New Plugin Config</label>
                <input
                  id="settings-new-plugin-key"
                  className={inputClass}
                  type="text"
                  placeholder="plugin name"
                  value={newPluginKey}
                  onChange={(event) => setNewPluginKey(event.target.value)}
                />
                <textarea
                  className={editorTextareaClass}
                  value={newPluginDraft}
                  onChange={(event) => setNewPluginDraft(event.target.value)}
                  spellCheck={false}
                />
                <button
                  className={primaryButtonClass}
                  type="button"
                  disabled={!newPluginKey.trim() || busyKey === `plugin:save:${newPluginKey.trim()}`}
                  onClick={() => void savePluginConfig(newPluginKey.trim(), newPluginDraft)}
                >
                  Add Plugin Config
                </button>
              </div>
            </div>
          ) : null}

          {!loading && activeTab === "lsp" ? (
            <div className="grid gap-6">
              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">LSP Servers</p>
                  <span>{lspStatus?.servers.length ?? 0}</span>
                </div>
                {lspStatus?.servers.length ? (
                  lspStatus.servers.map((server) => (
                    <div key={server} className="rounded-lg border border-border/40 bg-card p-4 flex items-center justify-between gap-4">
                      <strong>{server}</strong>
                    </div>
                  ))
                ) : (
                  <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">No LSP servers reported by `/lsp`.</p>
                )}
              </div>

              <div className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Formatters</p>
                  <span>{formatterStatus?.formatters.length ?? 0}</span>
                </div>
                {formatterStatus?.formatters.length ? (
                  formatterStatus.formatters.map((formatter) => (
                    <div key={formatter} className="rounded-lg border border-border/40 bg-card p-4 flex items-center justify-between gap-4">
                      <strong>{formatter}</strong>
                    </div>
                  ))
                ) : (
                  <p className="flex flex-col items-center justify-center gap-3 text-muted-foreground py-8">No formatter status reported by `/formatter`.</p>
                )}
              </div>
            </div>
          ) : null}
        </div>
      </section>
    </div>
  );
}
