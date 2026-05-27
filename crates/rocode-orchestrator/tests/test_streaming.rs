/// Integration test for streaming prompt execution (Phase 6.3)

use rocode_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use rocode_provider::{
    ChatRequest, ChatResponse, Message, ModelInfo, Provider, ProviderError, StreamEvent,
    StreamResult, Usage,
};
use async_trait::async_trait;
use futures::stream;
use std::sync::Arc;

/// Mock provider that returns a streaming response
struct MockStreamingProvider;

#[async_trait]
impl Provider for MockStreamingProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock Streaming Provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock".to_string(),
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

    fn get_model(&self, _id: &str) -> Option<&ModelInfo> {
        None
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![rocode_provider::Choice {
                index: 0,
                message: Message::assistant("Hello from mock provider"),
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
                cache_read_input_tokens: Some(0),
                cache_miss_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
            }),
        })
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        // Create a simple streaming response
        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta("Hello".to_string())),
            Ok(StreamEvent::TextDelta(" from".to_string())),
            Ok(StreamEvent::TextDelta(" streaming".to_string())),
            Ok(StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: rocode_provider::StreamUsage {
                    prompt_tokens: 10,
                    completion_tokens: 3,
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
async fn test_streaming_basic() {
    // Create orchestration core
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Register mock provider
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider));
    }

    // Execute streaming prompt
    let options = PromptExecutionOptions {
        model: Some("mock:mock-model".to_string()),
        ..Default::default()
    };

    let stream = core
        .execute_prompt_streaming("test-session", "Hello", options)
        .await
        .unwrap();

    // Collect events
    use futures::StreamExt;
    let events: Vec<_> = stream.collect().await;

    // Verify events
    assert!(!events.is_empty(), "Stream should produce events");

    // Check for Start event
    assert!(
        events.iter().any(|e| matches!(e, Ok(StreamEvent::Start))),
        "Stream should start with Start event"
    );

    // Check for TextDelta events
    let text_deltas: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Ok(StreamEvent::TextDelta(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(!text_deltas.is_empty(), "Stream should have text deltas");

    // Check for Done event
    assert!(
        events.iter().any(|e| matches!(e, Ok(StreamEvent::Done))),
        "Stream should end with Done event"
    );

    // Verify session was updated
    let session_detail = core.get_session("test-session").await.unwrap();
    assert_eq!(session_detail.messages.len(), 2); // User + Assistant
    assert_eq!(session_detail.messages[0].role, "User");
    assert_eq!(session_detail.messages[1].role, "Assistant");
}

#[tokio::test]
async fn test_streaming_accumulates_text() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock:mock-model".to_string()),
        ..Default::default()
    };

    let stream = core
        .execute_prompt_streaming("test-session-2", "Test", options)
        .await
        .unwrap();

    use futures::StreamExt;
    let events: Vec<_> = stream.collect().await;

    // Collect all text deltas
    let accumulated_text: String = events
        .iter()
        .filter_map(|e| match e {
            Ok(StreamEvent::TextDelta(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(accumulated_text, "Hello from streaming");

    // Verify session has the complete text
    let session_detail = core.get_session("test-session-2").await.unwrap();
    assert_eq!(session_detail.messages.len(), 2);
    assert_eq!(
        session_detail.messages[1].content,
        "Hello from streaming"
    );
}
