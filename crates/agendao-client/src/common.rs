use std::time::Duration;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use agendao_api::ConnectProviderRequest;
use agendao_state::RecentModelEntry;

pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RecentModelsPayload {
    #[serde(default)]
    pub(crate) recent_models: Vec<RecentModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LspStatusResponse {
    pub(crate) servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FormatterStatusResponse {
    pub(crate) formatters: Vec<String>,
}

pub(crate) fn server_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) fn build_session_list_params_with_directory(
    directory: Option<&str>,
    search: Option<&str>,
    limit: Option<usize>,
) -> Vec<(&'static str, String)> {
    let mut params = Vec::new();
    if let Some(directory) = directory.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("directory", directory.to_string()));
    }
    if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("search", search.to_string()));
    }
    if let Some(limit) = limit.filter(|value| *value > 0) {
        params.push(("limit", limit.to_string()));
    }
    params
}

pub(crate) fn build_connect_provider_request(
    provider_id: &str,
    api_key: &str,
    base_url: Option<String>,
    protocol: Option<String>,
) -> ConnectProviderRequest {
    ConnectProviderRequest {
        provider_id: provider_id.to_string(),
        api_key: api_key.to_string(),
        base_url,
        protocol,
    }
}

/// PUT `/provider/{id}` 请求体。字段名 / 形态与 server `UpdateProviderRequest`
/// (provider.rs:1695)同源:`base_url` 与 `protocol` server 端强制成对(任一
/// 为 `Some` 必须两者都 `Some`);`name` 独立可选。改字段时两边同步——
/// 这是跨 crate 共享的 wire contract,土律·第四条单点权威要求未来收归到
/// `agendao-api` crate,目前 server 端尚未抽离,先在 client 侧自维护一份。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateProviderWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) protocol: Option<String>,
}

pub(crate) fn build_update_provider_request(
    name: Option<&str>,
    base_url: Option<&str>,
    protocol: Option<&str>,
) -> UpdateProviderWire {
    UpdateProviderWire {
        name: name.map(str::to_string),
        base_url: base_url.map(str::to_string),
        protocol: protocol.map(str::to_string),
    }
}

pub(crate) fn http_error(action: &str, status: reqwest::StatusCode, text: String) -> anyhow::Error {
    anyhow!("Failed to {}: {} - {}", action, status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_param_is_emitted_when_provided() {
        let params = build_session_list_params_with_directory(
            Some("/home/me/proj"),
            None,
            None,
        );
        assert_eq!(params, vec![("directory", "/home/me/proj".to_string())]);
    }

    #[test]
    fn directory_param_skipped_when_none_or_empty() {
        assert!(build_session_list_params_with_directory(None, None, None).is_empty());
        assert!(build_session_list_params_with_directory(Some(""), None, None).is_empty());
        assert!(build_session_list_params_with_directory(Some("   "), None, None).is_empty());
    }

    #[test]
    fn directory_search_limit_compose_in_order() {
        let params = build_session_list_params_with_directory(
            Some("/p"),
            Some("hello"),
            Some(50),
        );
        assert_eq!(
            params,
            vec![
                ("directory", "/p".to_string()),
                ("search", "hello".to_string()),
                ("limit", "50".to_string()),
            ]
        );
    }

    #[test]
    fn update_provider_request_skips_none_fields() {
        // 仅 name → JSON 不含 base_url / protocol(server 端可只改 name)。
        let req = build_update_provider_request(Some("My OpenAI"), None, None);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json, serde_json::json!({"name": "My OpenAI"}));
    }

    #[test]
    fn update_provider_request_serializes_all_fields() {
        let req = build_update_provider_request(
            Some("My OpenAI"),
            Some("https://api.x.com/v1"),
            Some("openai"),
        );
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "My OpenAI",
                "base_url": "https://api.x.com/v1",
                "protocol": "openai",
            })
        );
    }
}
