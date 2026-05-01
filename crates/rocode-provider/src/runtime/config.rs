use crate::ProviderConfig;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub enabled: bool,
    pub preflight_enabled: bool,
    pub pipeline_enabled: bool,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_secs: u64,
    pub rate_limit_rps: f64,
    pub max_inflight: u32,
    pub protocol_path: Option<String>,
    pub protocol_version: Option<String>,
    pub hot_reload: bool,
}

pub fn runtime_pipeline_enabled(config: &ProviderConfig) -> bool {
    config
        .option_bool(&["runtime_pipeline"])
        .unwrap_or_else(|| {
            std::env::var("ROCODE_RUNTIME_PIPELINE")
                .ok()
                .and_then(|v| {
                    let lower = v.trim().to_ascii_lowercase();
                    if matches!(lower.as_str(), "1" | "true" | "yes" | "on") {
                        Some(true)
                    } else if matches!(lower.as_str(), "0" | "false" | "no" | "off") {
                        Some(false)
                    } else {
                        None
                    }
                })
                .unwrap_or(true)
        })
}

#[cfg(test)]
mod tests {
    use super::runtime_pipeline_enabled;
    use crate::ProviderConfig;
    use serde_json::Value;

    #[test]
    fn runtime_pipeline_option_disables_pipeline() {
        let mut config = ProviderConfig::new("openai", "", "");
        config
            .options
            .insert("runtime_pipeline".to_string(), Value::Bool(false));
        assert!(!runtime_pipeline_enabled(&config));
    }

    #[test]
    fn runtime_pipeline_option_enables_pipeline() {
        let mut config = ProviderConfig::new("openai", "", "");
        config
            .options
            .insert("runtime_pipeline".to_string(), Value::Bool(true));
        assert!(runtime_pipeline_enabled(&config));
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preflight_enabled: false,
            pipeline_enabled: true,
            circuit_breaker_threshold: 0,
            circuit_breaker_cooldown_secs: 30,
            rate_limit_rps: 0.0,
            max_inflight: 0,
            protocol_path: None,
            protocol_version: None,
            hot_reload: false,
        }
    }
}
