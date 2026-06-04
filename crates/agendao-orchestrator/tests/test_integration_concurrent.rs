/// Integration tests for concurrent scenarios (Phase 6.5)
///
/// Tests concurrent access patterns and session isolation.
use agendao_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use std::sync::Arc;

/// Mock provider for concurrent testing
struct MockConcurrentProvider {
    response_prefix: String,
}

impl MockConcurrentProvider {
    fn new(prefix: &str) -> Self {
        Self {
            response_prefix: prefix.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl agendao_provider::Provider for MockConcurrentProvider {
    fn id(&self) -> &str {
        "mock-concurrent"
    }

    fn name(&self) -> &str {
        "Mock Concurrent Provider"
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        vec![agendao_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-concurrent".to_string(),
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
        request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::ChatResponse, agendao_provider::ProviderError> {
        // Extract user message
        let user_msg = request
            .messages
            .iter()
            .filter(|m| matches!(m.role, agendao_provider::Role::User))
            .last()
            .and_then(|m| match &m.content {
                agendao_provider::Content::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("");

        let response_text = format!("{}: {}", self.response_prefix, user_msg);

        Ok(agendao_provider::ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![agendao_provider::Choice {
                index: 0,
                message: agendao_provider::Message::assistant(&response_text),
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
        request: agendao_provider::ChatRequest,
    ) -> Result<agendao_provider::StreamResult, agendao_provider::ProviderError> {
        use agendao_provider::StreamEvent;
        use futures::stream;

        let user_msg = request
            .messages
            .iter()
            .filter(|m| matches!(m.role, agendao_provider::Role::User))
            .last()
            .and_then(|m| match &m.content {
                agendao_provider::Content::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("");

        let response_text = format!("{}: {}", self.response_prefix, user_msg);

        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta(response_text)),
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
async fn test_concurrent_sessions() {
    let config = agendao_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockConcurrentProvider::new("Response")));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-concurrent:mock-model".to_string()),
        ..Default::default()
    };

    // Create 10 concurrent sessions
    let mut handles = vec![];

    for i in 0..10 {
        let core_clone = Arc::clone(&core);
        let options_clone = options.clone();
        let session_id = format!("concurrent-session-{}", i);
        let message = format!("Message {}", i);

        let handle = tokio::spawn(async move {
            core_clone
                .execute_prompt(&session_id, &message, options_clone)
                .await
        });

        handles.push((i, handle));
    }

    // Wait for all to complete
    let mut results = vec![];
    for (i, handle) in handles {
        let result = handle.await.unwrap().unwrap();
        results.push((i, result));
    }

    // Verify all sessions completed successfully
    assert_eq!(results.len(), 10);

    // Verify each session has correct state
    for (i, result) in results {
        let expected_text = format!("Response: Message {}", i);
        assert_eq!(result.text, expected_text);

        let session_id = format!("concurrent-session-{}", i);
        let session = core.get_session(&session_id).await.unwrap();
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].content, format!("Message {}", i));
        assert_eq!(session.messages[1].content, expected_text);
    }
}

#[tokio::test]
async fn test_concurrent_requests_same_session() {
    let config = agendao_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockConcurrentProvider::new("Response")));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-concurrent:mock-model".to_string()),
        ..Default::default()
    };

    let session_id = "same-session";

    // Launch 5 concurrent requests to the same session
    let mut handles = vec![];

    for i in 0..5 {
        let core_clone = Arc::clone(&core);
        let options_clone = options.clone();
        let message = format!("Message {}", i);

        let handle = tokio::spawn(async move {
            core_clone
                .execute_prompt(session_id, &message, options_clone)
                .await
        });

        handles.push(handle);
    }

    // Wait for all to complete
    let mut results = vec![];
    for handle in handles {
        let result = handle.await.unwrap().unwrap();
        results.push(result);
    }

    // Verify all requests completed
    assert_eq!(results.len(), 5);

    // Verify session has all messages (order may vary due to concurrency)
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 10); // 5 user + 5 assistant

    // Count user and assistant messages
    let user_count = session.messages.iter().filter(|m| m.role == "User").count();
    let assistant_count = session
        .messages
        .iter()
        .filter(|m| m.role == "Assistant")
        .count();

    assert_eq!(user_count, 5);
    assert_eq!(assistant_count, 5);
}

#[tokio::test]
async fn test_concurrent_streaming() {
    let config = agendao_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockConcurrentProvider::new("Stream")));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-concurrent:mock-model".to_string()),
        ..Default::default()
    };

    // Create 5 concurrent streaming sessions
    let mut handles = vec![];

    for i in 0..5 {
        let core_clone = Arc::clone(&core);
        let options_clone = options.clone();
        let session_id = format!("stream-concurrent-{}", i);
        let message = format!("Stream {}", i);

        let handle = tokio::spawn(async move {
            let stream = core_clone
                .execute_prompt_streaming(&session_id, &message, options_clone)
                .await
                .unwrap();

            use futures::StreamExt;
            let events: Vec<_> = stream.collect().await;
            (i, events)
        });

        handles.push(handle);
    }

    // Wait for all to complete
    let mut results = vec![];
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // Verify all streams completed
    assert_eq!(results.len(), 5);

    // Verify each stream received events
    for (i, events) in results {
        assert!(!events.is_empty());

        // Verify session state
        let session_id = format!("stream-concurrent-{}", i);
        let session = core.get_session(&session_id).await.unwrap();
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].content, format!("Stream {}", i));
    }
}

#[tokio::test]
async fn test_provider_concurrent_access() {
    let config = agendao_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockConcurrentProvider::new("Concurrent")));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-concurrent:mock-model".to_string()),
        ..Default::default()
    };

    // Launch 20 concurrent requests across different sessions
    let mut handles = vec![];

    for i in 0..20 {
        let core_clone = Arc::clone(&core);
        let options_clone = options.clone();
        let session_id = format!("provider-test-{}", i);
        let message = format!("Test {}", i);

        let handle = tokio::spawn(async move {
            core_clone
                .execute_prompt(&session_id, &message, options_clone)
                .await
        });

        handles.push(handle);
    }

    // Wait for all to complete
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap().is_ok() {
            success_count += 1;
        }
    }

    // Verify all requests succeeded
    assert_eq!(success_count, 20);
}

#[tokio::test]
async fn test_session_isolation_under_concurrency() {
    let config = agendao_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockConcurrentProvider::new("Isolated")));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-concurrent:mock-model".to_string()),
        ..Default::default()
    };

    // Create 3 sessions with 3 turns each, all concurrent
    let mut handles = vec![];

    for session_i in 0..3 {
        for turn_i in 0..3 {
            let core_clone = Arc::clone(&core);
            let options_clone = options.clone();
            let session_id = format!("isolated-{}", session_i);
            let message = format!("S{}T{}", session_i, turn_i);

            let handle = tokio::spawn(async move {
                core_clone
                    .execute_prompt(&session_id, &message, options_clone)
                    .await
            });

            handles.push((session_i, turn_i, handle));
        }
    }

    // Wait for all to complete
    for (_, _, handle) in handles {
        handle.await.unwrap().unwrap();
    }

    // Verify each session has exactly 6 messages (3 turns)
    for session_i in 0..3 {
        let session_id = format!("isolated-{}", session_i);
        let session = core.get_session(&session_id).await.unwrap();
        assert_eq!(session.messages.len(), 6); // 3 user + 3 assistant

        // Verify all messages belong to this session
        for msg in &session.messages {
            if msg.role == "User" {
                assert!(msg.content.starts_with(&format!("S{}", session_i)));
            }
        }
    }
}
