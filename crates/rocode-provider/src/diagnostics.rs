use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const PROVIDER_DIAGNOSTIC_METADATA_KEY: &str = "provider_diagnostic";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDiagnosticSeverity {
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDiagnosticSource {
    RequestValidation,
    ApiErrorRewrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDiagnosticSummary {
    pub severity: ProviderDiagnosticSeverity,
    pub source: ProviderDiagnosticSource,
    pub code: String,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub message: String,
}

impl ProviderDiagnosticSummary {
    pub fn metadata_value(&self) -> Value {
        serde_json::to_value(self).expect("provider diagnostic summary should serialize")
    }

    pub fn attach_to_metadata(&self, metadata: &mut HashMap<String, Value>) {
        metadata.insert(
            PROVIDER_DIAGNOSTIC_METADATA_KEY.to_string(),
            self.metadata_value(),
        );
    }
}

pub fn provider_diagnostic_label(summary: &ProviderDiagnosticSummary) -> &'static str {
    match summary.code.as_str() {
        "thinking_replay_missing" => "thinking replay missing",
        "thinking_replay_rejected" => "thinking replay rejected",
        _ => "provider diagnostic",
    }
}

pub fn provider_diagnostic_from_metadata(
    metadata: &HashMap<String, Value>,
) -> Option<ProviderDiagnosticSummary> {
    serde_json::from_value(metadata.get(PROVIDER_DIAGNOSTIC_METADATA_KEY).cloned()?).ok()
}

pub fn provider_diagnostic_from_error_text(
    provider_id: &str,
    model_id: Option<&str>,
    error: &str,
) -> Option<ProviderDiagnosticSummary> {
    let trimmed = error.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("requires assistant reasoning replay in thinking mode") {
        return Some(ProviderDiagnosticSummary {
            severity: ProviderDiagnosticSeverity::HardFail,
            source: ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: provider_id.to_string(),
            model_id: model_id.map(ToOwned::to_owned),
            message: trimmed.to_string(),
        });
    }

    if lower.contains("thinking-mode reasoning replay was missing or incompatible")
        || (lower.contains("reasoning_content")
            && lower.contains("thinking mode")
            && lower.contains("passed back"))
    {
        return Some(ProviderDiagnosticSummary {
            severity: ProviderDiagnosticSeverity::HardFail,
            source: ProviderDiagnosticSource::ApiErrorRewrite,
            code: "thinking_replay_rejected".to_string(),
            provider_id: provider_id.to_string(),
            model_id: model_id.map(ToOwned::to_owned),
            message: trimmed.to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_diagnostic_from_error_text_recognizes_validation_error() {
        let summary = provider_diagnostic_from_error_text(
            "deepseek",
            Some("deepseek-v4"),
            "provider `deepseek` requires assistant reasoning replay in thinking mode for each prior assistant tool-call continuation, but at least one assistant tool-call turn in request history lacks typed reasoning replay",
        )
        .expect("diagnostic should be detected");

        assert_eq!(summary.code, "thinking_replay_missing");
        assert_eq!(summary.source, ProviderDiagnosticSource::RequestValidation);
        assert_eq!(summary.provider_id, "deepseek");
        assert_eq!(summary.model_id.as_deref(), Some("deepseek-v4"));
    }

    #[test]
    fn provider_diagnostic_from_error_text_recognizes_rewritten_api_error() {
        let summary = provider_diagnostic_from_error_text(
            "deepseek",
            Some("deepseek-v4"),
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request",
        )
        .expect("diagnostic should be detected");

        assert_eq!(summary.code, "thinking_replay_rejected");
        assert_eq!(summary.source, ProviderDiagnosticSource::ApiErrorRewrite);
    }

    #[test]
    fn provider_diagnostic_round_trips_via_metadata() {
        let summary = ProviderDiagnosticSummary {
            severity: ProviderDiagnosticSeverity::HardFail,
            source: ProviderDiagnosticSource::RequestValidation,
            code: "thinking_replay_missing".to_string(),
            provider_id: "deepseek".to_string(),
            model_id: Some("deepseek-v4".to_string()),
            message: "missing replay".to_string(),
        };
        let mut metadata = HashMap::new();
        summary.attach_to_metadata(&mut metadata);

        let loaded = provider_diagnostic_from_metadata(&metadata).expect("summary should load");
        assert_eq!(loaded, summary);
    }
}
