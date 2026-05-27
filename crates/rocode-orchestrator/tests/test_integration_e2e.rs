/// Integration tests for end-to-end dialogue flow (Phase 6.5)
///
/// Tests the complete dialogue flow from session creation to response handling.

use rocode_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use std::sync::Arc;

/// Mock provider for integration testing
struct MockDialogueProvider {
    responses: Vec<String>,
    current_index: std::sync::atomic::AtomicUsize,
}

impl MockDialogueProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            current_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn next_response(&self) -> String {
        let index = self
            .current_index
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.responses
            .get(index % self.responses.len())
            .cloned()
            .unwrap_or_else(|| "Default response".to_string())
    }
}

#[async_trait::async_trait]
impl rocode_provider::Provider for MockDialogueProvider {
    fn id(&self) -> &str {
        "mock-dialogue"
    }

    fn name(&self) -> &str {
        "Mock Dialogue Provider"
    }

    fn models(&self) -> Vec<rocode_provider::ModelInfo> {
        vec![rocode_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-dialogue".to_string(),
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

    fn get_model(&self, _id: &str) -> Option<&rocode_provider::ModelInfo> {
        None
    }

    async fn chat(
        &self,
        _request: rocode_provider::ChatRequest,
    ) -> Result<rocode_provider::ChatResponse, rocode_provider::ProviderError> {
        let response_text = self.next_response();

        Ok(rocode_provider::ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![rocode_provider::Choice {
                index: 0,
                message: rocode_provider::Message::assistant(&response_text),
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(rocode_provider::Usage {
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
        _request: rocode_provider::ChatRequest,
    ) -> Result<rocode_provider::StreamResult, rocode_provider::ProviderError> {
        use futures::stream;
        use rocode_provider::StreamEvent;

        let response_text = self.next_response();

        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta(response_text)),
            Ok(StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: rocode_provider::StreamUsage {
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
async fn test_e2e_single_turn_dialogue() {
    // 1. Create OrchestrationCore
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // 2. Register mock provider
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockDialogueProvider::new(vec![
            "Hello! How can I help you?".to_string(),
        ])));
    }

    // 3. Execute single-turn dialogue
    let options = PromptExecutionOptions {
        model: Some("mock-dialogue:mock-model".to_string()),
        ..Default::default()
    };

    let result = core
        .execute_prompt("e2e-single-turn", "Hello", options)
        .await
        .unwrap();

    // 4. Verify response
    assert_eq!(result.text, "Hello! How can I help you?");
    assert_eq!(result.session_id, "e2e-single-turn");
    assert!(result.usage.is_some());

    // 5. Verify session state
    let session = core.get_session("e2e-single-turn").await.unwrap();
    assert_eq!(session.messages.len(), 2); // User + Assistant
    assert_eq!(session.messages[0].role, "User");
    assert_eq!(session.messages[0].content, "Hello");
    assert_eq!(session.messages[1].role, "Assistant");
    assert_eq!(session.messages[1].content, "Hello! How can I help you?");
}

#[tokio::test]
async fn test_e2e_multi_turn_dialogue() {
    // 1. Create OrchestrationCore
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // 2. Register mock provider with multiple responses
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockDialogueProvider::new(vec![
            "I need your location.".to_string(),
            "It's sunny in San Francisco.".to_string(),
            "You're welcome!".to_string(),
        ])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-dialogue:mock-model".to_string()),
        ..Default::default()
    };

    let session_id = "e2e-multi-turn";

    // 3. Execute 3-turn dialogue
    // Turn 1
    let result1 = core
        .execute_prompt(session_id, "What's the weather?", options.clone())
        .await
        .unwrap();
    assert_eq!(result1.text, "I need your location.");

    // Turn 2
    let result2 = core
        .execute_prompt(session_id, "San Francisco", options.clone())
        .await
        .unwrap();
    assert_eq!(result2.text, "It's sunny in San Francisco.");

    // Turn 3
    let result3 = core
        .execute_prompt(session_id, "Thank you", options)
        .await
        .unwrap();
    assert_eq!(result3.text, "You're welcome!");

    // 4. Verify session state
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 6); // 3 user + 3 assistant

    // Verify message order
    assert_eq!(session.messages[0].content, "What's the weather?");
    assert_eq!(session.messages[1].content, "I need your location.");
    assert_eq!(session.messages[2].content, "San Francisco");
    assert_eq!(session.messages[3].content, "It's sunny in San Francisco.");
    assert_eq!(session.messages[4].content, "Thank you");
    assert_eq!(session.messages[5].content, "You're welcome!");
}

#[tokio::test]
async fn test_e2e_session_management() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockDialogueProvider::new(vec![
            "Response 1".to_string(),
            "Response 2".to_string(),
        ])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-dialogue:mock-model".to_string()),
        ..Default::default()
    };

    // 1. Create multiple sessions
    let _ = core
        .execute_prompt("session-1", "Hello 1", options.clone())
        .await
        .unwrap();

    let _ = core
        .execute_prompt("session-2", "Hello 2", options.clone())
        .await
        .unwrap();

    // 2. List sessions
    let sessions = core.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 2);

    // Find our sessions
    let session_ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(session_ids.contains(&"session-1"));
    assert!(session_ids.contains(&"session-2"));

    // 3. Query specific session
    let session1 = core.get_session("session-1").await.unwrap();
    assert_eq!(session1.id, "session-1");
    assert_eq!(session1.messages.len(), 2);
    assert_eq!(session1.messages[0].content, "Hello 1");

    let session2 = core.get_session("session-2").await.unwrap();
    assert_eq!(session2.id, "session-2");
    assert_eq!(session2.messages.len(), 2);
    assert_eq!(session2.messages[0].content, "Hello 2");
}

#[tokio::test]
async fn test_e2e_session_isolation() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockDialogueProvider::new(vec![
            "Response A".to_string(),
            "Response B".to_string(),
        ])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-dialogue:mock-model".to_string()),
        ..Default::default()
    };

    // Execute in different sessions
    let _ = core
        .execute_prompt("isolated-1", "Message A", options.clone())
        .await
        .unwrap();

    let _ = core
        .execute_prompt("isolated-2", "Message B", options.clone())
        .await
        .unwrap();

    // Verify sessions are isolated
    let session1 = core.get_session("isolated-1").await.unwrap();
    let session2 = core.get_session("isolated-2").await.unwrap();

    assert_eq!(session1.messages.len(), 2);
    assert_eq!(session2.messages.len(), 2);

    // Session 1 should only have its own messages
    assert_eq!(session1.messages[0].content, "Message A");
    assert_eq!(session1.messages[1].content, "Response A");

    // Session 2 should only have its own messages
    assert_eq!(session2.messages[0].content, "Message B");
    assert_eq!(session2.messages[1].content, "Response B");
}

#[tokio::test]
async fn test_e2e_usage_accumulation() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockDialogueProvider::new(vec![
            "Response 1".to_string(),
            "Response 2".to_string(),
        ])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-dialogue:mock-model".to_string()),
        ..Default::default()
    };

    let session_id = "usage-test";

    // Execute 2 turns
    let result1 = core
        .execute_prompt(session_id, "Turn 1", options.clone())
        .await
        .unwrap();

    let result2 = core
        .execute_prompt(session_id, "Turn 2", options)
        .await
        .unwrap();

    // Verify usage is reported for each turn
    assert!(result1.usage.is_some());
    assert!(result2.usage.is_some());

    let usage1 = result1.usage.unwrap();
    let usage2 = result2.usage.unwrap();

    // Each turn should have usage
    assert_eq!(usage1.input_tokens, 10);
    assert_eq!(usage1.output_tokens, 5);
    assert_eq!(usage2.input_tokens, 10);
    assert_eq!(usage2.output_tokens, 5);
}
