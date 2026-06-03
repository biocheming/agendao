pub mod lifecycle;

use std::collections::HashMap;

use agendao_provider::{ChatRequest, Message, ToolDefinition};

pub use lifecycle::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionModelRef {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledExecutionRequest {
    pub model_id: String,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionRequestContext {
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionRequestDefaults {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub variant: Option<String>,
    pub provider_options: Option<HashMap<String, serde_json::Value>>,
}

impl CompiledExecutionRequest {
    pub fn max_tokens_or(&self, default: u64) -> u64 {
        self.max_tokens.unwrap_or(default)
    }

    pub fn with_model(&self, model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            ..self.clone()
        }
    }

    pub fn with_variant(&self, variant: Option<String>) -> Self {
        Self {
            variant,
            ..self.clone()
        }
    }

    pub fn with_default_max_tokens(&self, default: u64) -> Self {
        Self {
            max_tokens: Some(self.max_tokens_or(default)),
            ..self.clone()
        }
    }

    pub fn inherit_missing(&self, defaults: &ExecutionRequestDefaults) -> Self {
        Self {
            model_id: self.model_id.clone(),
            max_tokens: self.max_tokens.or(defaults.max_tokens),
            temperature: self.temperature.or(defaults.temperature),
            top_p: self.top_p.or(defaults.top_p),
            variant: self.variant.clone().or_else(|| defaults.variant.clone()),
            provider_options: self
                .provider_options
                .clone()
                .or_else(|| defaults.provider_options.clone()),
        }
    }

    pub fn to_chat_request(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        stream: bool,
    ) -> ChatRequest {
        self.to_chat_request_with_system(messages, tools, Some(stream), None)
    }

    pub fn to_chat_request_with_system(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        stream: Option<bool>,
        system: Option<String>,
    ) -> ChatRequest {
        ChatRequest {
            model: self.model_id.clone(),
            messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            top_p: self.top_p,
            system,
            tools: (!tools.is_empty()).then_some(tools),
            stream,
            provider_options: self.provider_options.clone(),
            variant: self.variant.clone(),
        }
    }
}

impl ExecutionRequestContext {
    pub fn model_ref(&self) -> Option<ExecutionModelRef> {
        Some(ExecutionModelRef {
            provider_id: self.provider_id.clone()?,
            model_id: self.model_id.clone()?,
        })
    }

    pub fn compile(&self) -> Option<CompiledExecutionRequest> {
        Some(CompiledExecutionRequest {
            model_id: self.model_id.clone()?,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            top_p: self.top_p,
            variant: self.variant.clone(),
            provider_options: self.provider_options.clone(),
        })
    }

    pub fn compile_with_model(&self, model_id: impl Into<String>) -> CompiledExecutionRequest {
        CompiledExecutionRequest {
            model_id: model_id.into(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            top_p: self.top_p,
            variant: self.variant.clone(),
            provider_options: self.provider_options.clone(),
        }
    }

    pub fn compile_with_model_and_defaults(
        &self,
        model_id: impl Into<String>,
        defaults: &ExecutionRequestDefaults,
    ) -> CompiledExecutionRequest {
        self.compile_with_model(model_id).inherit_missing(defaults)
    }
}

impl ExecutionRequestDefaults {
    pub fn with_max_tokens(mut self, max_tokens: Option<u64>) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: Option<f32>) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_top_p(mut self, top_p: Option<f32>) -> Self {
        self.top_p = top_p;
        self
    }

    pub fn with_variant(mut self, variant: Option<String>) -> Self {
        self.variant = variant;
        self
    }

    pub fn with_provider_options(
        mut self,
        provider_options: Option<HashMap<String, serde_json::Value>>,
    ) -> Self {
        self.provider_options = provider_options;
        self
    }
}

pub fn session_runtime_request_defaults(variant: Option<String>) -> ExecutionRequestDefaults {
    ExecutionRequestDefaults::default()
        .with_max_tokens(Some(8192))
        .with_variant(variant)
}

pub fn inline_subtask_request_defaults(variant: Option<String>) -> ExecutionRequestDefaults {
    ExecutionRequestDefaults::default()
        .with_max_tokens(Some(2048))
        .with_temperature(Some(0.2))
        .with_variant(variant)
}

pub fn compaction_request(
    model_id: impl Into<String>,
    variant: Option<String>,
) -> CompiledExecutionRequest {
    CompiledExecutionRequest {
        model_id: model_id.into(),
        ..Default::default()
    }
    .inherit_missing(
        &ExecutionRequestDefaults::default()
            .with_max_tokens(Some(4096))
            .with_temperature(Some(0.0))
            .with_variant(variant),
    )
}

pub fn session_title_request(model_id: impl Into<String>) -> CompiledExecutionRequest {
    CompiledExecutionRequest {
        model_id: model_id.into(),
        ..Default::default()
    }
    .inherit_missing(
        &ExecutionRequestDefaults::default()
            .with_max_tokens(Some(100))
            .with_temperature(Some(0.0)),
    )
}

pub fn message_title_request(model_id: impl Into<String>) -> CompiledExecutionRequest {
    CompiledExecutionRequest {
        model_id: model_id.into(),
        ..Default::default()
    }
    .inherit_missing(
        &ExecutionRequestDefaults::default()
            .with_max_tokens(Some(64))
            .with_temperature(Some(0.0)),
    )
}

pub fn agent_generation_request(model_id: impl Into<String>) -> CompiledExecutionRequest {
    CompiledExecutionRequest {
        model_id: model_id.into(),
        ..Default::default()
    }
    .inherit_missing(&ExecutionRequestDefaults::default().with_temperature(Some(0.3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inherit_missing_only_fills_absent_fields() {
        let request = CompiledExecutionRequest {
            model_id: "model-a".to_string(),
            max_tokens: Some(32),
            temperature: None,
            top_p: Some(0.7),
            variant: None,
            provider_options: None,
        };

        let inherited = request.inherit_missing(
            &ExecutionRequestDefaults::default()
                .with_max_tokens(Some(64))
                .with_temperature(Some(0.2))
                .with_top_p(Some(0.9))
                .with_variant(Some("fast".to_string()))
                .with_provider_options(Some(HashMap::from([(
                    "thinking".to_string(),
                    json!(true),
                )]))),
        );

        assert_eq!(inherited.max_tokens, Some(32));
        assert_eq!(inherited.temperature, Some(0.2));
        assert_eq!(inherited.top_p, Some(0.7));
        assert_eq!(inherited.variant.as_deref(), Some("fast"));
        assert_eq!(
            inherited
                .provider_options
                .as_ref()
                .and_then(|options| options.get("thinking")),
            Some(&json!(true))
        );
    }

    #[test]
    fn compile_with_model_and_defaults_respects_context_overrides() {
        let context = ExecutionRequestContext {
            model_id: Some("ctx-model".to_string()),
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            variant: Some("deep".to_string()),
            provider_options: None,
            provider_id: Some("provider".to_string()),
        };

        let compiled = context.compile_with_model_and_defaults(
            "override-model",
            &inline_subtask_request_defaults(Some("fast".to_string())),
        );

        assert_eq!(compiled.model_id, "override-model");
        assert_eq!(compiled.max_tokens, Some(512));
        assert_eq!(compiled.temperature, Some(0.2));
        assert_eq!(compiled.variant.as_deref(), Some("deep"));
    }

    #[test]
    fn compiled_request_fields_propagate_to_chat_request() {
        let compiled = CompiledExecutionRequest {
            model_id: "regression-model".to_string(),
            max_tokens: Some(999),
            temperature: Some(0.42),
            top_p: Some(0.88),
            variant: Some("deep".to_string()),
            provider_options: Some(HashMap::from([(
                "thinking".to_string(),
                json!({"enabled": true}),
            )])),
        };

        let chat = compiled.to_chat_request(vec![], vec![], true);

        assert_eq!(chat.model, "regression-model");
        assert_eq!(chat.max_tokens, Some(999));
        assert_eq!(chat.temperature, Some(0.42));
        assert_eq!(chat.top_p, Some(0.88));
        assert_eq!(chat.variant.as_deref(), Some("deep"));
        assert_eq!(
            chat.provider_options
                .as_ref()
                .and_then(|options| options.get("thinking")),
            Some(&json!({"enabled": true}))
        );
    }
}
