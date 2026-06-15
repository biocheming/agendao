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
}
