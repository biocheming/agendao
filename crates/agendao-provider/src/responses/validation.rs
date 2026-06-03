use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::message::Message;
use crate::tools::{InputTool, InputToolChoice};

use super::types::{
    FinishReason, MetadataExtractor, ResponseMetadata, ResponsesModelConfig,
    ResponsesProviderOptions, ResponsesUsage, ServiceTier,
};

// ---------------------------------------------------------------------------
// Call Warning Types
// ---------------------------------------------------------------------------

/// Warnings generated during argument preparation.
/// Mirrors TS `LanguageModelV2CallWarning`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CallWarning {
    #[serde(rename = "unsupported-setting")]
    UnsupportedSetting {
        setting: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },
    #[serde(rename = "unsupported-tool")]
    UnsupportedTool {
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    #[serde(rename = "other")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Validation & Warnings
// ---------------------------------------------------------------------------

pub struct ResponsesSettingsValidation<'a> {
    pub model_config: &'a ResponsesModelConfig,
    pub options: &'a ResponsesProviderOptions,
    pub top_k: Option<f32>,
    pub seed: Option<u64>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub stop_sequences: Option<&'a [String]>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

/// Generate warnings for unsupported settings.
/// Mirrors the TS `getArgs()` warning logic.
pub fn validate_responses_settings(input: ResponsesSettingsValidation<'_>) -> Vec<CallWarning> {
    let ResponsesSettingsValidation {
        model_config,
        options,
        top_k,
        seed,
        presence_penalty,
        frequency_penalty,
        stop_sequences,
        temperature,
        top_p,
    } = input;
    let mut warnings = Vec::new();
    let supports_reasoning = model_config.is_reasoning_model
        || options.reasoning_effort.is_some()
        || options.reasoning_summary.is_some();

    if top_k.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "topK".to_string(),
            details: None,
        });
    }
    if seed.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "seed".to_string(),
            details: None,
        });
    }
    if presence_penalty.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "presencePenalty".to_string(),
            details: None,
        });
    }
    if frequency_penalty.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if stop_sequences.is_some() {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "stopSequences".to_string(),
            details: None,
        });
    }

    // Reasoning model validations
    if supports_reasoning {
        if temperature.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "temperature".to_string(),
                details: Some("temperature is not supported for reasoning models".to_string()),
            });
        }
        if top_p.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "topP".to_string(),
                details: Some("topP is not supported for reasoning models".to_string()),
            });
        }
    } else {
        if options.reasoning_effort.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "reasoningEffort".to_string(),
                details: Some(
                    "reasoningEffort is not supported for non-reasoning models".to_string(),
                ),
            });
        }
        if options.reasoning_summary.is_some() {
            warnings.push(CallWarning::UnsupportedSetting {
                setting: "reasoningSummary".to_string(),
                details: Some(
                    "reasoningSummary is not supported for non-reasoning models".to_string(),
                ),
            });
        }
    }

    // Flex processing validation
    if options.service_tier == Some(ServiceTier::Flex) && !model_config.supports_flex_processing {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "serviceTier".to_string(),
            details: Some(
                "flex processing is only available for o3, o4-mini, and gpt-5 models".to_string(),
            ),
        });
    }

    // Priority processing validation
    if options.service_tier == Some(ServiceTier::Priority)
        && !model_config.supports_priority_processing
    {
        warnings.push(CallWarning::UnsupportedSetting {
            setting: "serviceTier".to_string(),
            details: Some(
                "priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported"
                    .to_string(),
            ),
        });
    }

    warnings
}

// ---------------------------------------------------------------------------
// OpenAI Responses Runtime
// ---------------------------------------------------------------------------

pub type UrlBuilder = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;
pub type HeadersBuilder = Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>;
pub type IdGenerator = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone)]
pub struct OpenAIResponsesConfig {
    pub provider: String,
    pub url: UrlBuilder,
    pub headers: HeadersBuilder,
    pub client: Option<Client>,
    pub file_id_prefixes: Option<Vec<String>>,
    pub generate_id: Option<IdGenerator>,
    pub metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
}

impl std::fmt::Debug for OpenAIResponsesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIResponsesConfig")
            .field("provider", &self.provider)
            .field("file_id_prefixes", &self.file_id_prefixes)
            .finish()
    }
}

impl Default for OpenAIResponsesConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            url: Arc::new(|path, _model| format!("https://api.openai.com/v1{}", path)),
            headers: Arc::new(HashMap::new),
            client: None,
            file_id_prefixes: None,
            generate_id: None,
            metadata_extractor: None,
        }
    }
}

#[derive(Clone)]
pub struct OpenAIResponsesLanguageModel {
    pub model_id: String,
    pub config: OpenAIResponsesConfig,
}

impl std::fmt::Debug for OpenAIResponsesLanguageModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIResponsesLanguageModel")
            .field("model_id", &self.model_id)
            .field("config", &self.config)
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub prompt: Vec<Message>,
    pub tools: Option<Vec<InputTool>>,
    pub tool_choice: Option<InputToolChoice>,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<f32>,
    pub seed: Option<u64>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
    pub provider_options: Option<ResponsesProviderOptions>,
    pub response_format: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub generate: GenerateOptions,
}

#[derive(Debug, Clone)]
pub struct PreparedArgs {
    pub web_search_tool_name: Option<String>,
    pub body: Value,
    pub warnings: Vec<CallWarning>,
}

#[derive(Debug, Clone)]
pub struct ResponsesGenerateResult {
    pub message: Message,
    pub finish_reason: FinishReason,
    pub usage: ResponsesUsage,
    pub metadata: ResponseMetadata,
    pub warnings: Vec<CallWarning>,
}
