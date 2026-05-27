/// Performance benchmarks (Phase 6.4 — revised 2026-05-27)
///
/// All measurements use a mock provider — numbers reflect framework overhead,
/// NOT end-to-end LLM latency (which is dominated by network + model inference).
/// Mock-provider measurements are labeled "framework" to distinguish them from
/// "end-to-end" measurements that would require a real provider.
///
/// Testing methodology:
///   - `Instant::now()` for single-operation timing
///   - Repeated samples (warm-up + measurement rounds)
///   - Soft assertions (verify no catastrophic regression, not specific µs targets)

use rocode_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use std::sync::Arc;
use std::time::Instant;

/// Mock provider for performance testing
struct MockProvider;

#[async_trait::async_trait]
impl rocode_provider::Provider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn name(&self) -> &str { "Mock Provider" }

    fn models(&self) -> Vec<rocode_provider::ModelInfo> {
        vec![rocode_provider::ModelInfo {
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

    fn get_model(&self, _id: &str) -> Option<&rocode_provider::ModelInfo> { None }

    async fn chat(
        &self,
        _request: rocode_provider::ChatRequest,
    ) -> Result<rocode_provider::ChatResponse, rocode_provider::ProviderError> {
        Ok(rocode_provider::ChatResponse {
            id: "test".to_string(),
            model: "mock-model".to_string(),
            choices: vec![rocode_provider::Choice {
                index: 0,
                message: rocode_provider::Message::assistant("Test response"),
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
        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta("Test".to_string())),
            Ok(StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: rocode_provider::StreamUsage {
                    prompt_tokens: 10, completion_tokens: 1, context_tokens: 10,
                    reasoning_tokens: 0, cache_read_tokens: 0, cache_miss_tokens: 0,
                    cache_write_tokens: 0,
                },
                provider_metadata: None,
            }),
            Ok(StreamEvent::Done),
        ];
        Ok(Box::pin(stream::iter(events)))
    }
}

/// Mock tool for tool-call overhead measurement.
struct MockEchoTool;

#[async_trait::async_trait]
impl rocode_tool::Tool for MockEchoTool {
    fn id(&self) -> &str { "echo" }
    fn description(&self) -> &str { "Echo tool for perf testing" }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "message": { "type": "string" } },
            "required": ["message"]
        })
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: rocode_tool::ToolContext,
    ) -> Result<rocode_tool::ToolResult, rocode_tool::ToolError> {
        let msg = input.get("message").and_then(|v| v.as_str()).unwrap_or("");
        Ok(rocode_tool::ToolResult {
            output: format!("echo: {}", msg),
            title: String::new(),
            metadata: std::collections::HashMap::new(),
            truncated: false,
        })
    }
}

fn measure(label: &str, samples: u32, mut f: impl FnMut() -> std::time::Duration) {
    // Warm-up
    for _ in 0..3 { f(); }
    let mut total = std::time::Duration::ZERO;
    let mut min = std::time::Duration::MAX;
    let mut max = std::time::Duration::ZERO;
    for _ in 0..samples {
        let d = f();
        total += d;
        if d < min { min = d; }
        if d > max { max = d; }
    }
    let avg = total / samples;
    println!("{label}: avg={avg:?} min={min:?} max={max:?} (n={samples})");
}

// ── Phase 6.4 补充基准 ──────────────────────────────────────

#[tokio::test]
async fn test_cold_start_with_shared_authorities() {
    // Measure cold start using shared authorities (config + sessions).
    let config = rocode_config::Config::default();
    let config_store = Arc::new(rocode_config::ConfigStore::new(config.clone()));
    let sessions = Arc::new(tokio::sync::Mutex::new(
        rocode_session_core::SessionManager::new(),
    ));
    let providers = Arc::new(tokio::sync::RwLock::new(
        rocode_provider::ProviderRegistry::new(),
    ));
    let tools = Arc::new(tokio::sync::RwLock::new(
        rocode_tool::ToolRegistry::new(),
    ));

    measure("shared-authority cold start", 10, || {
        let start = Instant::now();
        let _core = rocode_orchestrator::OrchestrationCore::<
            rocode_session_core::SessionManager,
        >::new_with_shared_authorities(
            Arc::clone(&config_store),
            Arc::clone(&sessions),
            Arc::clone(&providers),
            Arc::clone(&tools),
        );
        start.elapsed()
    });

    // Sanity: core should be functional.
    let core = rocode_orchestrator::OrchestrationCore::<
        rocode_session_core::SessionManager,
    >::new_with_shared_authorities(config_store, sessions, providers, tools);
    assert!(core.list_sessions().await.is_ok());
}

#[tokio::test]
async fn test_tool_registration_overhead() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    let start = Instant::now();
    core.tools().write().await.register(MockEchoTool).await;
    let duration = start.elapsed();

    println!("Tool registration (1 tool): {:?}", duration);
    assert!(core.tools().read().await.get("echo").await.is_some());
    assert!(duration.as_millis() < 100, "Tool registration too slow: {:?}", duration);
}

#[tokio::test]
async fn test_concurrent_session_creation() {
    let config = rocode_config::Config::default();
    let core = Arc::new(OrchestrationCore::new(&config).await.unwrap());
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockProvider));
    }
    const SESSION_COUNT: usize = 100;

    let mut handles = Vec::new();
    let start = Instant::now();
    for i in 0..SESSION_COUNT {
        let session_id = format!("perf-concurrent-{}", i);
        let core = Arc::clone(&core);
        handles.push(tokio::spawn(async move {
            let opts = PromptExecutionOptions {
                model: Some("mock:mock-model".to_string()),
                ..Default::default()
            };
            let result = core.execute_prompt(&session_id, "bench", opts).await;
            (session_id, result)
        }));
    }

    let mut created = 0;
    for h in handles {
        if h.await.unwrap().1.is_ok() {
            created += 1;
        }
    }
    let duration = start.elapsed();

    println!(
        "Concurrent session creation: {created}/{SESSION_COUNT} ok in {duration:?} \
         ({:.1} sessions/sec)",
        created as f64 / duration.as_secs_f64()
    );

    // Verify sessions are visible.
    let sessions = core.list_sessions().await.unwrap();
    println!("  list_sessions after creation: {} sessions", sessions.len());

    assert!(created > 0, "At least some sessions should succeed");
}

#[tokio::test]
async fn test_provider_hot_reload_visibility() {
    // Verify that a provider registered after core construction is
    // immediately visible (shared-authority path).
    let config = rocode_config::Config::default();
    let config_store = Arc::new(rocode_config::ConfigStore::new(config.clone()));
    let sessions = Arc::new(tokio::sync::Mutex::new(
        rocode_session_core::SessionManager::new(),
    ));
    let providers = Arc::new(tokio::sync::RwLock::new(
        rocode_provider::ProviderRegistry::new(),
    ));
    let tools = Arc::new(tokio::sync::RwLock::new(
        rocode_tool::ToolRegistry::new(),
    ));

    let core = rocode_orchestrator::OrchestrationCore::<
        rocode_session_core::SessionManager,
    >::new_with_shared_authorities(
        Arc::clone(&config_store),
        Arc::clone(&sessions),
        Arc::clone(&providers),
        Arc::clone(&tools),
    );

    // Before registration — mock should not be visible.
    {
        let p = core.providers().read().await;
        assert!(p.get("mock").is_none(), "mock provider should NOT be visible yet");
    }

    // Register via shared Arc — must be visible immediately.
    {
        let mut p = core.providers().write().await;
        p.register_arc(Arc::new(MockProvider));
    }

    {
        let p = core.providers().read().await;
        assert!(p.get("mock").is_some(), "mock provider MUST be visible after registration");
    }

    println!("Provider hot-reload visibility: OK (no restart needed)");
}

#[tokio::test]
async fn test_cold_start_performance() {
    // Measure cold start time
    let start = Instant::now();
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();
    let duration = start.elapsed();

    println!("Cold start time: {:?}", duration);

    // Verify core is functional
    assert!(core.list_sessions().await.is_ok());

    // Target: < 100ms for Direct Transport
    // This is a soft check - actual performance depends on hardware
    // We just verify it's not catastrophically slow (> 1 second)
    assert!(
        duration.as_millis() < 1000,
        "Cold start took {:?}, expected < 1s",
        duration
    );
}

#[tokio::test]
async fn test_startup_with_provider() {
    let start = Instant::now();
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Register provider
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockProvider));
    }

    let duration = start.elapsed();
    println!("Startup with provider: {:?}", duration);

    // Verify provider is registered
    let providers = core.providers().read().await;
    assert!(providers.get("mock").is_some());

    assert!(
        duration.as_millis() < 1000,
        "Startup with provider took {:?}, expected < 1s",
        duration
    );
}

#[tokio::test]
async fn test_single_turn_execution_performance() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock:mock-model".to_string()),
        ..Default::default()
    };

    // Measure execution time
    let start = Instant::now();
    let result = core
        .execute_prompt("perf-test-session", "Hello", options)
        .await
        .unwrap();
    let duration = start.elapsed();

    println!("Single-turn execution: {:?}", duration);
    assert_eq!(result.text, "Test response");

    // Should be fast since it's just a mock provider
    assert!(
        duration.as_millis() < 500,
        "Single-turn execution took {:?}, expected < 500ms",
        duration
    );
}

#[tokio::test]
async fn test_streaming_ttfb() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock:mock-model".to_string()),
        ..Default::default()
    };

    // Measure time to first byte
    let start = Instant::now();
    let mut stream = core
        .execute_prompt_streaming("perf-test-stream", "Hello", options)
        .await
        .unwrap();

    use futures::StreamExt;
    let first_event = stream.next().await;
    let ttfb = start.elapsed();

    println!("Streaming TTFB: {:?}", ttfb);
    assert!(first_event.is_some());

    // Target: < 50ms TTFB
    // Soft check - just verify it's reasonable
    assert!(
        ttfb.as_millis() < 200,
        "TTFB was {:?}, expected < 200ms",
        ttfb
    );
}

#[tokio::test]
async fn test_multi_turn_performance() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockProvider));
    }

    let options = PromptExecutionOptions {
        model: Some("mock:mock-model".to_string()),
        ..Default::default()
    };

    let session_id = "perf-multi-turn";

    // Measure 3-turn dialogue
    let start = Instant::now();

    let _ = core
        .execute_prompt(session_id, "Hello", options.clone())
        .await
        .unwrap();

    let _ = core
        .execute_prompt(session_id, "How are you?", options.clone())
        .await
        .unwrap();

    let _ = core
        .execute_prompt(session_id, "Goodbye", options)
        .await
        .unwrap();

    let duration = start.elapsed();
    println!("3-turn dialogue: {:?}", duration);

    // Verify session has 6 messages (3 user + 3 assistant)
    let session = core.get_session(session_id).await.unwrap();
    assert_eq!(session.messages.len(), 6);

    assert!(
        duration.as_millis() < 1500,
        "3-turn dialogue took {:?}, expected < 1.5s",
        duration
    );
}

#[tokio::test]
async fn test_session_creation_performance() {
    let config = rocode_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Measure session creation time (implicit via get_session)
    let start = Instant::now();
    let result = core.get_session("new-session-perf-test").await;
    let duration = start.elapsed();

    println!("Session creation (via get_session): {:?}", duration);

    // Session doesn't exist yet, so this should error
    assert!(result.is_err());

    // But the check should be fast
    assert!(
        duration.as_millis() < 100,
        "Session lookup took {:?}, expected < 100ms",
        duration
    );
}
