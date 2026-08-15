use agendao_provider::{ProviderConfig, ProviderProfileResolver, ProviderRuntimeAdapter};
use std::collections::HashMap;

#[test]
fn test_explicit_custom_chat_profile_uses_openai_compatible_adapter() {
    let adapter = custom_chat_adapter("custom-chat");
    assert_eq!(adapter, ProviderRuntimeAdapter::OpenAiCompatible);
}

#[test]
fn test_custom_messages_endpoint() {
    let adapter = adapter_from_resolved_profile("anthropic", "@ai-sdk/anthropic");
    assert_eq!(adapter, ProviderRuntimeAdapter::Anthropic);

    let config = ProviderConfig::new(
        "bailian",
        "https://coding.dashscope.aliyuncs.com/api/v1/messages",
        "sk-sp-xxx",
    );

    assert_eq!(
        config.base_url,
        "https://coding.dashscope.aliyuncs.com/api/v1/messages"
    );
}

fn adapter_from_resolved_profile(provider_id: &str, npm: &str) -> ProviderRuntimeAdapter {
    let options = HashMap::new();
    let profile = ProviderProfileResolver::resolve_with_npm(provider_id, npm, &options);
    ProviderRuntimeAdapter::from_profile(&profile)
}

fn custom_chat_adapter(provider_id: &str) -> ProviderRuntimeAdapter {
    let options = HashMap::from([(
        "provider_profile".to_string(),
        serde_json::json!({
            "api_style": "openai-compatible",
            "api_shape": "chat-completions",
            "transport": "bearer",
            "usage_shape": "openai-cached-tokens"
        }),
    )]);
    let profile = ProviderProfileResolver::resolve_with_npm(
        provider_id,
        "@ai-sdk/openai-compatible",
        &options,
    );
    ProviderRuntimeAdapter::from_profile(&profile)
}
