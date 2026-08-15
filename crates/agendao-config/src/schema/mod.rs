use crate::runtime_budget::RuntimeBudgetConfig;
use agendao_types::RepairPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub keybinds: Option<KeybindsConfig>,

    #[serde(rename = "logLevel", skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tui: Option<TuiConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<HashMap<String, CommandConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<SkillsConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<DocsConfig>,

    #[serde(rename = "taskCategoryPath", skip_serializing_if = "Option::is_none")]
    pub task_category_path: Option<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub skill_paths: HashMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher: Option<WatcherConfig>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub plugin: HashMap<String, PluginConfig>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub plugin_paths: HashMap<String, String>,

    #[serde(default, rename = "toolImports", skip_serializing_if = "Vec::is_empty")]
    pub tool_imports: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<ShareMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoupdate: Option<AutoUpdateMode>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_providers: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_providers: Vec<String>,

    /// Tool ids (or `family/*` category wildcards) removed from the tool
    /// surface at registry build time. Facade/bridge tools (`tool_catalog_*`,
    /// `skills_*`, `skill`, `skill_view`) are exempt and can never be disabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,

    /// Plugin names (or `prefix/*` wildcards) skipped at plugin load time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_plugins: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentConfigs>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<HashMap<String, ProviderConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<HashMap<String, McpServerConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatter: Option<FormatterConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspConfig>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutMode>,

    #[serde(rename = "uiPreferences", skip_serializing_if = "Option::is_none")]
    pub ui_preferences: Option<UiPreferencesConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,

    #[serde(rename = "webSearch", skip_serializing_if = "Option::is_none")]
    pub web_search: Option<WebSearchConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub multimodal: Option<MultimodalConfig>,

    #[serde(rename = "externalAdapter", skip_serializing_if = "Option::is_none")]
    pub external_adapter: Option<ExternalAdapterConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise: Option<EnterpriseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionConfig>,

    #[serde(rename = "repairPolicy", skip_serializing_if = "Option::is_none")]
    pub repair_policy: Option<RepairPolicy>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    /// Runtime budget authority (§5). When absent, Default applies.
    /// Every numerical budget/limit that governs runtime resource usage
    /// is read from this single struct — no semantic duplicates elsewhere.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "runtimeBudget"
    )]
    pub runtime_budget: Option<RuntimeBudgetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UiPreferencesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,

    #[serde(rename = "webTheme", skip_serializing_if = "Option::is_none")]
    pub web_theme: Option<String>,

    #[serde(rename = "webMode", skip_serializing_if = "Option::is_none")]
    pub web_mode: Option<String>,

    #[serde(rename = "showHeader", skip_serializing_if = "Option::is_none")]
    pub show_header: Option<bool>,

    #[serde(rename = "showScrollbar", skip_serializing_if = "Option::is_none")]
    pub show_scrollbar: Option<bool>,

    #[serde(rename = "tipsHidden", skip_serializing_if = "Option::is_none")]
    pub tips_hidden: Option<bool>,

    #[serde(rename = "showTimestamps", skip_serializing_if = "Option::is_none")]
    pub show_timestamps: Option<bool>,

    #[serde(rename = "showThinking", skip_serializing_if = "Option::is_none")]
    pub show_thinking: Option<bool>,

    #[serde(rename = "showToolDetails", skip_serializing_if = "Option::is_none")]
    pub show_tool_details: Option<bool>,

    #[serde(rename = "messageDensity", skip_serializing_if = "Option::is_none")]
    pub message_density: Option<String>,

    #[serde(rename = "semanticHighlight", skip_serializing_if = "Option::is_none")]
    pub semantic_highlight: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    /// MCP endpoint base URL, e.g. `"https://mcp.exa.ai"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// URL path appended to `base_url` (default `"/mcp"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// MCP tool method name (default `"web_search_exa"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    /// Default search type sent when the caller does not specify one
    /// (e.g. `"auto"`, `"fast"`, `"deep"`).
    #[serde(rename = "defaultSearchType", skip_serializing_if = "Option::is_none")]
    pub default_search_type: Option<String>,

    /// Default number of results (default `8`).
    #[serde(rename = "defaultNumResults", skip_serializing_if = "Option::is_none")]
    pub default_num_results: Option<usize>,

    /// Provider-specific key-value options that are forwarded as extra MCP
    /// arguments (e.g. `{ "livecrawl": "fallback" }`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MultimodalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<MultimodalLimitsConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<MultimodalAttachmentPolicyConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MultimodalLimitsConfig {
    #[serde(rename = "maxInputBytes", skip_serializing_if = "Option::is_none")]
    pub max_input_bytes: Option<usize>,

    #[serde(
        rename = "maxAttachmentsPerPrompt",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_attachments_per_prompt: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MultimodalAttachmentPolicyConfig {
    #[serde(rename = "allowAudioInput", skip_serializing_if = "Option::is_none")]
    pub allow_audio_input: Option<bool>,

    #[serde(rename = "allowImageInput", skip_serializing_if = "Option::is_none")]
    pub allow_image_input: Option<bool>,

    #[serde(rename = "allowFileInput", skip_serializing_if = "Option::is_none")]
    pub allow_file_input: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VoiceConfig {
    #[serde(rename = "durationSeconds", skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,

    #[serde(rename = "attachAudio", skip_serializing_if = "Option::is_none")]
    pub attach_audio: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<VoiceCommandConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcribe: Option<VoiceCommandConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VoiceCommandConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub adapters: HashMap<String, ExternalAdapterEntryConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay: Option<ExternalAdapterReplayConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterEntryConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Secret lookup key owned by `AuthManager`, for example
    /// `external-adapter:generic`. The config layer owns the reference only,
    /// never the secret value.
    #[serde(rename = "secretRef", skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,

    #[serde(rename = "defaultWorkspace", skip_serializing_if = "Option::is_none")]
    pub default_workspace: Option<String>,

    #[serde(rename = "routePolicyId", skip_serializing_if = "Option::is_none")]
    pub route_policy_id: Option<String>,

    /// Explicit execution gate for endpoints that can call into the shared
    /// session runtime entrypoint after webhook verification and replay
    /// recording. Defaults to false when omitted.
    #[serde(rename = "allowSessionRun", skip_serializing_if = "Option::is_none")]
    pub allow_session_run: Option<bool>,

    #[serde(
        default,
        rename = "allowedWorkspaces",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allowed_workspaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExternalAdapterReplayConfig {
    #[serde(rename = "retentionSeconds", skip_serializing_if = "Option::is_none")]
    pub retention_seconds: Option<u64>,

    #[serde(rename = "nonceWindowSeconds", skip_serializing_if = "Option::is_none")]
    pub nonce_window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareMode {
    Manual,
    Auto,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AutoUpdateMode {
    Boolean(bool),
    Notify(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct KeybindsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_exit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_open: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollbar_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_export: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_new: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_timeline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_fork: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_rename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_favorite_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_share: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_unshare: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_interrupt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_compact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_page_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_page_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_line_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_line_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_half_page_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_half_page_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_last: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_last_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_copy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_undo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_redo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages_toggle_conceal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_recent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_recent_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_favorite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_cycle_favorite_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_cycle_reverse: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_cycle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_clear: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_paste: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_submit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_newline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_move_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_visual_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_visual_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_visual_line_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_visual_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_buffer_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_buffer_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_buffer_home: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_buffer_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_to_line_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_to_line_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_backspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_undo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_redo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_select_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_word_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_delete_word_backward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_attached_focus: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_workspace_focus: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_attached_open: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_suspend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_title_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tips_toggle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_thinking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_acceleration: Option<ScrollAccelerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ScrollAccelerationConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdns: Option<bool>,
    #[serde(rename = "mdnsDomain", skip_serializing_if = "Option::is_none")]
    pub mdns_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SkillsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    /// Skill names (exact) or `category/*` wildcards filtered out of the skill
    /// catalog at discovery time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub: Option<SkillHubConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillHubRegistrySourceConfig {
    #[serde(rename = "sourceId")]
    pub source_id: String,
    #[serde(rename = "sourceKind", default = "default_registry_source_kind")]
    pub source_kind: agendao_types::SkillSourceKind,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_registry_source_kind() -> agendao_types::SkillSourceKind {
    agendao_types::SkillSourceKind::Registry
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SkillHubConfig {
    #[serde(
        rename = "artifactCacheRetentionSeconds",
        skip_serializing_if = "Option::is_none"
    )]
    pub artifact_cache_retention_seconds: Option<u64>,
    #[serde(rename = "fetchTimeoutMs", skip_serializing_if = "Option::is_none")]
    pub fetch_timeout_ms: Option<u64>,
    #[serde(rename = "maxDownloadBytes", skip_serializing_if = "Option::is_none")]
    pub max_download_bytes: Option<u64>,
    #[serde(rename = "maxExtractBytes", skip_serializing_if = "Option::is_none")]
    pub max_extract_bytes: Option<u64>,
    #[serde(
        rename = "defaultRegistries",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_registries: Option<Vec<SkillHubRegistrySourceConfig>>,
    #[serde(
        rename = "indexFreshnessMaxAgeSeconds",
        skip_serializing_if = "Option::is_none"
    )]
    pub index_freshness_max_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DocsConfig {
    #[serde(
        rename = "contextDocsRegistryPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_docs_registry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WatcherConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfigs {
    #[serde(flatten)]
    pub entries: HashMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Primary,
    Subagent,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub whitelist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blacklist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<HashMap<String, ModelVariantConfig>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// Model-level reasoning effort: none/minimal/low/medium/high.
    /// Validated at resolution time; invalid values are ignored with a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Request-level timeout in seconds for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Streaming stall detection: declare the stream dead after N seconds without a chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_stall_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,
    /// Supports both `true` (boolean) and `{ "field": "reasoning_content" }` (object) forms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<ModelInterleavedConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCostConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimitConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ModelProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelInterleavedConfig {
    Bool(bool),
    Field { field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelModalities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelCostConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_over_200k: Option<Box<ModelCostConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelLimitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelVariantConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// PluginConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Plugin type schema.
    ///
    /// Runtime auto-loading is currently wired for "npm", "file", and "dylib".
    /// Only "npm", "file", and "dylib" are supported.
    #[serde(rename = "type", deserialize_with = "deserialize_plugin_type")]
    pub plugin_type: String,

    /// Package name for package-based plugin specs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,

    /// Version constraint (e.g. "latest", ">=1.0", "0.3.2")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// File path (for type="file")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Runtime override (e.g. "python3.11", "bun")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,

    /// Extra plugin-specific options
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub options: HashMap<String, serde_json::Value>,
}

fn deserialize_plugin_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let plugin_type = String::deserialize(deserializer)?;
    match plugin_type.as_str() {
        "npm" | "file" | "dylib" => Ok(plugin_type),
        _ => Err(serde::de::Error::unknown_variant(
            &plugin_type,
            &["npm", "file", "dylib"],
        )),
    }
}

mod merge;
pub mod plugin;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use plugin::*;
