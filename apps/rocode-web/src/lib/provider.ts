export interface ProviderModelModalityRecord {
  text?: boolean | null;
  audio?: boolean | null;
  image?: boolean | null;
  video?: boolean | null;
  pdf?: boolean | null;
}

export interface ProviderModelCapabilitiesRecord {
  attachment?: boolean | null;
  tool_call?: boolean | null;
  reasoning?: boolean | null;
  temperature?: boolean | null;
  input?: ProviderModelModalityRecord | null;
  output?: ProviderModelModalityRecord | null;
}

export interface ProviderModelRecord {
  id: string;
  name?: string;
  context_window?: number | null;
  max_output_tokens?: number | null;
  cost_per_million_input?: number | null;
  cost_per_million_output?: number | null;
  capabilities?: ProviderModelCapabilitiesRecord | null;
}

export interface ProviderRecord {
  id: string;
  name: string;
  models?: ProviderModelRecord[];
}

export interface KnownProviderEntry {
  id: string;
  name: string;
  env?: string[];
  connected?: boolean;
  model_count?: number;
  base_url?: string | null;
  protocol?: string | null;
  npm?: string | null;
  supports_api_key_connect?: boolean;
}

export interface ConnectProtocolOption {
  id: string;
  name: string;
}

export type ProviderConnectDraftMode = "known" | "custom";

export interface ProviderConnectDraft {
  mode: ProviderConnectDraftMode;
  provider_id: string;
  known_provider_id?: string | null;
  name?: string | null;
  base_url?: string | null;
  protocol?: string | null;
  env?: string[];
  connected?: boolean;
  model_count?: number;
  supports_api_key_connect?: boolean;
}

export interface ResolveProviderConnectResponseRecord {
  query: string;
  suggested_mode: ProviderConnectDraftMode;
  exact_match: boolean;
  matches: KnownProviderEntry[];
  draft: ProviderConnectDraft;
  custom_draft: ProviderConnectDraft;
}

export interface ConfigProvidersResponseRecord {
  providers?: ProviderRecord[];
  all?: ProviderRecord[];
}

export interface ProviderConnectSchemaResponseRecord {
  providers: KnownProviderEntry[];
  protocols: ConnectProtocolOption[];
}

export interface ManagedProviderInfoRecord {
  id: string;
  name: string;
  status: string;
  connected: boolean;
  configured: boolean;
  known: boolean;
  has_auth: boolean;
  auth_type?: string | null;
  env?: string[];
  base_url?: string | null;
  protocol?: string | null;
  model_overrides?: Array<{ key: string }>;
  models?: ProviderModelRecord[];
}

export interface ManagedProviderListResponseRecord {
  providers: ManagedProviderInfoRecord[];
}

export interface RefreshProviderCatalogueResponseRecord {
  changed: boolean;
  generation_before: number;
  generation_after: number;
  status: "updated" | "not_modified" | "fallback_cached";
  error_message?: string | null;
}

export function flattenProviderModels(providers: ProviderRecord[]) {
  return providers.flatMap((provider) =>
    (provider.models ?? []).map((model) => ({
      key: `${provider.id}/${model.id}`,
      label: `${provider.name} / ${model.name || model.id}`,
    })),
  );
}
