use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ProtocolManifest {
    pub id: String,
    pub protocol_version: String,
    pub endpoint: EndpointConfig,
    pub streaming: Option<StreamingConfig>,
    #[serde(default)]
    pub capabilities: HashMap<String, bool>,
    #[serde(default)]
    pub retry_policy: Option<serde_json::Value>,
    #[serde(default)]
    pub rate_limit_headers: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EndpointConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamingConfig {
    #[serde(default)]
    pub content_path: Option<String>,
    #[serde(default)]
    pub tool_call_path: Option<String>,
    #[serde(default)]
    pub usage_path: Option<String>,
    #[serde(default)]
    pub decoder: DecoderConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DecoderConfig {
    #[serde(default)]
    pub delimiter: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub done_signal: Option<String>,
}

pub struct ProtocolLoader {
    base_path: Option<PathBuf>,
}

impl Default for ProtocolLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolLoader {
    pub fn new() -> Self {
        Self { base_path: None }
    }

    pub fn with_base_path(mut self, path: impl AsRef<Path>) -> Self {
        self.base_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn try_load_provider(
        &self,
        provider_id: &str,
        options: &HashMap<String, serde_json::Value>,
    ) -> Option<ProtocolManifest> {
        if let Some(path) = options.get("protocol_path").and_then(|v| v.as_str()) {
            if let Ok(manifest) = self.load_from_file(Path::new(path)) {
                return Some(manifest);
            }
            if let Some(base) = &self.base_path {
                let joined = base.join(path);
                if let Ok(manifest) = self.load_from_file(&joined) {
                    return Some(manifest);
                }
            }
        }

        if let Ok(root) =
            std::env::var("AI_PROTOCOL_DIR").or_else(|_| std::env::var("AI_PROTOCOL_PATH"))
        {
            let path = PathBuf::from(root)
                .join("dist")
                .join("v1")
                .join("providers")
                .join(format!("{provider_id}.json"));
            if let Ok(manifest) = self.load_from_json_file(&path) {
                return Some(manifest);
            }
        }

        if let Some(base) = &self.base_path {
            let path = base
                .join("dist")
                .join("v1")
                .join("providers")
                .join(format!("{provider_id}.json"));
            if let Ok(manifest) = self.load_from_json_file(&path) {
                return Some(manifest);
            }
        }

        for root in ["ai-protocol", "../ai-protocol", "../../ai-protocol"] {
            let path = PathBuf::from(root)
                .join("dist")
                .join("v1")
                .join("providers")
                .join(format!("{provider_id}.json"));
            if let Ok(manifest) = self.load_from_json_file(&path) {
                return Some(manifest);
            }
        }

        None
    }

    pub fn load_from_file(&self, path: &Path) -> Result<ProtocolManifest, ProtocolLoadError> {
        match path.extension().and_then(|s| s.to_str()) {
            Some("json") => self.load_from_json_file(path),
            _ => Err(ProtocolLoadError::UnsupportedFormat(
                path.to_string_lossy().to_string(),
            )),
        }
    }

    pub fn load_from_json_file(&self, path: &Path) -> Result<ProtocolManifest, ProtocolLoadError> {
        let bytes = std::fs::read(path).map_err(|e| ProtocolLoadError::Io {
            path: path.to_string_lossy().to_string(),
            reason: e.to_string(),
        })?;
        serde_json::from_slice(&bytes).map_err(|e| ProtocolLoadError::Parse {
            path: path.to_string_lossy().to_string(),
            reason: e.to_string(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolLoadError {
    #[error("protocol IO error at {path}: {reason}")]
    Io { path: String, reason: String },

    #[error("protocol parse error at {path}: {reason}")]
    Parse { path: String, reason: String },

    #[error("unsupported protocol format: {0}")]
    UnsupportedFormat(String),
}
