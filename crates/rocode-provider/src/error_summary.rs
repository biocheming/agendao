use crate::diagnostics::{provider_diagnostic_from_error_text, ProviderDiagnosticSummary};
use crate::error_classification::classify_provider_error;
use crate::error_code::StandardErrorCode;
use crate::provider::{format_error_message, is_openai_error_retryable, ProviderError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const PROVIDER_ERROR_SUMMARY_METADATA_KEY: &str = "provider_error_summary";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    ApiErrorWithStatus,
    ApiError,
    NetworkError,
    AuthError,
    RateLimit,
    ModelNotFound,
    InvalidRequest,
    StreamError,
    Timeout,
    ProviderNotFound,
    ConfigError,
    ContextOverflow,
}

impl From<&ProviderError> for ProviderErrorKind {
    fn from(error: &ProviderError) -> Self {
        match error {
            ProviderError::ApiErrorWithStatus { .. } => Self::ApiErrorWithStatus,
            ProviderError::ApiError(_) => Self::ApiError,
            ProviderError::NetworkError(_) => Self::NetworkError,
            ProviderError::AuthError(_) => Self::AuthError,
            ProviderError::RateLimit => Self::RateLimit,
            ProviderError::ModelNotFound(_) => Self::ModelNotFound,
            ProviderError::InvalidRequest(_) => Self::InvalidRequest,
            ProviderError::StreamError(_) => Self::StreamError,
            ProviderError::Timeout => Self::Timeout,
            ProviderError::ProviderNotFound(_) => Self::ProviderNotFound,
            ProviderError::ConfigError(_) => Self::ConfigError,
            ProviderError::ContextOverflow(_) => Self::ContextOverflow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorSummary {
    pub kind: ProviderErrorKind,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    pub standard_code: StandardErrorCode,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<ProviderDiagnosticSummary>,
}

impl ProviderErrorSummary {
    pub fn from_error(provider_id: &str, model_id: Option<&str>, error: &ProviderError) -> Self {
        let raw_message = error.to_string();
        let message = format_error_message(provider_id, error);
        let provider_diagnostic = provider_diagnostic_from_error_text(
            provider_id,
            model_id,
            &raw_message,
        )
        .or_else(|| {
            (raw_message != message)
                .then(|| provider_diagnostic_from_error_text(provider_id, model_id, &message))
                .flatten()
        });

        Self {
            kind: ProviderErrorKind::from(error),
            provider_id: provider_id.to_string(),
            model_id: model_id.map(ToOwned::to_owned),
            status_code: provider_error_status_code(error),
            standard_code: provider_standard_error_code(&message, error),
            retryable: provider_error_retryable(provider_id, error),
            provider_diagnostic,
            message,
        }
    }

    pub fn metadata_value(&self) -> Value {
        serde_json::to_value(self).expect("provider error summary should serialize")
    }

    pub fn attach_to_metadata(&self, metadata: &mut HashMap<String, Value>) {
        metadata.insert(
            PROVIDER_ERROR_SUMMARY_METADATA_KEY.to_string(),
            self.metadata_value(),
        );
    }
}

pub fn summarize_provider_error(
    provider_id: &str,
    model_id: Option<&str>,
    error: &ProviderError,
) -> ProviderErrorSummary {
    ProviderErrorSummary::from_error(provider_id, model_id, error)
}

pub fn provider_error_summary_from_metadata(
    metadata: &HashMap<String, Value>,
) -> Option<ProviderErrorSummary> {
    serde_json::from_value(metadata.get(PROVIDER_ERROR_SUMMARY_METADATA_KEY).cloned()?).ok()
}

fn provider_standard_error_code(message: &str, error: &ProviderError) -> StandardErrorCode {
    if ProviderError::is_overflow(message) {
        StandardErrorCode::RequestTooLarge
    } else {
        classify_provider_error(error)
    }
}

fn provider_error_status_code(error: &ProviderError) -> Option<u16> {
    match error {
        ProviderError::ApiErrorWithStatus { status_code, .. } => Some(*status_code),
        ProviderError::RateLimit => Some(429),
        _ => None,
    }
}

fn provider_error_retryable(provider_id: &str, error: &ProviderError) -> bool {
    match error {
        ProviderError::ApiErrorWithStatus { status_code, .. } => {
            if provider_id.starts_with("openai") {
                is_openai_error_retryable(*status_code)
            } else {
                matches!(status_code, 429 | 500 | 502 | 503 | 504)
            }
        }
        ProviderError::RateLimit | ProviderError::Timeout | ProviderError::NetworkError(_) => true,
        ProviderError::ApiError(_)
        | ProviderError::AuthError(_)
        | ProviderError::ModelNotFound(_)
        | ProviderError::InvalidRequest(_)
        | ProviderError::StreamError(_)
        | ProviderError::ProviderNotFound(_)
        | ProviderError::ConfigError(_)
        | ProviderError::ContextOverflow(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_provider_error_preserves_provider_specific_message() {
        let summary = summarize_provider_error(
            "github-copilot",
            Some("gpt-4.1"),
            &ProviderError::api_error_with_status("Forbidden", 403),
        );

        assert_eq!(summary.kind, ProviderErrorKind::ApiErrorWithStatus);
        assert_eq!(summary.status_code, Some(403));
        assert!(summary.message.contains("reauthenticate"));
    }

    #[test]
    fn summarize_provider_error_detects_retryable_openai_404() {
        let summary = summarize_provider_error(
            "openai",
            Some("gpt-4.1"),
            &ProviderError::api_error_with_status("missing", 404),
        );

        assert_eq!(summary.status_code, Some(404));
        assert!(summary.retryable);
        assert_eq!(summary.standard_code, StandardErrorCode::NotFound);
    }

    #[test]
    fn summarize_provider_error_detects_thinking_replay_diagnostic() {
        let summary = summarize_provider_error(
            "deepseek",
            Some("deepseek-reasoner"),
            &ProviderError::InvalidRequest(
                "provider `deepseek` requires assistant reasoning replay in thinking mode for each prior assistant tool-call continuation, but at least one assistant tool-call turn in request history lacks typed reasoning replay".to_string(),
            ),
        );

        assert_eq!(summary.kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(summary.standard_code, StandardErrorCode::InvalidRequest);
        assert_eq!(
            summary
                .provider_diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.code.as_str()),
            Some("thinking_replay_missing")
        );
    }

    #[test]
    fn summarize_provider_error_promotes_overflow_message_to_request_too_large() {
        let summary = summarize_provider_error(
            "ethnopic",
            Some("claude"),
            &ProviderError::ApiError("prompt is too long".to_string()),
        );

        assert_eq!(summary.kind, ProviderErrorKind::ApiError);
        assert_eq!(summary.standard_code, StandardErrorCode::RequestTooLarge);
        assert!(!summary.retryable);
    }

    #[test]
    fn provider_error_summary_round_trips_via_metadata() {
        let summary = summarize_provider_error(
            "deepseek",
            Some("deepseek-reasoner"),
            &ProviderError::InvalidRequest("bad request".to_string()),
        );
        let mut metadata = HashMap::new();
        summary.attach_to_metadata(&mut metadata);

        let loaded = provider_error_summary_from_metadata(&metadata).expect("summary should load");
        assert_eq!(loaded, summary);
    }
}
