use rocode_provider::{
    create_provider_adapter, create_provider_adapter_for_profile, ChatRequest, ProviderAdapter,
    ProviderConfig, ProviderProfileResolver, ProviderRuntimeAdapter,
};
use std::collections::HashMap;

#[test]
fn test_adapter_from_profile_ethnopic_family() {
    let adapter = adapter_from_resolved_profile("ethnopic", "@ai-sdk/anthropic");
    assert_eq!(adapter, ProviderRuntimeAdapter::Ethnopic);
}

#[test]
fn test_adapter_from_profile_ethnopic_alias() {
    let adapter = adapter_from_resolved_profile("ethnopic", "ethnopic-compatible");
    assert_eq!(adapter, ProviderRuntimeAdapter::Ethnopic);
}

#[test]
fn test_adapter_from_profile_openai() {
    let adapter = adapter_from_resolved_profile("openai", "@ai-sdk/openai");
    assert_eq!(adapter, ProviderRuntimeAdapter::CloseAiCompatible);
}

#[test]
fn test_adapter_from_profile_closeai_and_openai_aliases() {
    assert_eq!(
        adapter_from_resolved_profile("custom-closeai", "closeai-compatible"),
        ProviderRuntimeAdapter::CloseAiCompatible
    );
    assert_eq!(
        adapter_from_resolved_profile("custom-closeai", "openai-compatible"),
        ProviderRuntimeAdapter::CloseAiCompatible
    );
}

#[test]
fn test_adapter_from_profile_openrouter_and_perplexity() {
    assert_eq!(
        adapter_from_resolved_profile("openrouter", "@openrouter/ai-sdk-provider"),
        ProviderRuntimeAdapter::CloseAiCompatible
    );
    assert_eq!(
        adapter_from_resolved_profile("perplexity", "@ai-sdk/perplexity"),
        ProviderRuntimeAdapter::CloseAiCompatible
    );
}

#[test]
fn test_adapter_from_profile_unknown_defaults_to_closeai_compatible() {
    let adapter = adapter_from_resolved_profile("custom", "@custom/unknown-provider");
    assert_eq!(adapter, ProviderRuntimeAdapter::CloseAiCompatible);
}

#[test]
fn test_adapter_from_profile_vertex() {
    let adapter = adapter_from_resolved_profile("google-vertex", "@ai-sdk/google-vertex");
    assert_eq!(adapter, ProviderRuntimeAdapter::VertexGemini);
}

#[test]
fn test_adapter_from_profile_google() {
    let adapter = adapter_from_resolved_profile("google", "@ai-sdk/google");
    assert_eq!(adapter, ProviderRuntimeAdapter::Gemini);
}

#[test]
fn test_adapter_from_profile_bedrock() {
    let adapter = adapter_from_resolved_profile("amazon-bedrock", "@ai-sdk/bedrock");
    assert_eq!(adapter, ProviderRuntimeAdapter::BedrockConverse);
}

#[test]
fn test_adapter_from_profile_github_copilot() {
    let adapter = adapter_from_resolved_profile("github-copilot", "@ai-sdk/github-copilot");
    assert_eq!(adapter, ProviderRuntimeAdapter::GitHubCopilotCloseAi);
}

#[test]
fn test_adapter_from_profile_gitlab() {
    let adapter = adapter_from_resolved_profile("gitlab", "@ai-sdk/gitlab");
    assert_eq!(adapter, ProviderRuntimeAdapter::GitLabCloseAi);
}

#[test]
fn test_adapter_resolution_is_case_insensitive() {
    assert_eq!(
        adapter_from_resolved_profile("ethnopic", "@AI-SDK/ANTHROPIC"),
        ProviderRuntimeAdapter::Ethnopic
    );
    assert_eq!(
        adapter_from_resolved_profile("openai", "@Ai-Sdk/Openai"),
        ProviderRuntimeAdapter::CloseAiCompatible
    );
}

fn adapter_from_resolved_profile(provider_id: &str, npm: &str) -> ProviderRuntimeAdapter {
    let options = HashMap::new();
    let profile = ProviderProfileResolver::resolve_with_npm(provider_id, npm, &options);
    ProviderRuntimeAdapter::from_profile(&profile)
}

#[test]
fn test_adapter_display_labels() {
    assert_eq!(ProviderRuntimeAdapter::Ethnopic.to_string(), "ethnopic");
    assert_eq!(
        ProviderRuntimeAdapter::CloseAiCompatible.to_string(),
        "closeai-compatible"
    );
}

#[test]
fn test_adapter_from_profile_projects_vendor_specific_adapters() {
    let options = HashMap::new();
    let gitlab =
        ProviderProfileResolver::resolve_with_npm("gitlab", "@gitlab/gitlab-ai-provider", &options);
    let copilot = ProviderProfileResolver::resolve_with_npm(
        "github-copilot",
        "@ai-sdk/github-copilot",
        &options,
    );
    let vertex = ProviderProfileResolver::resolve_with_npm(
        "google-vertex",
        "@ai-sdk/google-vertex",
        &options,
    );
    let closeai = ProviderProfileResolver::resolve_with_npm(
        "custom-closeai",
        "@ai-sdk/openai-compatible",
        &options,
    );

    assert_eq!(
        ProviderRuntimeAdapter::from_profile(&gitlab),
        ProviderRuntimeAdapter::GitLabCloseAi
    );
    assert_eq!(
        ProviderRuntimeAdapter::from_profile(&copilot),
        ProviderRuntimeAdapter::GitHubCopilotCloseAi
    );
    assert_eq!(
        ProviderRuntimeAdapter::from_profile(&vertex),
        ProviderRuntimeAdapter::VertexGemini
    );
    assert_eq!(
        ProviderRuntimeAdapter::from_profile(&closeai),
        ProviderRuntimeAdapter::CloseAiCompatible
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
    ) -> Result<rocode_provider::ChatResponse, rocode_provider::ProviderError> {
        unimplemented!()
    }

    async fn chat_stream(
        &self,
        _client: &reqwest::Client,
        _config: &ProviderConfig,
        _request: ChatRequest,
    ) -> Result<rocode_provider::StreamResult, rocode_provider::ProviderError> {
        unimplemented!()
    }
}

#[test]
fn test_provider_adapter_trait_bounds() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockProviderAdapter>();
}

#[test]
fn test_create_provider_adapter_closeai_compatible() {
    let adapter = create_provider_adapter(ProviderRuntimeAdapter::CloseAiCompatible);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}

#[test]
fn test_create_provider_adapter_ethnopic() {
    let adapter = create_provider_adapter(ProviderRuntimeAdapter::Ethnopic);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}

#[test]
fn test_create_provider_adapter_for_profile() {
    let options = HashMap::new();
    let profile =
        ProviderProfileResolver::resolve_with_npm("ethnopic", "@ai-sdk/anthropic", &options);
    let adapter = create_provider_adapter_for_profile(&profile);
    let _arc: std::sync::Arc<dyn ProviderAdapter> = adapter;
}
