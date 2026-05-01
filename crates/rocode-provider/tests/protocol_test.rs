use rocode_provider::{
    create_legacy_protocol_impl, create_protocol_impl, create_protocol_impl_for_profile,
    ChatRequest, Protocol, ProtocolImpl, ProviderConfig, ProviderProfileResolver,
};
use std::collections::HashMap;

#[test]
fn test_protocol_from_npm_messages_family() {
    let protocol = Protocol::from_npm("@ai-sdk/anthropic");
    assert_eq!(protocol, Protocol::Messages);
}

#[test]
fn test_protocol_from_npm_ethnopic_alias() {
    let protocol = Protocol::from_npm("ethnopic-compatible");
    assert_eq!(protocol, Protocol::Messages);
}

#[test]
fn test_protocol_from_npm_openai() {
    let protocol = Protocol::from_npm("@ai-sdk/openai");
    assert_eq!(protocol, Protocol::OpenAI);
}

#[test]
fn test_protocol_from_npm_closeai_and_openai_aliases() {
    assert_eq!(Protocol::from_npm("closeai-compatible"), Protocol::OpenAI);
    assert_eq!(Protocol::from_npm("openai-compatible"), Protocol::OpenAI);
}

#[test]
fn test_protocol_from_npm_openrouter_and_perplexity() {
    assert_eq!(
        Protocol::from_npm("@openrouter/ai-sdk-provider"),
        Protocol::OpenAI
    );
    assert_eq!(Protocol::from_npm("@ai-sdk/perplexity"), Protocol::OpenAI);
}

#[test]
fn test_protocol_from_npm_unknown_defaults_to_openai() {
    let protocol = Protocol::from_npm("@custom/unknown-provider");
    assert_eq!(protocol, Protocol::OpenAI);
}

#[test]
fn test_protocol_from_npm_vertex() {
    let protocol = Protocol::from_npm("@ai-sdk/google-vertex");
    assert_eq!(protocol, Protocol::Vertex);
}

#[test]
fn test_protocol_from_npm_google() {
    let protocol = Protocol::from_npm("@ai-sdk/google");
    assert_eq!(protocol, Protocol::Google);
}

#[test]
fn test_protocol_from_npm_bedrock() {
    let protocol = Protocol::from_npm("@ai-sdk/bedrock");
    assert_eq!(protocol, Protocol::Bedrock);
}

#[test]
fn test_protocol_from_npm_github_copilot() {
    let protocol = Protocol::from_npm("@ai-sdk/github-copilot");
    assert_eq!(protocol, Protocol::GitHubCopilot);
}

#[test]
fn test_protocol_from_npm_gitlab() {
    let protocol = Protocol::from_npm("@ai-sdk/gitlab");
    assert_eq!(protocol, Protocol::GitLab);
}

#[test]
fn test_protocol_case_insensitive() {
    assert_eq!(Protocol::from_npm("@AI-SDK/ANTHROPIC"), Protocol::Messages);
    assert_eq!(Protocol::from_npm("@Ai-Sdk/Openai"), Protocol::OpenAI);
}

#[test]
fn test_protocol_alias_labels() {
    assert_eq!(Protocol::Messages.to_string(), "Anthropic");
    assert_eq!(Protocol::OpenAI.to_string(), "OpenAI");
}

#[test]
fn test_protocol_from_profile_projects_vendor_protocols_as_legacy_adapter() {
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

    assert_eq!(Protocol::from_profile(&gitlab), Protocol::GitLab);
    assert_eq!(Protocol::from_profile(&copilot), Protocol::GitHubCopilot);
    assert_eq!(Protocol::from_profile(&vertex), Protocol::Vertex);
    assert_eq!(Protocol::from_profile(&closeai), Protocol::OpenAI);
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

struct MockProtocol;

#[async_trait::async_trait]
impl ProtocolImpl for MockProtocol {
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
fn test_protocol_impl_trait_bounds() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockProtocol>();
}

#[test]
fn test_create_protocol_impl_openai() {
    let protocol = create_protocol_impl(Protocol::OpenAI);
    let _arc: std::sync::Arc<dyn ProtocolImpl> = protocol;
}

#[test]
fn test_create_legacy_protocol_impl_openai() {
    let protocol = create_legacy_protocol_impl(Protocol::OpenAI);
    let _arc: std::sync::Arc<dyn ProtocolImpl> = protocol;
}

#[test]
fn test_create_protocol_impl_ethnopic_family() {
    let protocol = create_protocol_impl(Protocol::Messages);
    let _arc: std::sync::Arc<dyn ProtocolImpl> = protocol;
}

#[test]
fn test_create_protocol_impl_for_profile() {
    let options = HashMap::new();
    let profile =
        ProviderProfileResolver::resolve_with_npm("ethnopic", "@ai-sdk/anthropic", &options);
    let protocol = create_protocol_impl_for_profile(&profile);
    let _arc: std::sync::Arc<dyn ProtocolImpl> = protocol;
}
