use super::*;
use crate::models::{ModelInfo, ModelInterleaved, ModelLimit, ModelModalities, ModelProvider};
use std::collections::HashMap;

fn provider_model(model_id: &str) -> ProviderModel {
    ProviderModel {
        id: model_id.to_string(),
        provider_id: "test".to_string(),
        name: model_id.to_string(),
        api: ProviderModelApi {
            id: model_id.to_string(),
            url: "https://example.com".to_string(),
            npm: "@ai-sdk/openai".to_string(),
        },
        family: None,
        capabilities: ModelCapabilities {
            temperature: true,
            reasoning: true,
            attachment: false,
            toolcall: true,
            input: ModalitySet {
                text: true,
                audio: false,
                image: false,
                video: false,
                pdf: false,
            },
            output: ModalitySet {
                text: true,
                audio: false,
                image: false,
                video: false,
                pdf: false,
            },
            interleaved: InterleavedConfig::Bool(false),
        },
        cost: ProviderModelCost {
            input: 0.0,
            output: 0.0,
            cache: ModelCostCache {
                read: 0.0,
                write: 0.0,
            },
            experimental_over_200k: None,
        },
        limit: ProviderModelLimit {
            context: 128_000,
            input: None,
            output: 8_192,
        },
        status: "active".to_string(),
        options: HashMap::new(),
        headers: HashMap::new(),
        release_date: "2026-01-01".to_string(),
        variants: None,
    }
}

fn model_info(model_id: &str) -> ModelInfo {
    ModelInfo {
        id: model_id.to_string(),
        name: model_id.to_string(),
        family: None,
        release_date: Some("2026-01-01".to_string()),
        attachment: false,
        reasoning: true,
        temperature: true,
        tool_call: true,
        interleaved: Some(ModelInterleaved::Bool(false)),
        cost: None,
        limit: ModelLimit {
            context: 128_000,
            input: None,
            output: 8_192,
        },
        modalities: Some(ModelModalities {
            input: vec!["text".to_string()],
            output: vec!["text".to_string()],
        }),
        experimental: None,
        status: Some("active".to_string()),
        options: HashMap::new(),
        headers: None,
        provider: Some(ModelProvider {
            npm: Some("@ai-sdk/openai".to_string()),
            api: Some("https://api.openai.com/v1".to_string()),
        }),
        variants: None,
    }
}

fn provider_info(provider_id: &str, model: ModelInfo) -> ModelsProviderInfo {
    let mut models = HashMap::new();
    models.insert(model.id.clone(), model);
    ModelsProviderInfo {
        api: Some("https://example.com".to_string()),
        name: provider_id.to_string(),
        env: vec![],
        id: provider_id.to_string(),
        npm: Some("@ai-sdk/openai".to_string()),
        models,
    }
}

fn provider_state(id: &str) -> ProviderState {
    ProviderState {
        id: id.to_string(),
        name: id.to_string(),
        source: "env".to_string(),
        env: vec![],
        key: None,
        options: HashMap::new(),
        models: HashMap::new(),
    }
}

#[test]
fn creates_openai_provider_from_state_key() {
    let mut state = provider_state("openai");
    state.key = Some("test-key".to_string());

    let provider = create_concrete_provider("openai", &state).expect("provider should exist");
    assert_eq!(provider.id(), "openai");
}

#[test]
fn creates_bedrock_provider_from_options() {
    let mut state = provider_state("amazon-bedrock");
    state.options.insert(
        "accessKeyId".to_string(),
        serde_json::Value::String("akid".to_string()),
    );
    state.options.insert(
        "secretAccessKey".to_string(),
        serde_json::Value::String("secret".to_string()),
    );
    state.options.insert(
        "region".to_string(),
        serde_json::Value::String("us-east-1".to_string()),
    );

    let provider =
        create_concrete_provider("amazon-bedrock", &state).expect("provider should exist");
    assert_eq!(provider.id(), "amazon-bedrock");
}

#[test]
fn creates_custom_provider_from_declared_closeai_profile() {
    let mut state = provider_state("my-custom");
    state.key = Some("test-key".to_string());
    state.options.insert(
        "providerProfile".to_string(),
        serde_json::json!({
            "api_style": "closeai-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "closeai-cached-tokens"
        }),
    );

    let provider = create_concrete_provider("my-custom", &state).expect("provider should exist");
    assert_eq!(provider.id(), "my-custom");
}

#[test]
fn rejects_custom_provider_with_invalid_profile() {
    let mut state = provider_state("my-custom");
    state.key = Some("test-key".to_string());
    state.options.insert(
        "providerProfile".to_string(),
        serde_json::json!({
            "api_style": "ethnopic-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "ethnopic-read-write"
        }),
    );

    assert!(create_concrete_provider("my-custom", &state).is_none());
}

#[test]
fn sort_models_prioritizes_big_pickle_over_non_priority_models() {
    let mut models = vec![
        provider_model("my-custom-model"),
        provider_model("big-pickle-v2"),
    ];
    ProviderBootstrapState::sort_models(&mut models);
    assert_eq!(models[0].id, "big-pickle-v2");
}

#[test]
fn apply_custom_loaders_applies_zenmux_headers() {
    let model = model_info("zenmux-model");
    let mut data = HashMap::new();
    data.insert("zenmux".to_string(), provider_info("zenmux", model));

    apply_custom_loaders(&mut data);

    let provider = data.get("zenmux").expect("zenmux provider should exist");
    let model = provider
        .models
        .get("zenmux-model")
        .expect("zenmux model should exist");
    let headers = model.headers.as_ref().expect("headers should be set");
    assert_eq!(
        headers.get("HTTP-Referer").map(String::as_str),
        Some("https://opencode.ai/")
    );
    assert_eq!(headers.get("X-Title").map(String::as_str), Some("opencode"));
}

#[test]
fn bedrock_loader_reads_provider_state_options() {
    let loader = AmazonBedrockLoader;
    let mut state = provider_state("amazon-bedrock");
    state.options.insert(
        "region".to_string(),
        serde_json::Value::String("us-west-2".to_string()),
    );
    state.options.insert(
        "profile".to_string(),
        serde_json::Value::String("dev-profile".to_string()),
    );
    state.options.insert(
        "endpoint".to_string(),
        serde_json::Value::String("https://bedrock.internal".to_string()),
    );

    let result = loader.load(
        &provider_info("amazon-bedrock", model_info("test-vendor.test-model-large")),
        Some(&state),
    );
    assert!(result.autoload);
    assert_eq!(
        result.options.get("region"),
        Some(&serde_json::Value::String("us-west-2".to_string()))
    );
    assert_eq!(
        result.options.get("profile"),
        Some(&serde_json::Value::String("dev-profile".to_string()))
    );
    assert_eq!(
        result.options.get("endpoint"),
        Some(&serde_json::Value::String(
            "https://bedrock.internal".to_string()
        ))
    );
    assert!(result.has_custom_get_model);
}

#[test]
fn from_models_dev_model_merges_transform_and_explicit_variants() {
    let mut model = model_info("gpt-5");
    let mut explicit = HashMap::new();
    explicit.insert(
        "custom".to_string(),
        HashMap::from([(
            "reasoningEffort".to_string(),
            serde_json::Value::String("custom".to_string()),
        )]),
    );
    model.variants = Some(explicit);

    let provider = provider_info("openai", model.clone());
    let runtime_model = from_models_dev_model(&provider, &model);
    let variants = runtime_model
        .variants
        .expect("variants should include generated and explicit values");
    assert!(variants.contains_key("custom"));
    assert!(variants.contains_key("low"));
}
