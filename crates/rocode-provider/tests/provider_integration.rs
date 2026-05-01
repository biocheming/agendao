use rocode_provider::{
    ChatRequest, Message, ModelInfo, ProviderConfig, ProviderInstance, ProviderRegistry,
    ProviderRuntimeAdapter,
};
use std::collections::HashMap;

fn create_test_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();

    // Ethnopic-family provider via runtime adapter.
    let ethnopic_models: HashMap<String, ModelInfo> = vec![
        ModelInfo {
            id: "test-model-large".to_string(),
            name: "Test Model Large".to_string(),
            provider: "ethnopic".to_string(),
            context_window: 200000,
            max_input_tokens: None,
            max_output_tokens: 16000,
            supports_vision: true,
            supports_tools: true,
            cost_per_million_input: 3.0,
            cost_per_million_output: 15.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        },
        ModelInfo {
            id: "test-model-v2".to_string(),
            name: "Test Model V2".to_string(),
            provider: "ethnopic".to_string(),
            context_window: 200000,
            max_input_tokens: None,
            max_output_tokens: 8192,
            supports_vision: true,
            supports_tools: true,
            cost_per_million_input: 3.0,
            cost_per_million_output: 15.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        },
    ]
    .into_iter()
    .map(|m| (m.id.clone(), m))
    .collect();

    registry.register(ProviderInstance::new(
        "ethnopic".to_string(),
        "Ethnopic".to_string(),
        ProviderConfig::new("ethnopic", "", "test-key"),
        rocode_provider::create_provider_adapter(ProviderRuntimeAdapter::Ethnopic),
        ethnopic_models,
    ));

    // OpenAI provider using the closeai-compatible runtime adapter.
    let openai_models: HashMap<String, ModelInfo> = vec![ModelInfo {
        id: "gpt-4o".to_string(),
        name: "GPT-4o".to_string(),
        provider: "openai".to_string(),
        context_window: 128000,
        max_input_tokens: None,
        max_output_tokens: 16384,
        supports_vision: true,
        supports_tools: true,
        cost_per_million_input: 2.5,
        cost_per_million_output: 10.0,
        cost_per_million_cache_read: None,
        cost_per_million_cache_write: None,
    }]
    .into_iter()
    .map(|m| (m.id.clone(), m))
    .collect();

    registry.register(ProviderInstance::new(
        "openai".to_string(),
        "OpenAI".to_string(),
        ProviderConfig::new("openai", "", "test-key"),
        rocode_provider::create_provider_adapter(ProviderRuntimeAdapter::CloseAiCompatible),
        openai_models,
    ));

    // Google provider using the Gemini runtime adapter.
    let google_models: HashMap<String, ModelInfo> = vec![ModelInfo {
        id: "gemini-2.0-flash".to_string(),
        name: "Gemini 2.0 Flash".to_string(),
        provider: "google".to_string(),
        context_window: 1_000_000,
        max_input_tokens: None,
        max_output_tokens: 8192,
        supports_vision: true,
        supports_tools: true,
        cost_per_million_input: 0.1,
        cost_per_million_output: 0.4,
        cost_per_million_cache_read: None,
        cost_per_million_cache_write: None,
    }]
    .into_iter()
    .map(|m| (m.id.clone(), m))
    .collect();

    registry.register(ProviderInstance::new(
        "google".to_string(),
        "Google AI".to_string(),
        ProviderConfig::new("google", "", "test-key"),
        rocode_provider::create_provider_adapter(ProviderRuntimeAdapter::Gemini),
        google_models,
    ));

    registry
}

#[test]
fn test_registry_lists_providers() {
    let registry = create_test_registry();
    let providers = registry.list_providers();

    assert!(!providers.is_empty(), "Registry should have providers");

    let provider_ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    assert!(
        provider_ids.contains(&"ethnopic"),
        "Should have ethnopic provider"
    );
    assert!(
        provider_ids.contains(&"openai"),
        "Should have openai provider"
    );
    assert!(
        provider_ids.contains(&"google"),
        "Should have google provider"
    );
}

#[test]
fn test_registry_lists_models() {
    let registry = create_test_registry();
    let models = registry.list_models();

    assert!(!models.is_empty(), "Registry should have models");

    let model_ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(
        model_ids.iter().any(|id| id.contains("test-model")),
        "Should have test models"
    );
    assert!(
        model_ids.iter().any(|id| id.contains("gpt")),
        "Should have GPT models"
    );
}

#[test]
fn test_find_model_by_id() {
    let registry = create_test_registry();

    let result = registry.find_model("test-model-v2");
    assert!(result.is_some(), "Should find test-model-v2 model");

    let (provider_id, model) = result.unwrap();
    assert_eq!(provider_id, "ethnopic");
    assert!(model.supports_vision);
    assert!(model.supports_tools);
}

#[test]
fn test_provider_metadata() {
    let registry = create_test_registry();

    let ethnopic = registry.get("ethnopic");
    assert!(ethnopic.is_some());
    let provider = ethnopic.unwrap();

    assert_eq!(provider.id(), "ethnopic");
    assert_eq!(provider.name(), "Ethnopic");

    let models = provider.models();
    assert!(!models.is_empty());

    let model = provider.get_model("test-model-v2");
    assert!(model.is_some());
}

#[test]
fn test_chat_request_builder() {
    let request = ChatRequest::new(
        "gpt-4o",
        vec![Message::system("You are helpful"), Message::user("Hello")],
    )
    .with_temperature(0.7)
    .with_max_tokens(1000)
    .with_stream(true);

    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.temperature, Some(0.7));
    assert_eq!(request.max_tokens, Some(1000));
    assert_eq!(request.stream, Some(true));
}

#[test]
fn test_model_info_clone() {
    let model = ModelInfo {
        id: "test-model".to_string(),
        name: "Test Model".to_string(),
        provider: "test".to_string(),
        context_window: 128000,
        max_input_tokens: None,
        max_output_tokens: 4096,
        supports_vision: true,
        supports_tools: true,
        cost_per_million_input: 1.0,
        cost_per_million_output: 2.0,
        cost_per_million_cache_read: None,
        cost_per_million_cache_write: None,
    };

    let cloned = model.clone();
    assert_eq!(cloned.id, model.id);
    assert_eq!(cloned.context_window, model.context_window);
}

#[test]
fn test_all_providers_have_models() {
    let registry = create_test_registry();

    for provider in registry.list() {
        let models = provider.models();
        assert!(
            !models.is_empty(),
            "Provider {} should have models",
            provider.id()
        );
    }
}
