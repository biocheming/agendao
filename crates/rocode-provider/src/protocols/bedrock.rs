use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{
    ChatRequest, ChatResponse, Choice, Content, Message, ProtocolImpl, ProviderConfig,
    ProviderError, Role, StreamEvent, StreamResult, Usage,
};

const BEDROCK_RUNTIME_URL: &str = "https://bedrock-runtime.{region}.amazonaws.com";

pub struct BedrockProtocol;

impl Default for BedrockProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl BedrockProtocol {
    pub fn new() -> Self {
        Self
    }

    fn extract_config(config: &ProviderConfig) -> Result<BedrockExtractedConfig, ProviderError> {
        let region = config
            .option_string(&["region"])
            .unwrap_or_else(|| "us-east-1".to_string());
        let access_key_id = config
            .option_string(&["access_key_id", "accessKeyId"])
            .unwrap_or_else(|| config.api_key.clone());
        let secret_access_key = config
            .option_string(&["secret_access_key", "secretAccessKey"])
            .ok_or_else(|| {
                ProviderError::ConfigError(
                    "bedrock requires secret_access_key/secretAccessKey option".to_string(),
                )
            })?;
        let session_token = config.option_string(&["session_token", "sessionToken"]);
        let endpoint_url = if config.base_url.trim().is_empty() {
            config.option_string(&["endpoint", "endpoint_url", "endpointUrl", "endpointURL"])
        } else {
            Some(config.base_url.clone())
        };

        Ok(BedrockExtractedConfig {
            region,
            access_key_id,
            secret_access_key,
            session_token,
            endpoint_url,
        })
    }

    fn get_endpoint(bedrock_config: &BedrockExtractedConfig) -> String {
        if let Some(ref url) = bedrock_config.endpoint_url {
            return url.clone();
        }
        BEDROCK_RUNTIME_URL.replace("{region}", &bedrock_config.region)
    }

    fn convert_request(request: ChatRequest) -> BedrockConverseRequest {
        let mut messages = Vec::new();
        let mut system = Vec::new();

        for msg in request.messages {
            match msg.role {
                Role::System => {
                    if let Content::Text(text) = msg.content {
                        system.push(BedrockSystemContent { text });
                    }
                }
                Role::User => {
                    let content = match msg.content {
                        Content::Text(t) => vec![BedrockContentBlock::text(&t)],
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.as_ref().map(|t| BedrockContentBlock::text(t)))
                            .collect(),
                    };
                    messages.push(BedrockMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
                Role::Assistant => {
                    let content = match msg.content {
                        Content::Text(t) => vec![BedrockContentBlock::text(&t)],
                        Content::Parts(parts) => parts
                            .iter()
                            .filter_map(|p| p.text.as_ref().map(|t| BedrockContentBlock::text(t)))
                            .collect(),
                    };
                    messages.push(BedrockMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
                Role::Tool => {}
            }
        }

        BedrockConverseRequest {
            messages,
            system: if system.is_empty() {
                None
            } else {
                Some(system)
            },
            inference_config: Some(BedrockInferenceConfig {
                max_tokens: request.max_tokens,
                temperature: request.temperature,
            }),
        }
    }

    fn sign_request(
        bedrock_config: &BedrockExtractedConfig,
        method: &str,
        path: &str,
        host: &str,
        body: &[u8],
    ) -> Result<reqwest::header::HeaderMap, ProviderError> {
        let mut headers = reqwest::header::HeaderMap::new();

        let now = chrono::Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();

        let service = "bedrock";
        let region = &bedrock_config.region;

        let body_hash = hex::encode(sha256(body));
        let host_lower = host.to_ascii_lowercase();
        let amz_date_header = amz_date.clone();

        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert("X-Amz-Date", amz_date.parse().unwrap());
        headers.insert("Host", host.parse().unwrap());

        let mut canonical_headers = vec![
            format!("host:{host_lower}"),
            format!("x-amz-date:{amz_date_header}"),
        ];
        let mut signed_headers = vec!["host", "x-amz-date"];

        if let Some(ref token) = bedrock_config.session_token {
            headers.insert("X-Amz-Security-Token", token.parse().unwrap());
            canonical_headers.push(format!("x-amz-security-token:{token}"));
            signed_headers.push("x-amz-security-token");
        }

        let canonical_headers = canonical_headers.join("\n");
        let signed_headers = signed_headers.join(";");
        let canonical_request = format!(
            "{}\n{}\n\n{}\n{}\n{}",
            method, path, canonical_headers, signed_headers, body_hash
        );

        let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date_header,
            credential_scope,
            hex::encode(sha256(canonical_request.as_bytes()))
        );

        let signing_key = get_signature_key(
            &bedrock_config.secret_access_key,
            &date_stamp,
            region,
            service,
        );

        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            bedrock_config.access_key_id, credential_scope, signed_headers, signature
        );

        headers.insert("Authorization", authorization.parse().unwrap());

        Ok(headers)
    }

    fn endpoint_url_and_signing_path(
        endpoint: &str,
        action_path: &str,
        region: &str,
    ) -> (String, String, String) {
        let endpoint = endpoint.trim_end_matches('/');
        let default_host = format!("bedrock-runtime.{region}.amazonaws.com");

        if let Ok(parsed) = Url::parse(endpoint) {
            let host = parsed
                .host_str()
                .map(|h| match parsed.port() {
                    Some(port) => format!("{h}:{port}"),
                    None => h.to_string(),
                })
                .unwrap_or(default_host);
            let prefix = parsed.path().trim_end_matches('/');
            let canonical_path = if prefix.is_empty() || prefix == "/" {
                action_path.to_string()
            } else {
                format!("{prefix}{action_path}")
            };
            (format!("{endpoint}{action_path}"), canonical_path, host)
        } else {
            (
                format!("{endpoint}{action_path}"),
                action_path.to_string(),
                default_host,
            )
        }
    }
}

#[async_trait]
impl ProtocolImpl for BedrockProtocol {
    async fn chat(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let bedrock_config = Self::extract_config(config)?;
        let endpoint = Self::get_endpoint(&bedrock_config);
        let model_id = request.model.clone();
        let model_id_encoded = urlencoding::encode(&model_id);
        let action_path = format!("/model/{}/converse", model_id_encoded);
        let (url, signing_path, signing_host) =
            Self::endpoint_url_and_signing_path(&endpoint, &action_path, &bedrock_config.region);

        let bedrock_request = Self::convert_request(request);
        let body = serde_json::to_vec(&bedrock_request)
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let headers =
            Self::sign_request(&bedrock_config, "POST", &signing_path, &signing_host, &body)?;

        let mut req_builder = client.post(&url).headers(headers).body(body);

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let bedrock_response: BedrockConverseResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        Ok(convert_bedrock_response(bedrock_response))
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        config: &ProviderConfig,
        request: ChatRequest,
    ) -> Result<StreamResult, ProviderError> {
        let bedrock_config = Self::extract_config(config)?;
        let endpoint = Self::get_endpoint(&bedrock_config);
        let model_id = request.model.clone();
        let model_id_encoded = urlencoding::encode(&model_id);
        let action_path = format!("/model/{}/converse-stream", model_id_encoded);
        let (url, signing_path, signing_host) =
            Self::endpoint_url_and_signing_path(&endpoint, &action_path, &bedrock_config.region);

        let bedrock_request = Self::convert_request(request);
        let body = serde_json::to_vec(&bedrock_request)
            .map_err(|e| ProviderError::ApiError(e.to_string()))?;

        let headers =
            Self::sign_request(&bedrock_config, "POST", &signing_path, &signing_host, &body)?;

        let mut req_builder = client
            .post(&url)
            .headers(headers)
            .header("Accept", "application/vnd.amazon.eventstream")
            .body(body);

        for (key, value) in &config.headers {
            req_builder = req_builder.header(key, value);
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|error| format!("<body read failed: {}>", error));
            return Err(ProviderError::ApiError(format!("{}: {}", status, body)));
        }

        let stream = response
            .bytes_stream()
            .map(move |chunk_result| match chunk_result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    parse_bedrock_stream(&text)
                }
                Err(e) => Err(ProviderError::StreamError(e.to_string())),
            });

        Ok(Box::pin(stream))
    }
}

// ---- Internal config ----

struct BedrockExtractedConfig {
    region: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    endpoint_url: Option<String>,
}

// ---- Request/Response types ----

#[derive(Debug, Serialize)]
struct BedrockConverseRequest {
    messages: Vec<BedrockMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<BedrockSystemContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_config: Option<BedrockInferenceConfig>,
}

#[derive(Debug, Serialize)]
struct BedrockMessage {
    role: String,
    content: Vec<BedrockContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BedrockContentBlock {
    text: String,
}

impl BedrockContentBlock {
    fn text(t: &str) -> Self {
        Self {
            text: t.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct BedrockSystemContent {
    text: String,
}

#[derive(Debug, Serialize)]
struct BedrockInferenceConfig {
    #[serde(rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct BedrockConverseResponse {
    output: BedrockOutput,
    usage: BedrockUsage,
}

#[derive(Debug, Deserialize)]
struct BedrockOutput {
    message: BedrockResponseMessage,
}

#[derive(Debug, Deserialize)]
struct BedrockResponseMessage {
    _role: String,
    content: Vec<BedrockContentBlock>,
}

#[derive(Debug, Deserialize)]
struct BedrockUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: Option<u64>,
}

// ---- Crypto helpers ----

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

// ---- Helpers ----

fn convert_bedrock_response(response: BedrockConverseResponse) -> ChatResponse {
    let content = response
        .output
        .message
        .content
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();

    ChatResponse {
        id: format!("bedrock_{}", uuid::Uuid::new_v4()),
        model: "amazon-bedrock".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(&content),
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(Usage {
            prompt_tokens: response.usage.input_tokens,
            completion_tokens: response.usage.output_tokens,
            total_tokens: response
                .usage
                .total_tokens
                .unwrap_or_else(|| response.usage.input_tokens + response.usage.output_tokens),
            cache_read_input_tokens: None,
            cache_miss_input_tokens: None,
            cache_creation_input_tokens: None,
        }),
    }
}

fn parse_bedrock_stream(text: &str) -> Result<StreamEvent, ProviderError> {
    if text.contains("\"contentBlockDelta\"") {
        if let Some(start) = text.find("\"text\":") {
            let rest = &text[start + 8..];
            if let Some(end) = rest.find("\"") {
                let text_content = &rest[..end];
                if let Ok(decoded) =
                    serde_json::from_str::<String>(&format!("\"{}\"", text_content))
                {
                    return Ok(StreamEvent::TextDelta(decoded));
                }
                return Ok(StreamEvent::TextDelta(text_content.to_string()));
            }
        }
    }

    if text.contains("\"messageStop\"") {
        return Ok(StreamEvent::Done);
    }

    Ok(StreamEvent::TextDelta(String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_and_signing_path_respects_custom_endpoint_prefix() {
        let (url, signing_path, signing_host) = BedrockProtocol::endpoint_url_and_signing_path(
            "https://localhost:4566/custom-prefix/",
            "/model/foo/converse",
            "us-west-2",
        );

        assert_eq!(
            url,
            "https://localhost:4566/custom-prefix/model/foo/converse"
        );
        assert_eq!(signing_path, "/custom-prefix/model/foo/converse");
        assert_eq!(signing_host, "localhost:4566");
    }

    #[test]
    fn sign_request_includes_security_token_in_signed_headers() {
        let cfg = BedrockExtractedConfig {
            region: "us-west-2".to_string(),
            access_key_id: "AKIA_TEST".to_string(),
            secret_access_key: "SECRET_TEST".to_string(),
            session_token: Some("SESSION_TOKEN".to_string()),
            endpoint_url: None,
        };

        let headers = BedrockProtocol::sign_request(
            &cfg,
            "POST",
            "/model/foo/converse",
            "localhost:4566",
            b"{\"ok\":true}",
        )
        .expect("signing should succeed");

        let auth = headers
            .get("Authorization")
            .expect("authorization header should exist")
            .to_str()
            .expect("authorization header should be valid utf-8");
        assert!(auth.contains("SignedHeaders=host;x-amz-date;x-amz-security-token"));
        assert_eq!(
            headers
                .get("X-Amz-Security-Token")
                .expect("session token header should exist"),
            "SESSION_TOKEN"
        );
        assert_eq!(
            headers
                .get("Host")
                .expect("host header should exist")
                .to_str()
                .unwrap(),
            "localhost:4566"
        );
    }
}
