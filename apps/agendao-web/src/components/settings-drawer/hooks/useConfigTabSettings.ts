import { useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type {
  ConfigPolicyValidationItemRecord,
  ConfigPolicyValidationOwnerRecord,
  ConfigPolicyValidationSnapshotRecord,
} from "@/lib/configPolicy";
import { useI18n } from "@/i18n/I18nProvider";
import { parseObjectJson } from "../shared";
import type {
  FormatterStatus,
  LspStatus,
  McpStatusInfo,
  PluginAuthProviderInfo,
  SchedulerConfigResponse,
} from "../types";

export interface ConfigTabSettingsState {
  schedulerConfig: SchedulerConfigResponse | null;
  setSchedulerConfig: Dispatch<SetStateAction<SchedulerConfigResponse | null>>;
  schedulerPathDraft: string;
  setSchedulerPathDraft: Dispatch<SetStateAction<string>>;
  schedulerContentDraft: string;
  setSchedulerContentDraft: Dispatch<SetStateAction<string>>;
  configValidation: ConfigPolicyValidationSnapshotRecord | null;
  setConfigValidation: Dispatch<SetStateAction<ConfigPolicyValidationSnapshotRecord | null>>;
  validationReports: ConfigPolicyValidationItemRecord[];
  validationGroups: Array<{
    owner: ConfigPolicyValidationOwnerRecord;
    items: ConfigPolicyValidationItemRecord[];
  }>;
  validationErrorCount: number;
  validationWarningCount: number;
  mcpStatus: Record<string, McpStatusInfo>;
  setMcpStatus: Dispatch<SetStateAction<Record<string, McpStatusInfo>>>;
  mcpDrafts: Record<string, string>;
  setMcpDrafts: Dispatch<SetStateAction<Record<string, string>>>;
  newMcpKey: string;
  setNewMcpKey: Dispatch<SetStateAction<string>>;
  newMcpDraft: string;
  setNewMcpDraft: Dispatch<SetStateAction<string>>;
  pluginAuthProviders: PluginAuthProviderInfo[];
  setPluginAuthProviders: Dispatch<SetStateAction<PluginAuthProviderInfo[]>>;
  pluginDrafts: Record<string, string>;
  setPluginDrafts: Dispatch<SetStateAction<Record<string, string>>>;
  newPluginKey: string;
  setNewPluginKey: Dispatch<SetStateAction<string>>;
  newPluginDraft: string;
  setNewPluginDraft: Dispatch<SetStateAction<string>>;
  lspStatus: LspStatus | null;
  setLspStatus: Dispatch<SetStateAction<LspStatus | null>>;
  formatterStatus: FormatterStatus | null;
  setFormatterStatus: Dispatch<SetStateAction<FormatterStatus | null>>;
}

export function useConfigTabSettingsState(): ConfigTabSettingsState {
  const [schedulerConfig, setSchedulerConfig] = useState<SchedulerConfigResponse | null>(null);
  const [schedulerPathDraft, setSchedulerPathDraft] = useState("");
  const [schedulerContentDraft, setSchedulerContentDraft] = useState("");
  const [configValidation, setConfigValidation] =
    useState<ConfigPolicyValidationSnapshotRecord | null>(null);
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

  const validationReports = useMemo(
    () => configValidation?.reports ?? [],
    [configValidation?.reports],
  );
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

  return {
    schedulerConfig,
    setSchedulerConfig,
    schedulerPathDraft,
    setSchedulerPathDraft,
    schedulerContentDraft,
    setSchedulerContentDraft,
    configValidation,
    setConfigValidation,
    validationReports,
    validationGroups,
    validationErrorCount,
    validationWarningCount,
    mcpStatus,
    setMcpStatus,
    mcpDrafts,
    setMcpDrafts,
    newMcpKey,
    setNewMcpKey,
    newMcpDraft,
    setNewMcpDraft,
    pluginAuthProviders,
    setPluginAuthProviders,
    pluginDrafts,
    setPluginDrafts,
    newPluginKey,
    setNewPluginKey,
    newPluginDraft,
    setNewPluginDraft,
    lspStatus,
    setLspStatus,
    formatterStatus,
    setFormatterStatus,
  };
}

export interface ConfigTabSettingsMutationsDeps {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  runMutation: (
    key: string,
    action: () => Promise<string | void>,
    success: string,
  ) => Promise<void>;
  schedulerPathDraft: string;
  schedulerContentDraft: string;
  setSchedulerConfig: Dispatch<SetStateAction<SchedulerConfigResponse | null>>;
  setSchedulerPathDraft: Dispatch<SetStateAction<string>>;
  setSchedulerContentDraft: Dispatch<SetStateAction<string>>;
}

export interface ConfigTabSettingsMutations {
  saveScheduler: () => Promise<void>;
  saveMcpConfig: (key: string, raw: string) => Promise<void>;
  deleteMcpConfig: (key: string) => Promise<void>;
  savePluginConfig: (key: string, raw: string) => Promise<void>;
  deletePluginConfig: (key: string) => Promise<void>;
  runMcpAction: (name: string, action: "connect" | "disconnect" | "restart") => Promise<void>;
}

export function useConfigTabSettingsMutations({
  api,
  apiJson,
  runMutation,
  schedulerPathDraft,
  schedulerContentDraft,
  setSchedulerConfig,
  setSchedulerPathDraft,
  setSchedulerContentDraft,
}: ConfigTabSettingsMutationsDeps): ConfigTabSettingsMutations {
  const { t } = useI18n();
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
      t("settings.feedback.schedulerSaved"),
    );
  };

  const saveMcpConfig = async (key: string, raw: string) => {
    await runMutation(
      `mcp:save:${key}`,
      async () => {
        await api(`/config/mcp/${encodeURIComponent(key)}`, {
          method: "PUT",
          body: JSON.stringify(parseObjectJson(`MCP ${key}`, raw, t)),
        });
      },
      t("settings.feedback.mcpSaved", { key }),
    );
  };

  const deleteMcpConfig = async (key: string) => {
    await runMutation(
      `mcp:delete:${key}`,
      async () => {
        await api(`/config/mcp/${encodeURIComponent(key)}`, { method: "DELETE" });
      },
      t("settings.feedback.mcpRemoved", { key }),
    );
  };

  const savePluginConfig = async (key: string, raw: string) => {
    await runMutation(
      `plugin:save:${key}`,
      async () => {
        await api(`/config/plugin/${encodeURIComponent(key)}`, {
          method: "PUT",
          body: JSON.stringify(parseObjectJson(`Plugin ${key}`, raw, t)),
        });
      },
      t("settings.feedback.pluginSaved", { key }),
    );
  };

  const deletePluginConfig = async (key: string) => {
    await runMutation(
      `plugin:delete:${key}`,
      async () => {
        await api(`/config/plugin/${encodeURIComponent(key)}`, { method: "DELETE" });
      },
      t("settings.feedback.pluginRemoved", { key }),
    );
  };

  const runMcpAction = async (name: string, action: "connect" | "disconnect" | "restart") => {
    await runMutation(
      `mcp:${action}:${name}`,
      async () => {
        await api(`/mcp/${encodeURIComponent(name)}/${action}`, { method: "POST" });
      },
      t("settings.feedback.mcpActionComplete", { name, action }),
    );
  };

  return {
    saveScheduler,
    saveMcpConfig,
    deleteMcpConfig,
    savePluginConfig,
    deletePluginConfig,
    runMcpAction,
  };
}
