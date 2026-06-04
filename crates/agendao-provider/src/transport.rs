use crate::profile::ProviderTransportKind;
use crate::protocol::ProviderConfig;
use crate::provider::ProviderError;

pub const HEADER_ACCEPT: &str = "Accept";
pub const HEADER_AUTHORIZATION: &str = "Authorization";
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_COPILOT_INTEGRATION_ID: &str = "Copilot-Integration-Id";
pub const HEADER_PRIVATE_TOKEN: &str = "PRIVATE-TOKEN";
pub const HEADER_X_API_KEY: &str = "x-api-key";
pub const HEADER_ANTHROPIC_VERSION: &str = "anthropic-version";

pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const ACCEPT_EVENT_STREAM: &str = "text/event-stream";
pub const COPILOT_INTEGRATION_VSCODE_CHAT: &str = "vscode-chat";
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

pub fn apply_private_token_auth(
    builder: reqwest::RequestBuilder,
    token: &str,
) -> reqwest::RequestBuilder {
    builder.header(HEADER_PRIVATE_TOKEN, token)
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

pub fn apply_transport_auth(
    kind: ProviderTransportKind,
    builder: reqwest::RequestBuilder,
    config: &ProviderConfig,
) -> reqwest::RequestBuilder {
    match kind {
        ProviderTransportKind::Bearer
        | ProviderTransportKind::VertexBearer
        | ProviderTransportKind::OAuth => apply_bearer_auth(builder, &config.api_key),
        ProviderTransportKind::PrivateToken => apply_private_token_auth(builder, &config.api_key),
        ProviderTransportKind::SigV4
        | ProviderTransportKind::HeaderSet
        | ProviderTransportKind::Custom => builder,
    }
}

pub fn apply_copilot_headers(
    builder: reqwest::RequestBuilder,
    config: &ProviderConfig,
) -> reqwest::RequestBuilder {
    apply_transport_auth(ProviderTransportKind::OAuth, builder, config).header(
        HEADER_COPILOT_INTEGRATION_ID,
        COPILOT_INTEGRATION_VSCODE_CHAT,
    )
}

pub fn apply_messages_api_headers(
    builder: reqwest::RequestBuilder,
    config: &ProviderConfig,
) -> reqwest::RequestBuilder {
    builder
        .header(HEADER_X_API_KEY, &config.api_key)
        .header(HEADER_ANTHROPIC_VERSION, ANTHROPIC_VERSION_2023_06_01)
}

#[derive(Debug, Clone, Copy)]
pub struct AwsSigV4Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub session_token: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AwsSigV4Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub host: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub body: &'a [u8],
}

pub fn sign_aws_sigv4_json_request(
    credentials: AwsSigV4Credentials<'_>,
    request: AwsSigV4Request<'_>,
) -> Result<reqwest::header::HeaderMap, ProviderError> {
    let mut headers = reqwest::header::HeaderMap::new();

    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let body_hash = hex::encode(sha256(request.body));
    let host_lower = request.host.to_ascii_lowercase();
    let amz_date_header = amz_date.clone();

    headers.insert(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON.parse().unwrap());
    headers.insert("X-Amz-Date", amz_date.parse().unwrap());
    headers.insert("Host", request.host.parse().unwrap());

    let mut canonical_headers = vec![
        format!("host:{host_lower}"),
        format!("x-amz-date:{amz_date_header}"),
    ];
    let mut signed_headers = vec!["host", "x-amz-date"];

    if let Some(token) = credentials.session_token {
        headers.insert("X-Amz-Security-Token", token.parse().unwrap());
        canonical_headers.push(format!("x-amz-security-token:{token}"));
        signed_headers.push("x-amz-security-token");
    }

    let canonical_headers = canonical_headers.join("\n");
    let signed_headers = signed_headers.join(";");
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        request.method, request.path, canonical_headers, signed_headers, body_hash
    );

    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, request.region, request.service
    );

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date_header,
        credential_scope,
        hex::encode(sha256(canonical_request.as_bytes()))
    );

    let signing_key = get_signature_key(
        credentials.secret_access_key,
        &date_stamp,
        request.region,
        request.service,
    );
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        credentials.access_key_id, credential_scope, signed_headers, signature
    );

    headers.insert(HEADER_AUTHORIZATION, authorization.parse().unwrap());

    Ok(headers)
}

fn sha256(data: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn get_signature_key(key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{}", key).as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
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
        let headers = build_headers(apply_transport_auth(
            ProviderTransportKind::Bearer,
            client.post("https://example.test"),
            &ProviderConfig::new("openai", "", "test-token"),
        ));

        assert_eq!(
            headers
                .get(HEADER_AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
    }

    #[test]
    fn applies_private_token_auth() {
        let client = reqwest::Client::new();
        let headers = build_headers(apply_transport_auth(
            ProviderTransportKind::PrivateToken,
            client.post("https://example.test"),
            &ProviderConfig::new("gitlab", "", "gitlab-token"),
        ));

        assert_eq!(
            headers
                .get(HEADER_PRIVATE_TOKEN)
                .and_then(|value| value.to_str().ok()),
            Some("gitlab-token")
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
    fn applies_copilot_headers() {
        let client = reqwest::Client::new();
        let headers = build_headers(apply_copilot_headers(
            client.post("https://example.test"),
            &ProviderConfig::new("github-copilot", "", "oauth-token"),
        ));

        assert_eq!(
            headers
                .get(HEADER_AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer oauth-token")
        );
        assert_eq!(
            headers
                .get(HEADER_COPILOT_INTEGRATION_ID)
                .and_then(|value| value.to_str().ok()),
            Some(COPILOT_INTEGRATION_VSCODE_CHAT)
        );
    }

    #[test]
    fn applies_messages_api_headers() {
        let client = reqwest::Client::new();
        let headers = build_headers(apply_messages_api_headers(
            client.post("https://example.test"),
            &ProviderConfig::new("ethnopic", "", "api-key"),
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
    fn sigv4_signing_includes_session_token_in_signed_headers() {
        let headers = sign_aws_sigv4_json_request(
            AwsSigV4Credentials {
                access_key_id: "AKIA_TEST",
                secret_access_key: "secret",
                session_token: Some("session-token"),
            },
            AwsSigV4Request {
                method: "POST",
                path: "/model/test/converse",
                host: "bedrock-runtime.us-east-1.amazonaws.com",
                region: "us-east-1",
                service: "bedrock",
                body: br#"{"messages":[]}"#,
            },
        )
        .expect("headers should sign");

        let auth = headers
            .get(HEADER_AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .expect("authorization header should exist");

        assert!(auth.contains("AWS4-HMAC-SHA256 Credential=AKIA_TEST/"));
        assert!(auth.contains("SignedHeaders=host;x-amz-date;x-amz-security-token"));
        assert_eq!(
            headers
                .get("X-Amz-Security-Token")
                .and_then(|value| value.to_str().ok()),
            Some("session-token")
        );
    }
}
