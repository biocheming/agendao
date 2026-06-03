/// Integration tests for error handling (Phase 6.5)
///
/// Tests error propagation and recovery across the orchestration layer.

use agendao_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use std::sync::Arc;

/// Mock provider that always fails
struct MockFailingProvider;

#[async_trait::async_trait]
impl agendao_provider::Provider for MockFailingProvider {
    fn id(&self) -> &str {
        "mock-failing"
    }

    fn name(&self) -> &str {
        "Mock Failing Provider"
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        vec![agendao_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-failing".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 4096,
            supports_vision: false,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        }]
    }

    fn get_model(&self, _id: &str) -> Option<&agendao_provider::ModelInfo> {
        None
    }

    async fn chat(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
        Err(agendao_provider::ProviderError::ApiError(
            "Simulated provider failure".to_string(),
        ))
    }

    async fn chat_stream(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
        Err(agendao_provider::ProviderError::ApiError(
            "Simulated streaming failure".to_string(),
        ))
    }
}

/// Mock provider for successful responses
struct MockSuccessProvider;

#[async_trait::async_trait]
impl agendao_provider::Provider for MockSuccessProvider {
    fn id(&self) -> &str {
        "mock-success"
    }

    fn name(&self) -> &str {
        "Mock Success Provider"
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        vec![agendao_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-success".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 4096,
            supports_vision: false,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
            cost_per_million_cache_read: None,
            cost_per_million_cache_write: None,
        }]
    }

    fn get_model(&self, _id: &str) -> Option<&agendao_provider::ModelInfo> {
        None
    }

    async fn chat(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
        Ok(agendao_provider::ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![agendao_provider::Choice {
                index: 0,
                message: agendao_provider::Message::assistant("Success response"),
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(agendao_provider::Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                cache_read_input_tokens: Some(0),
                cache_miss_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
            }),
        })
    }

    async fn chat_stream(
        &self,
        _request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
        use futures::stream;
        use agendao_provider::StreamEvent;

        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta("Success".to_string())),
            Ok(StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: agendao_provider::StreamUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    context_tokens: 10,
                    reasoning_tokens: 0,
                    cache_read_tokens: 0,
                    cache_miss_tokens: 0,
                    cache_write_tokens: 0,
                },
                provider_metadata: None,
            }),
            Ok(StreamEvent::Done),
        ];

        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn test_provider_error_handling() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockFailingProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-failing:mock-model".to_string()),
        ..Default::default()
    };

    // Execute dialogue - should fail
    let result = core
        .execute_prompt("error-test", "Test", options)
        .await;

    // Verify error is returned
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Simulated provider failure"));
}

#[tokio::test]
async fn test_streaming_error_handling() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockFailingProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-failing:mock-model".to_string()),
        ..Default::default()
    };

    // Execute streaming dialogue - should fail
    let result = core
        .execute_prompt_streaming("stream-error-test", "Test", options)
        .await;

    // Verify error is returned
    assert!(result.is_err());
    if let Err(err) = result {
        assert!(err.to_string().contains("Simulated streaming failure"));
    }
}

#[tokio::test]
async fn test_invalid_model_spec() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockSuccessProvider));
    }

    // Use invalid model spec (non-existent provider)
    let options = PromptExecutionOptions {
        model: Some("non-existent:model".to_string()),
        ..Default::default()
    };

    let result = core
        .execute_prompt("invalid-model-test", "Test", options)
        .await;

    // Verify error is returned
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Provider not found"));
}

#[tokio::test]
async fn test_session_not_found() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Try to get non-existent session
    let result = core.get_session("non-existent-session").await;

    // Verify error is returned
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Session not found"));
}

#[tokio::test]
async fn test_session_state_consistency_after_error() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockSuccessProvider));
        providers.register_arc(Arc::new(MockFailingProvider));
    }

    let session_id = "consistency-test";

    // First turn - success
    let options1 = PromptExecutionOptions {
        model: Some("mock-success:mock-model".to_string()),
        ..Default::default()
    };

    let result1 = core
        .execute_prompt(session_id, "Turn 1", options1)
        .await
        .unwrap();

    assert_eq!(result1.text, "Success response");

    // Second turn - failure
    let options2 = PromptExecutionOptions {
        model: Some("mock-failing:mock-model".to_string()),
        ..Default::default()
    };

    let result2 = core
        .execute_prompt(session_id, "Turn 2", options2)
        .await;

    assert!(result2.is_err());

    // Verify session state is consistent
    // Note: Failed requests still add user message to session
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 3); // Turn 1 (user + assistant) + Turn 2 (user only, failed)
    assert_eq!(session.messages[0].content, "Turn 1");
    assert_eq!(session.messages[1].content, "Success response");
    assert_eq!(session.messages[2].content, "Turn 2");
}

#[tokio::test]
async fn test_error_recovery() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockSuccessProvider));
        providers.register_arc(Arc::new(MockFailingProvider));
    }

    let session_id = "recovery-test";

    // Turn 1 - success
    let options1 = PromptExecutionOptions {
        model: Some("mock-success:mock-model".to_string()),
        ..Default::default()
    };

    let _ = core
        .execute_prompt(session_id, "Turn 1", options1)
        .await
        .unwrap();

    // Turn 2 - failure
    let options2 = PromptExecutionOptions {
        model: Some("mock-failing:mock-model".to_string()),
        ..Default::default()
    };

    let _ = core
        .execute_prompt(session_id, "Turn 2", options2)
        .await;

    // Turn 3 - recovery (success again)
    let options3 = PromptExecutionOptions {
        model: Some("mock-success:mock-model".to_string()),
        ..Default::default()
    };

    let result3 = core
        .execute_prompt(session_id, "Turn 3", options3)
        .await
        .unwrap();

    assert_eq!(result3.text, "Success response");

    // Verify session has recovered and continued
    // Note: Failed requests still add user message to session
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 5); // Turn 1 (user + assistant) + Turn 2 (user only, failed) + Turn 3 (user + assistant)
    assert_eq!(session.messages[0].content, "Turn 1");
    assert_eq!(session.messages[1].content, "Success response");
    assert_eq!(session.messages[2].content, "Turn 2");
    assert_eq!(session.messages[3].content, "Turn 3");
    assert_eq!(session.messages[4].content, "Success response");
}
