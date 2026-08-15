use crate::protocol::ProviderConfig;

pub const HEADER_ACCEPT: &str = "Accept";
pub const HEADER_AUTHORIZATION: &str = "Authorization";
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_X_API_KEY: &str = "x-api-key";
pub const HEADER_ANTHROPIC_VERSION: &str = "anthropic-version";

pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const ACCEPT_EVENT_STREAM: &str = "text/event-stream";
pub const ANTHROPIC_VERSION_2023_06_01: &str = "2023-06-01";

pub fn apply_json_content_type(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    builder.header(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON)
}

pub fn apply_sse_accept(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    builder.header(HEADER_ACCEPT, ACCEPT_EVENT_STREAM)
}

pub fn apply_bearer_auth(builder: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    builder.header(HEADER_AUTHORIZATION, format!("Bearer {}", token))
}

pub fn apply_config_headers(
    mut builder: reqwest::RequestBuilder,
    config: &ProviderConfig,
) -> reqwest::RequestBuilder {
    for (key, value) in &config.headers {
        builder = builder.header(key, value);
    }
    builder
}

pub fn apply_messages_api_headers(
    builder: reqwest::RequestBuilder,
    config: &ProviderConfig,
) -> reqwest::RequestBuilder {
    builder
        .header(HEADER_X_API_KEY, &config.api_key)
        .header(HEADER_ANTHROPIC_VERSION, ANTHROPIC_VERSION_2023_06_01)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderConfig;

    fn build_headers(builder: reqwest::RequestBuilder) -> reqwest::header::HeaderMap {
        builder
            .build()
            .expect("request should build")
            .headers()
            .clone()
    }

    #[test]
    fn applies_bearer_auth() {
        let client = reqwest::Client::new();
        let headers = build_headers(apply_bearer_auth(
            client.post("https://example.test"),
            "test-token",
        ));

        assert_eq!(
            headers
                .get(HEADER_AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
    }

    #[test]
    fn applies_config_headers_after_protocol_headers() {
        let client = reqwest::Client::new();
        let config =
            ProviderConfig::new("custom", "", "token").with_header("X-Custom", "custom-value");
        let headers = build_headers(apply_config_headers(
            apply_json_content_type(client.post("https://example.test")),
            &config,
        ));

        assert_eq!(
            headers
                .get("X-Custom")
                .and_then(|value| value.to_str().ok()),
            Some("custom-value")
        );
        assert_eq!(
            headers
                .get(HEADER_CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(CONTENT_TYPE_JSON)
        );
    }

    #[test]
    fn applies_messages_api_headers() {
        let client = reqwest::Client::new();
        let headers = build_headers(apply_messages_api_headers(
            client.post("https://example.test"),
            &ProviderConfig::new("anthropic", "", "api-key"),
        ));

        assert_eq!(
            headers
                .get(HEADER_X_API_KEY)
                .and_then(|value| value.to_str().ok()),
            Some("api-key")
        );
        assert_eq!(
            headers
                .get(HEADER_ANTHROPIC_VERSION)
                .and_then(|value| value.to_str().ok()),
            Some(ANTHROPIC_VERSION_2023_06_01)
        );
    }

    #[test]
    fn normalize_base_url_keeps_existing_version_segment() {
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/v1"),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/v2"),
            "https://api.example.com/v2"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/v1beta"),
            "https://api.example.com/v1beta"
        );
        assert_eq!(
            normalize_provider_base_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn normalize_base_url_appends_v1() {
        assert_eq!(
            normalize_provider_base_url("https://api.kimi.com/coding"),
            "https://api.kimi.com/coding/v1"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.kimi.com/coding"),
            "https://api.kimi.com/coding/v1"
        );
    }

    #[test]
    fn normalize_base_url_strips_trailing_slash() {
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn normalize_base_url_returns_empty_as_is() {
        assert_eq!(normalize_provider_base_url(""), "");
        assert_eq!(normalize_provider_base_url("   "), "   ");
    }

    #[test]
    fn normalize_base_url_ignores_non_version_segments() {
        // "v1beta" 必须完整成段；路径中间的 v1 不算结尾版本段。
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/v1/proxy"),
            "https://api.example.com/v1/proxy/v1"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/vbeta"),
            "https://api.example.com/vbeta/v1"
        );
        assert_eq!(
            normalize_provider_base_url("https://api.example.com/version"),
            "https://api.example.com/version/v1"
        );
    }
}

/// 判断 base URL 是否以 API 版本段结尾（/v1、/v2、/v1beta 等 `/v{digits}[beta]`）。
fn ends_with_version_segment(base: &str) -> bool {
    let last = base.rsplit('/').next().unwrap_or("");
    let Some(rest) = last.strip_prefix('v') else {
        return false;
    };
    let rest = rest.strip_suffix("beta").unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// 归一 provider base URL：确保以协议对应的版本段结尾。
///
/// - base 已以版本段结尾（/v1、/v2、/v1beta 等）→ 原样（仅去掉尾 `/`）；
/// - 否则补 `/v1`；
/// - 空 base（trim 后为空）→ 原样返回，由各适配器走自己的默认 URL。
///
/// 聊天适配器与 `connection_test` 共用本函数，保证测连与实际请求打到同一版本段。
pub fn normalize_provider_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return base_url.to_string();
    }
    let base = trimmed.trim_end_matches('/');
    if ends_with_version_segment(base) {
        return base.to_string();
    }
    format!("{base}/v1")
}

/// 连接测试结果（只读探测，无副作用）。
#[derive(Debug, Clone)]
pub struct ConnectionTestOutcome {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: u128,
    pub error: Option<String>,
}

/// 测试与 provider 的连接：对其 models 端点发轻量 GET，回报 ok/status/延迟/错误。
///
/// 覆盖 OpenAI 族（Bearer）与 Anthropic（x-api-key + version）。
pub async fn connection_test(
    base_url: &str,
    protocol: &str,
    api_key: Option<&str>,
) -> ConnectionTestOutcome {
    // URL 归一：base 已含版本段（/v1 或 /v1beta）直接补 /models；否则按协议补。
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/models") {
        base.to_string()
    } else {
        format!("{}/models", normalize_provider_base_url(base))
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            return ConnectionTestOutcome {
                ok: false,
                status: None,
                latency_ms: 0,
                error: Some(e.to_string()),
            }
        }
    };
    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = match protocol {
            "anthropic" => {
                apply_messages_api_headers(req, &crate::protocol::ProviderConfig::new("", "", key))
            }
            _ => apply_bearer_auth(req, key),
        };
    }

    let t0 = std::time::Instant::now();
    let resp = req.send().await;
    let latency_ms = t0.elapsed().as_millis();
    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            ConnectionTestOutcome {
                ok: r.status().is_success(),
                status: Some(status),
                latency_ms,
                error: None,
            }
        }
        Err(e) => ConnectionTestOutcome {
            ok: false,
            status: None,
            latency_ms,
            error: Some(e.to_string()),
        },
    }
}
