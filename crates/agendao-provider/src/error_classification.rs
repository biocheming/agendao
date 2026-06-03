use crate::error_code::StandardErrorCode;
use crate::provider::ProviderError;

/// Classify a provider runtime error into a standard V2 error code.
pub fn classify_provider_error(error: &ProviderError) -> StandardErrorCode {
    match error {
        ProviderError::RateLimit => StandardErrorCode::RateLimited,
        ProviderError::Timeout => StandardErrorCode::Timeout,
        ProviderError::AuthError(_) => StandardErrorCode::Authentication,
        ProviderError::ModelNotFound(_) => StandardErrorCode::NotFound,
        ProviderError::ContextOverflow(_) => StandardErrorCode::RequestTooLarge,
        ProviderError::InvalidRequest(_) => StandardErrorCode::InvalidRequest,
        ProviderError::ApiErrorWithStatus {
            message,
            status_code,
        } => {
            if let Some(provider_code) = extract_provider_code_from_message(message) {
                if let Some(std_code) = StandardErrorCode::from_provider_code(&provider_code) {
                    return std_code;
                }
            }
            StandardErrorCode::from_http_status(*status_code)
        }
        ProviderError::ApiError(message) => {
            if let Some(provider_code) = extract_provider_code_from_message(message) {
                if let Some(std_code) = StandardErrorCode::from_provider_code(&provider_code) {
                    return std_code;
                }
            }
            StandardErrorCode::Unknown
        }
        ProviderError::NetworkError(_) => StandardErrorCode::ServerError,
        ProviderError::StreamError(_) => StandardErrorCode::Unknown,
        ProviderError::ProviderNotFound(_) => StandardErrorCode::NotFound,
        ProviderError::ConfigError(_) => StandardErrorCode::InvalidRequest,
    }
}

fn extract_provider_code_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(error) = value.get("error") {
        if let Some(code) = error.get("code").and_then(|v| v.as_str()) {
            return Some(code.to_string());
        }
        if let Some(kind) = error.get("type").and_then(|v| v.as_str()) {
            return Some(kind.to_string());
        }
    }
    None
}

/// Extract provider-specific error code from message body text.
///
/// Handles:
/// - pure JSON payload
/// - `STATUS: <json>` payload
/// - plain text containing known tokens
pub fn extract_provider_code_from_message(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(code) = extract_provider_code_from_json(&value) {
            return Some(code);
        }
    }

    if let Some((_, body)) = trimmed.split_once(": ") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(code) = extract_provider_code_from_json(&value) {
                return Some(code);
            }
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    for token in [
        "context_length_exceeded",
        "insufficient_quota",
        "rate_limit_exceeded",
        "overloaded_error",
        "invalid_request_error",
        "authentication_error",
    ] {
        if lower.contains(token) {
            return Some(token.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_status() {
        let e = ProviderError::api_error_with_status("server", 503);
        assert_eq!(classify_provider_error(&e).code(), "E3002");
    }

    #[test]
    fn classify_by_provider_code() {
        let e = ProviderError::api_error_with_status(
            r#"{"error":{"code":"context_length_exceeded"}}"#,
            400,
        );
        assert_eq!(classify_provider_error(&e).code(), "E1005");
    }
}
