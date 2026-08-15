use agendao_provider::{
    create_provider_adapter, create_provider_adapter_for_profile, ChatRequest, ProviderAdapter,
    ProviderConfig, ProviderProfileResolver, ProviderRuntimeAdapter,
};
use std::collections::HashMap;

#[test]
fn test_adapter_from_profile_anthropic_family() {
    let adapter = adapter_from_resolved_profile("anthropic", "@ai-sdk/anthropic");
    assert_eq!(adapter, ProviderRuntimeAdapter::Anthropic);
}

#[test]
fn test_adapter_from_profile_openai() {
    let adapter = adapter_from_resolved_profile("openai", "@ai-sdk/openai");
    assert_eq!(adapter, ProviderRuntimeAdapter::OpenAiCompatible);
}

#[test]
fn test_adapter_from_explicit_custom_chat_profile() {
    let options = custom_chat_options();
    let profile =
        ProviderProfileResolver::resolve_with_npm("custom", "@ai-sdk/openai-compatible", &options);
    assert_eq!(
        ProviderRuntimeAdapter::from_profile(&profile),
        ProviderRuntimeAdapter::OpenAiCompatible
    );
}

#[test]
fn test_unknown_and_removed_sdk_shapes_are_rejected() {
    let options = HashMap::new();
    for (provider_id, npm) in [
        ("google", "@ai-sdk/google"),
        ("google-vertex", "@ai-sdk/google-vertex"),
        ("amazon-bedrock", "@ai-sdk/amazon-bedrock"),
        ("github-copilot", "@ai-sdk/github-copilot"),
        ("gitlab", "@gitlab/gitlab-ai-provider"),
    ] {
        assert!(ProviderProfileResolver::try_resolve_with_npm(provider_id, npm, &options).is_err());
    }

    assert!(ProviderProfileResolver::try_resolve_with_npm(
        "custom",
        "@custom/unknown-provider",
        &custom_chat_options(),
    )
    .is_err());
}

#[test]
fn test_adapter_resolution_is_case_insensitive() {
    assert_eq!(
        adapter_from_resolved_profile("anthropic", "@AI-SDK/ANTHROPIC"),
        ProviderRuntimeAdapter::Anthropic
    );
    assert_eq!(
        adapter_from_resolved_profile("openai", "@Ai-Sdk/Openai"),
        ProviderRuntimeAdapter::OpenAiCompatible
    );
}

fn adapter_from_resolved_profile(provider_id: &str, npm: &str) -> ProviderRuntimeAdapter {
    let options = HashMap::new();
    let profile = ProviderProfileResolver::resolve_with_npm(provider_id, npm, &options);
    ProviderRuntimeAdapter::from_profile(&profile)
}

fn custom_chat_options() -> HashMap<String, serde_json::Value> {
    HashMap::from([(
        "provider_profile".to_string(),
        serde_json::json!({
            "api_style": "openai-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "openai-cached-tokens"
        }),
    )])
}

#[test]
fn test_adapter_display_labels() {
    assert_eq!(ProviderRuntimeAdapter::Anthropic.to_string(), "anthropic");
    assert_eq!(
        ProviderRuntimeAdapter::OpenAiCompatible.to_string(),
        "openai-compatible"
    );
}

#[test]
fn test_provider_config_basic() {
    let config = ProviderConfig {
        provider_id: "deepseek".to_string(),
        base_url: "https://api.deepseek.com/chat/completions".to_string(),
        api_key: "sk-test".to_string(),
        headers: HashMap::new(),
        options: HashMap::new(),
    };

    assert_eq!(config.provider_id, "deepseek");
    assert_eq!(config.base_url, "https://api.deepseek.com/chat/completions");
}

#[test]
fn test_provider_config_with_custom_headers() {
    let mut headers = HashMap::new();
    headers.insert(
        "HTTP-Referer".to_string(),
        "https://opencode.ai/".to_string(),
    );
    headers.insert("X-Title".to_string(), "opencode".to_string());

    let config = ProviderConfig {
        provider_id: "openrouter".to_string(),
        base_url: "https://openrouter.ai/api/v1/chat/completions".to_string(),
        api_key: "sk-or-...".to_string(),
        headers,
        options: HashMap::new(),
    };

    assert_eq!(
        config.headers.get("HTTP-Referer").expect("header"),
        "https://opencode.ai/"
    );
}

#[test]
fn test_provider_config_with_options() {
    let mut options = HashMap::new();
    options.insert("endpoint_path".to_string(), serde_json::json!("/v2/chat"));

    let config = ProviderConfig {
        provider_id: "cohere".to_string(),
        base_url: "https://api.cohere.ai".to_string(),
        api_key: "sk-cohere".to_string(),
        headers: HashMap::new(),
        options,
    };

    assert_eq!(
        config.options.get("endpoint_path").expect("option"),
        "/v2/chat"
    );
}

struct MockProviderAdapter;

#[async_trait::async_trait]
impl ProviderAdapter for MockProviderAdapter {
    async fn chat(
        &self,
        _client: &reqwest::Client,
        _config: &ProviderConfig,
        _request: ChatRequest,
    ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
        unimplemented!()
    }

    async fn chat_stream(
        &self,
        _client: &reqwest::Client,
        _config: &ProviderConfig,
        _request: ChatRequest,
    ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
        unimplemented!()
    }
}

#[test]
fn test_provider_adapter_trait_bounds() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockProviderAdapter>();
}

#[test]
fn test_create_provider_adapter_openai_compatible() {
    let adapter = create_provider_adapter(ProviderRuntimeAdapter::OpenAiCompatible);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}

#[test]
fn test_create_provider_adapter_anthropic() {
    let adapter = create_provider_adapter(ProviderRuntimeAdapter::Anthropic);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}

#[test]
fn test_create_provider_adapter_for_profile() {
    let options = HashMap::new();
    let profile =
        ProviderProfileResolver::resolve_with_npm("anthropic", "@ai-sdk/anthropic", &options);
    let adapter = create_provider_adapter_for_profile(&profile);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}
