/// Integration tests for streaming output (Phase 6.5)
///
/// Tests the complete streaming flow including multi-turn dialogues.

use rocode_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use std::sync::Arc;

/// Mock streaming provider
struct MockStreamingProvider {
    responses: Vec<Vec<String>>,
    current_index: std::sync::atomic::AtomicUsize,
}

impl MockStreamingProvider {
    fn new(responses: Vec<Vec<String>>) -> Self {
        Self {
            responses,
            current_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn next_response(&self) -> Vec<String> {
        let index = self
            .current_index
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.responses
            .get(index % self.responses.len())
            .cloned()
            .unwrap_or_else(|| vec!["Default".to_string()])
    }
}

#[async_trait::async_trait]
impl rocode_provider::Provider for MockStreamingProvider {
    fn id(&self) -> &str {
        "mock-streaming"
    }

    fn name(&self) -> &str {
        "Mock Streaming Provider"
    }

    fn models(&self) -> Vec<rocode_provider::ModelInfo> {
        vec![rocode_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-streaming".to_string(),
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
        let chunks = self.next_response();
        let text = chunks.join("");

        Ok(rocode_provider::ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![rocode_provider::Choice {
                index: 0,
                message: rocode_provider::Message::assistant(&text),
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

        let chunks = self.next_response();
        let mut events = vec![Ok(StreamEvent::Start)];

        for chunk in chunks {
            events.push(Ok(StreamEvent::TextDelta(chunk)));
        }

        events.push(Ok(StreamEvent::FinishStep {
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
        }));
        events.push(Ok(StreamEvent::Done));

        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn test_streaming_e2e() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider::new(vec![vec![
            "Hello".to_string(),
            " from".to_string(),
            " streaming".to_string(),
        ]])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-streaming:mock-model".to_string()),
        ..Default::default()
    };

    // Execute streaming dialogue
    let stream = core
        .execute_prompt_streaming("stream-e2e", "Test", options)
        .await
        .unwrap();

    // Collect all events
    use futures::StreamExt;
    let events: Vec<_> = stream.collect().await;

    // Verify events
    assert!(!events.is_empty());

    // Check for Start event
    assert!(events
        .iter()
        .any(|e| matches!(e, Ok(rocode_provider::StreamEvent::Start))));

    // Check for TextDelta events
    let text_deltas: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Ok(rocode_provider::StreamEvent::TextDelta(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(!text_deltas.is_empty());

    // Check for Done event
    assert!(events
        .iter()
        .any(|e| matches!(e, Ok(rocode_provider::StreamEvent::Done))));

    // Verify session state
    let session = core.get_session("stream-e2e").await.unwrap();
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[1].content, "Hello from streaming");
}

#[tokio::test]
async fn test_streaming_multi_turn() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider::new(vec![
            vec!["First".to_string(), " response".to_string()],
            vec!["Second".to_string(), " response".to_string()],
        ])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-streaming:mock-model".to_string()),
        ..Default::default()
    };

    let session_id = "stream-multi";

    // Turn 1
    let stream1 = core
        .execute_prompt_streaming(session_id, "Turn 1", options.clone())
        .await
        .unwrap();

    use futures::StreamExt;
    let _events1: Vec<_> = stream1.collect().await;

    // Turn 2
    let stream2 = core
        .execute_prompt_streaming(session_id, "Turn 2", options)
        .await
        .unwrap();

    let _events2: Vec<_> = stream2.collect().await;

    // Verify session has both turns
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 4); // 2 user + 2 assistant

    assert_eq!(session.messages[0].content, "Turn 1");
    assert_eq!(session.messages[1].content, "First response");
    assert_eq!(session.messages[2].content, "Turn 2");
    assert_eq!(session.messages[3].content, "Second response");
}

#[tokio::test]
async fn test_streaming_text_accumulation() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider::new(vec![vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ]])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-streaming:mock-model".to_string()),
        ..Default::default()
    };

    let stream = core
        .execute_prompt_streaming("stream-accum", "Test", options)
        .await
        .unwrap();

    use futures::StreamExt;
    let events: Vec<_> = stream.collect().await;

    // Collect all text deltas
    let accumulated: String = events
        .iter()
        .filter_map(|e| match e {
            Ok(rocode_provider::StreamEvent::TextDelta(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(accumulated, "ABCD");

    // Verify session has complete text
    let session = core.get_session("stream-accum").await.unwrap();
    assert_eq!(session.messages[1].content, "ABCD");
}

#[tokio::test]
async fn test_streaming_vs_non_streaming_consistency() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider::new(vec![vec![
            "Consistent".to_string(),
            " response".to_string(),
        ]])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-streaming:mock-model".to_string()),
        ..Default::default()
    };

    // Non-streaming
    let result_non_stream = core
        .execute_prompt("consistency-non-stream", "Test", options.clone())
        .await
        .unwrap();

    // Streaming
    let stream = core
        .execute_prompt_streaming("consistency-stream", "Test", options)
        .await
        .unwrap();

    use futures::StreamExt;
    let _events: Vec<_> = stream.collect().await;

    // Both should produce the same final text
    let session_non_stream = core.get_session("consistency-non-stream").await.unwrap();
    let session_stream = core.get_session("consistency-stream").await.unwrap();

    assert_eq!(result_non_stream.text, "Consistent response");
    assert_eq!(session_non_stream.messages[1].content, "Consistent response");
    assert_eq!(session_stream.messages[1].content, "Consistent response");
}

#[tokio::test]
async fn test_streaming_event_order() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockStreamingProvider::new(vec![vec![
            "Test".to_string(),
        ]])));
    }

    let options = PromptExecutionOptions {
        model: Some("mock-streaming:mock-model".to_string()),
        ..Default::default()
    };

    let stream = core
        .execute_prompt_streaming("stream-order", "Test", options)
        .await
        .unwrap();

    use futures::StreamExt;
    let events: Vec<_> = stream.collect().await;

    // Verify event order: Start -> TextDelta* -> FinishStep -> Done
    let mut saw_start = false;
    let mut saw_text = false;
    let mut saw_finish = false;
    let mut saw_done = false;

    for event in events {
        match event {
            Ok(rocode_provider::StreamEvent::Start) => {
                assert!(!saw_start);
                saw_start = true;
            }
            Ok(rocode_provider::StreamEvent::TextDelta(_)) => {
                assert!(saw_start);
                assert!(!saw_finish);
                assert!(!saw_done);
                saw_text = true;
            }
            Ok(rocode_provider::StreamEvent::FinishStep { .. }) => {
                assert!(saw_start);
                assert!(!saw_finish);
                assert!(!saw_done);
                saw_finish = true;
            }
            Ok(rocode_provider::StreamEvent::Done) => {
                assert!(saw_start);
                assert!(saw_finish);
                assert!(!saw_done);
                saw_done = true;
            }
            _ => {}
        }
    }

    assert!(saw_start);
    assert!(saw_text);
    assert!(saw_finish);
    assert!(saw_done);
}
