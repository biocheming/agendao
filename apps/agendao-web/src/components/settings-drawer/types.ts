import type {
  ConnectProtocolOption,
  KnownProviderEntry,
  ProviderRecord,
  ResolveProviderConnectResponseRecord,
} from "@/lib/provider";

export interface ThemeOption {
  id: string;
  label: string;
}

export interface ModeOption {
  key: string;
  label: string;
}

export interface ModelOption {
  key: string;
  label: string;
}

export interface McpStatusInfo {
  name: string;
  status: string;
  tools: number;
  resources: number;
  error?: string | null;
}

export interface PluginAuthProviderInfo {
  provider: string;
  methods: Array<{ type?: string; label?: string }>;
}

export interface LspStatus {
  servers: string[];
}

export interface FormatterStatus {
  formatters: string[];
}

export type SkillEditorMode = "methodology" | "raw";

export interface AppConfigSnapshot extends Record<string, unknown> {
  provider?: Record<string, unknown>;
  plugin?: Record<string, unknown>;
  mcp?: Record<string, unknown>;
}

export interface ModelOverrideDraft {
  providerId: string;
  modelKey: string;
  modelId: string;
  name: string;
  baseUrl: string;
  family: string;
  status: string;
  releaseDate: string;
  reasoning: boolean;
  reasoningEffort: string;
  toolCall: boolean;
  attachment: boolean;
  temperature: boolean;
  experimental: boolean;
}

export interface SettingsDrawerProps {
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
