use rocode_provider::{ProviderConfig, ProviderProfileResolver, ProviderRuntimeAdapter};
use std::collections::HashMap;

#[test]
fn test_deepseek_uses_closeai_compatible_adapter() {
    let adapter = adapter_from_resolved_profile("deepseek", "@ai-sdk/openai-compatible");
    assert_eq!(adapter, ProviderRuntimeAdapter::CloseAiCompatible);
}

#[test]
fn test_custom_messages_endpoint() {
    let adapter = adapter_from_resolved_profile("ethnopic", "@ai-sdk/anthropic");
    assert_eq!(adapter, ProviderRuntimeAdapter::Ethnopic);

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

#[test]
fn test_custom_ethnopic_endpoint_alias() {
    let adapter = adapter_from_resolved_profile("ethnopic", "ethnopic-compatible");
    assert_eq!(adapter, ProviderRuntimeAdapter::Ethnopic);

    let config = ProviderConfig::new(
        "compatible-messages",
        "https://example.com/provider/messages",
        "sk-test",
    );

    assert_eq!(config.base_url, "https://example.com/provider/messages");
}

#[test]
fn test_openrouter_custom_headers() {
    let adapter = adapter_from_resolved_profile("openrouter", "@openrouter/ai-sdk-provider");
    assert_eq!(adapter, ProviderRuntimeAdapter::CloseAiCompatible);

    let config = ProviderConfig::new(
        "openrouter",
        "https://openrouter.ai/api/v1/chat/completions",
        "sk-or-xxx",
    )
    .with_header("HTTP-Referer", "https://opencode.ai/")
    .with_header("X-Title", "opencode");

    assert_eq!(
        config.headers.get("HTTP-Referer").expect("referer header"),
        "https://opencode.ai/"
    );
}

fn adapter_from_resolved_profile(provider_id: &str, npm: &str) -> ProviderRuntimeAdapter {
    let options = HashMap::new();
    let profile = ProviderProfileResolver::resolve_with_npm(provider_id, npm, &options);
    ProviderRuntimeAdapter::from_profile(&profile)
}
