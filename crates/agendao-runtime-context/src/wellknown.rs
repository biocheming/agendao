use agendao_config::{load_config, Config};
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
struct WellKnownResponse {
    #[serde(default)]
    config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct WellKnownAuth {
    key: String,
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum AuthEntry {
    #[serde(rename = "wellknown")]
    WellKnown { key: String, token: String },
    #[serde(other)]
    Other,
}

struct CacheEntry {
    config: Config,
    fetched_at: Instant,
}

static CACHE: Mutex<Option<HashMap<String, CacheEntry>>> = Mutex::new(None);

fn auth_json_path() -> PathBuf {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("opencode");
    data_dir.join("auth.json")
}

fn read_wellknown_entries() -> HashMap<String, WellKnownAuth> {
    let path = auth_json_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let raw: HashMap<String, serde_json::Value> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut result = HashMap::new();
    for (url, value) in raw {
        if let Ok(AuthEntry::WellKnown { key, token }) = serde_json::from_value::<AuthEntry>(value)
        {
            result.insert(url, WellKnownAuth { key, token });
        }
    }
    result
}

pub async fn load_wellknown() -> Config {
    let entries = read_wellknown_entries();
    if entries.is_empty() {
        return Config::default();
    }

    let mut merged = Config::default();
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .unwrap_or_default();

    for (url, auth) in &entries {
        if let Some(cached) = get_cached(url) {
            tracing::debug!(url = %url, "using cached wellknown config");
            merged.merge(cached);
            continue;
        }

        let endpoint = format!("{}/.well-known/opencode", url.trim_end_matches('/'));
        tracing::debug!(url = %endpoint, "fetching remote wellknown config");

        match fetch_wellknown_config(&client, &endpoint).await {
            Ok(mut config) => {
                apply_wellknown_auth_to_config(&mut config, auth);
                set_cached(url.clone(), config.clone());
                tracing::debug!(url = %url, "loaded remote config from well-known");
                merged.merge(config);
            }
            Err(e) => {
                tracing::warn!(url = %endpoint, error = %e, "failed to fetch wellknown config, skipping");
            }
        }
    }

    merged
}

pub async fn load_config_with_remote<P: AsRef<std::path::Path>>(project_dir: P) -> Result<Config> {
    let mut config = load_wellknown().await;
    config.merge(load_config(project_dir)?);
    Ok(config)
}

fn apply_wellknown_auth_to_config(config: &mut Config, auth: &WellKnownAuth) {
    let Some(providers) = config.provider.as_mut() else {
        return;
    };

    for provider in providers.values_mut() {
        let matches_auth_env = provider
            .env
            .as_ref()
            .map(|vars| vars.iter().any(|name| name == &auth.key))
            .unwrap_or(false);
        if !matches_auth_env || provider.api_key.is_some() {
            continue;
        }
        provider.api_key = Some(auth.token.clone());
    }
}

async fn fetch_wellknown_config(client: &reqwest::Client, endpoint: &str) -> Result<Config> {
    let resp = client.get(endpoint).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}", resp.status(), endpoint);
    }

    let wk: WellKnownResponse = resp.json().await?;
    let config_value = wk
        .config
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let config: Config = serde_json::from_value(config_value)?;
    Ok(config)
}

fn get_cached(url: &str) -> Option<Config> {
    let guard = CACHE.lock().ok()?;
    let map = guard.as_ref()?;
    let entry = map.get(url)?;
    if entry.fetched_at.elapsed() < CACHE_TTL {
        Some(entry.config.clone())
    } else {
        None
    }
}

fn set_cached(url: String, config: Config) {
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(
        url,
        CacheEntry {
            config,
            fetched_at: Instant::now(),
        },
    );
}

pub fn clear_wellknown_cache() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wellknown_auth_entries_from_json() {
        let json = r#"{
            "https://corp.example.com": {
                "type": "wellknown",
                "key": "CORP_TOKEN",
                "token": "secret-123"
            },
            "ethnopic": {
                "type": "api",
                "key": "sk-ant-xxx"
            }
        }"#;

        let raw: HashMap<String, serde_json::Value> = serde_json::from_str(json).unwrap();
        let mut result = HashMap::new();
        for (url, value) in raw {
            if let Ok(AuthEntry::WellKnown { key, token }) =
                serde_json::from_value::<AuthEntry>(value)
            {
                result.insert(url, WellKnownAuth { key, token });
            }
        }

        assert_eq!(result.len(), 1);
        let entry = result.get("https://corp.example.com").unwrap();
        assert_eq!(entry.key, "CORP_TOKEN");
        assert_eq!(entry.token, "secret-123");
    }

    #[test]
    fn wellknown_response_parses_config_field() {
        let json = r#"{
            "config": {
                "model": "gpt-5"
            }
        }"#;

        let parsed: WellKnownResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.config,
            Some(serde_json::json!({
                "model": "gpt-5"
            }))
        );
    }

    #[test]
    fn wellknown_response_handles_missing_config() {
        let json = r#"{}"#;
        let parsed: WellKnownResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.config.is_none());
    }

    #[test]
    fn apply_wellknown_auth_injects_matching_provider_api_key() {
        let mut config = Config {
            provider: Some(HashMap::from([(
                "corp".to_string(),
                agendao_config::ProviderConfig {
                    env: Some(vec!["CORP_TOKEN".to_string()]),
                    api_key: None,
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        apply_wellknown_auth_to_config(
            &mut config,
            &WellKnownAuth {
                key: "CORP_TOKEN".to_string(),
                token: "secret-123".to_string(),
            },
        );

        let provider = config
            .provider
            .as_ref()
            .and_then(|providers| providers.get("corp"))
            .unwrap();
        assert_eq!(provider.api_key.as_deref(), Some("secret-123"));
    }

    #[test]
    fn apply_wellknown_auth_does_not_override_explicit_provider_api_key() {
        let mut config = Config {
            provider: Some(HashMap::from([(
                "corp".to_string(),
                agendao_config::ProviderConfig {
                    env: Some(vec!["CORP_TOKEN".to_string()]),
                    api_key: Some("already-set".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        apply_wellknown_auth_to_config(
            &mut config,
            &WellKnownAuth {
                key: "CORP_TOKEN".to_string(),
                token: "secret-123".to_string(),
            },
        );

        let provider = config
            .provider
            .as_ref()
            .and_then(|providers| providers.get("corp"))
            .unwrap();
        assert_eq!(provider.api_key.as_deref(), Some("already-set"));
    }

    #[tokio::test]
    async fn load_wellknown_returns_default_when_no_auth_file() {
        clear_wellknown_cache();
        let config = load_wellknown().await;
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::to_value(Config::default()).unwrap()
        );
    }
}
