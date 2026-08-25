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
fn creates_custom_provider_from_declared_openai_profile() {
    let mut state = provider_state("my-custom");
    state.key = Some("test-key".to_string());
    state.options.insert(
        "provider_profile".to_string(),
        serde_json::json!({
            "api_style": "openai-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "openai-cached-tokens"
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
        "provider_profile".to_string(),
        serde_json::json!({
            "api_style": "anthropic-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "anthropic-read-write"
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

#[test]
fn authenticated_catalog_provider_registers_all_catalog_models() {
    let mut deepseek = provider_info("deepseek", model_info("deepseek-v4-pro"));
    deepseek.models.insert(
        "deepseek-v4-flash".to_string(),
        model_info("deepseek-v4-flash"),
    );
    let models_dev = HashMap::from([("deepseek".to_string(), deepseek)]);
    let auth_store = HashMap::from([(
        "deepseek".to_string(),
        AuthInfo::Api {
            key: "test-key".to_string(),
        },
    )]);

    let state = ProviderBootstrapState::init(&models_dev, &BootstrapConfig::default(), &auth_store);
    let provider = state
        .get_provider("deepseek")
        .expect("authenticated catalogue provider should be registered");

    assert_eq!(provider.key.as_deref(), Some("test-key"));
    assert!(provider.models.contains_key("deepseek-v4-pro"));
    assert!(provider.models.contains_key("deepseek-v4-flash"));
}

#[test]
fn auth_store_key_beats_config_file_api_key() {
    let models_dev = HashMap::from([(
        "deepseek".to_string(),
        provider_info("deepseek", model_info("deepseek-v4-pro")),
    )]);
    let config = BootstrapConfig {
        providers: HashMap::from([(
            "deepseek".to_string(),
            ConfigProvider {
                api_key: Some("stale-config-key".to_string()),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let auth_store = HashMap::from([(
        "deepseek".to_string(),
        AuthInfo::Api {
            key: "fresh-ui-key".to_string(),
        },
    )]);

    let state = ProviderBootstrapState::init(&models_dev, &config, &auth_store);
    let provider = state
        .get_provider("deepseek")
        .expect("provider should be registered");
    // The key the user just saved through the UI must win over a stale
    // api_key committed in a config file.
    assert_eq!(provider.key.as_deref(), Some("fresh-ui-key"));
    assert_eq!(provider.source, "api");
}

#[test]
fn env_var_key_beats_auth_store_key() {
    let mut provider = provider_info("deepseek", model_info("deepseek-v4-pro"));
    provider.env = vec!["DEEPSEEK_API_KEY".to_string()];
    let models_dev = HashMap::from([("deepseek".to_string(), provider)]);
    let auth_store = HashMap::from([(
        "deepseek".to_string(),
        AuthInfo::Api {
            key: "auth-store-key".to_string(),
        },
    )]);

    // Scoped env guard: restore the variable even if the test panics.
    struct EnvRestore(&'static str, Option<std::ffi::OsString>);
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.1.clone() {
                Some(previous) => std::env::set_var(self.0, previous),
                None => std::env::remove_var(self.0),
            }
        }
    }
    let _restore = EnvRestore("DEEPSEEK_API_KEY", std::env::var_os("DEEPSEEK_API_KEY"));
    std::env::set_var("DEEPSEEK_API_KEY", "env-key");

    let state = ProviderBootstrapState::init(&models_dev, &BootstrapConfig::default(), &auth_store);
    let registered = state
        .get_provider("deepseek")
        .expect("provider should be registered");
    assert_eq!(registered.key.as_deref(), Some("env-key"));
    assert_eq!(registered.source, "env");
}

#[test]
fn legacy_options_config_provider_materializes_all_models() {
    // Regression (9d2fd32): opencode-era options.baseURL/apiKey providers
    // failed profile validation and were skipped entirely. With the
    // npm-derived default profile fallback they must materialize with the
    // full catalogue plus config-declared models.
    let mut zhipu = provider_info("zhipuai-coding-plan", model_info("glm-4.5-air"));
    for id in [
        "glm-5.1",
        "glm-5",
        "glm-5v-turbo",
        "glm-5-turbo",
        "glm-4.6v",
    ] {
        zhipu.models.insert(id.to_string(), model_info(id));
    }
    zhipu.npm = Some("@ai-sdk/openai-compatible".to_string());
    let models_dev = HashMap::from([("zhipuai-coding-plan".to_string(), zhipu)]);

    let mut options = HashMap::new();
    options.insert(
        "baseURL".to_string(),
        serde_json::json!("https://open.bigmodel.cn/api/coding/paas/v4"),
    );
    options.insert("apiKey".to_string(), serde_json::json!("legacy-key"));
    let config = BootstrapConfig {
        providers: HashMap::from([(
            "zhipuai-coding-plan".to_string(),
            ConfigProvider {
                options: Some(options),
                models: Some(HashMap::from([(
                    "GLM-5.1".to_string(),
                    ConfigModel {
                        name: Some("GLM-5.1".to_string()),
                        ..Default::default()
                    },
                )])),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };

    let state = ProviderBootstrapState::init(&models_dev, &config, &HashMap::new());
    let provider = state
        .get_provider("zhipuai-coding-plan")
        .expect("legacy config provider should materialize");
    assert_eq!(provider.models.len(), 7, "6 catalogue + 1 config model");

    let concrete = create_concrete_provider("zhipuai-coding-plan", provider)
        .expect("concrete provider should exist");
    let runtime_models = concrete.models();
    let runtime_ids: Vec<&str> = runtime_models.iter().map(|m| m.id.as_str()).collect();
    assert!(
        runtime_ids.contains(&"GLM-5.1"),
        "config model must survive: {runtime_ids:?}"
    );
    assert!(
        runtime_ids.contains(&"glm-5.1"),
        "catalogue model must survive: {runtime_ids:?}"
    );
}
