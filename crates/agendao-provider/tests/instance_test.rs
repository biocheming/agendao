use agendao_provider::{ModelInfo, Provider, ProviderConfig, ProviderInstance};
use std::collections::HashMap;
use std::sync::Arc;

fn create_test_model(id: &str, provider: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        name: id.to_string(),
        provider: provider.to_string(),
        context_window: 128000,
        max_input_tokens: None,
        max_output_tokens: 4096,
        supports_vision: true,
        supports_tools: true,
        cost_per_million_input: 1.0,
        cost_per_million_output: 2.0,
        cost_per_million_cache_read: None,
        cost_per_million_cache_write: None,
    }
}

#[test]
fn test_provider_instance_metadata() {
    let config = ProviderConfig::new("test", "https://api.test.com", "sk-test");
    let models = HashMap::from([("model-a".to_string(), create_test_model("model-a", "test"))]);

    let instance = ProviderInstance::new(
        "test".to_string(),
        "Test Provider".to_string(),
        config,
        Arc::new(agendao_provider::protocols::CloseAiCompatibleAdapter::new()),
        models,
    );

    assert_eq!(instance.id(), "test");
    assert_eq!(instance.name(), "Test Provider");
    assert_eq!(instance.models().len(), 1);
    assert!(instance.get_model("model-a").is_some());
    assert!(instance.get_model("unknown").is_none());
}

#[test]
fn test_provider_instance_models_iterator() {
    let config = ProviderConfig::new("test", "https://api.test.com", "sk-test");
    let models = HashMap::from([
        ("model-a".to_string(), create_test_model("model-a", "test")),
        ("model-b".to_string(), create_test_model("model-b", "test")),
    ]);

    let instance = ProviderInstance::new(
        "test".to_string(),
        "Test".to_string(),
        config,
        Arc::new(agendao_provider::protocols::CloseAiCompatibleAdapter::new()),
        models,
    );

    let model_ids: Vec<String> = instance.models().iter().map(|m| m.id.clone()).collect();
    assert!(model_ids.iter().any(|id| id == "model-a"));
    assert!(model_ids.iter().any(|id| id == "model-b"));
}

#[test]
fn test_closeai_compatible_adapter_creation() {
    let adapter = agendao_provider::protocols::CloseAiCompatibleAdapter::new();
    let _arc: Arc<dyn agendao_provider::ProviderAdapter> = Arc::new(adapter);
}

#[test]
fn test_ethnopic_adapter_creation() {
    let adapter = agendao_provider::protocols::EthnopicAdapter::new();
    let _arc: Arc<dyn agendao_provider::ProviderAdapter> = Arc::new(adapter);
}
