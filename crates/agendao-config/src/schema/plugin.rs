use super::*;
use serde::Deserialize;
use std::collections::HashMap;

pub fn deserialize_plugin_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, PluginConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PluginField {
        Map(HashMap<String, PluginConfig>),
        List(Vec<String>),
    }

    match PluginField::deserialize(deserializer)? {
        PluginField::Map(map) => Ok(map),
        PluginField::List(list) => {
            let mut map = HashMap::new();
            for spec in list {
                let (key, config) = legacy_spec_to_plugin_config(&spec);
                map.entry(key).or_insert(config);
            }
            Ok(map)
        }
    }
}

/// Convert a legacy string spec (e.g. "oh-my-opencode@latest") to a PluginConfig.
fn legacy_spec_to_plugin_config(spec: &str) -> (String, PluginConfig) {
    PluginConfig::from_legacy_spec(spec)
}

/// Parse "pkg@version" into (name, version). Handles scoped packages like "@scope/pkg@1.0".
fn parse_npm_spec(spec: &str) -> (&str, &str) {
    if let Some(stripped) = spec.strip_prefix('@') {
        if let Some(idx) = stripped.find('@') {
            let split = idx + 1;
            return (&spec[..split], &spec[split + 1..]);
        }
        return (spec, "*");
    }
    if let Some(idx) = spec.find('@') {
        return (&spec[..idx], &spec[idx + 1..]);
    }
    (spec, "*")
}

impl PluginConfig {
    /// Create a file-type plugin from a `file://path` spec string.
    pub fn from_file_spec(spec: &str) -> (String, Self) {
        let path = spec.strip_prefix("file://").unwrap_or(spec);
        let name = std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "plugin".to_string());
        (
            name,
            Self {
                plugin_type: "file".to_string(),
                path: Some(path.to_string()),
                ..Default::default()
            },
        )
    }

    /// Create an npm-type plugin from a spec like "oh-my-opencode@latest".
    pub fn from_npm_spec(spec: &str) -> (String, Self) {
        let (pkg_name, version) = parse_npm_spec(spec);
        let key = pkg_name.trim_start_matches('@').replace('/', "-");
        (
            key,
            Self {
                plugin_type: "npm".to_string(),
                package: Some(pkg_name.to_string()),
                version: if version != "*" {
                    Some(version.to_string())
                } else {
                    None
                },
                ..Default::default()
            },
        )
    }

    /// Convert a legacy string spec to a PluginConfig entry.
    pub fn from_legacy_spec(spec: &str) -> (String, Self) {
        if spec.starts_with("file://") {
            Self::from_file_spec(spec)
        } else {
            Self::from_npm_spec(spec)
        }
    }

    /// Create a dylib-type plugin from a shared library path.
    pub fn from_dylib_path(path: &str) -> (String, Self) {
        let name = std::path::Path::new(path)
            .file_stem()
            .map(|s| {
                let s = s.to_string_lossy();
                // Strip common lib prefix (libfoo.so -> foo)
                s.strip_prefix("lib").unwrap_or(&s).to_string()
            })
            .unwrap_or_else(|| "native-plugin".to_string());
        (
            name,
            Self {
                plugin_type: "dylib".to_string(),
                path: Some(path.to_string()),
                ..Default::default()
            },
        )
    }

    /// Convert this config back to a subprocess-loader-compatible spec string.
    ///
    /// Only the current subprocess-backed runtime paths (`npm` and `file`) map to
    /// loader specs. Types such as `pip`, `cargo`, and `dylib` are accepted by the
    /// schema but do not flow through this loader path today.
    pub fn to_loader_spec(&self, name: &str) -> Option<String> {
        match self.plugin_type.as_str() {
            "npm" => {
                let pkg = self.package.as_deref().unwrap_or(name);
                if let Some(ver) = &self.version {
                    Some(format!("{pkg}@{ver}"))
                } else {
                    Some(pkg.to_string())
                }
            }
            "file" => self.path.as_ref().map(|p| format!("file://{p}")),
            _ => None,
        }
    }

    /// Whether this plugin should be loaded as a native dylib (in-process).
    pub fn is_native(&self) -> bool {
        self.plugin_type == "dylib"
    }

    /// Return the dylib path if this is a native plugin.
    pub fn dylib_path(&self) -> Option<&str> {
        if self.is_native() {
            self.path.as_deref()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Enabled { enabled: bool },
    Full(Box<McpServer>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServer {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub server_type: Option<String>,

    /// For local: command array; for remote: unused
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,

    /// For local: environment variables
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,

    /// For remote: URL of the MCP server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    /// For remote: headers to send
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// For remote: OAuth config (or false to disable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,

    // Legacy fields kept for backward compatibility
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
}

/// OAuth configuration for remote MCP servers.
/// Can be a full config object or `false` to disable OAuth auto-detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpOAuthConfig {
    Disabled(bool),
    Config(McpOAuth),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpOAuth {
    #[serde(
        rename = "clientId",
        alias = "client_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_id: Option<String>,
    #[serde(
        rename = "clientSecret",
        alias = "client_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormatterConfig {
    Disabled(bool),
    Enabled(HashMap<String, FormatterEntry>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FormatterEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LspConfig {
    Disabled(bool),
    Enabled(HashMap<String, LspServerConfig>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspServerConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialization: Option<HashMap<String, serde_json::Value>>,
}

/// Layout mode: "auto" or "stretch" (TS: z.enum(["auto", "stretch"]))
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    Auto,
    Stretch,
}

/// Permission config: a record of tool name -> permission rule.
/// Each rule can be a simple action string ("ask"/"allow"/"deny") or
/// a record of sub-keys to actions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionConfig {
    #[serde(flatten)]
    pub rules: HashMap<String, PermissionRule>,
}

/// A permission rule: either a simple action or a map of sub-keys to actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionRule {
    Action(PermissionAction),
    Object(HashMap<String, PermissionAction>),
}

/// Permission action: "ask", "allow", or "deny".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    Ask,
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnterpriseConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_config_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExperimentalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_paste_summary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_tool: Option<bool>,
    #[serde(alias = "openTelemetry", skip_serializing_if = "Option::is_none")]
    pub open_telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continue_loop_on_deny: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_timeout: Option<u64>,
}
