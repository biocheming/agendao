/// Integration boundary tests (Phase 6.5 follow-up)
///
/// Covers the historical regression points and mixed-transport edge cases
/// identified in the followup plan:
///   1. continue_last pure continue (empty text)
///   2. tool result visibility after unified authority
///   3. same-session concurrent request history consistency
///   4. provider hot-reload visibility
///   5. session state consistency across prompt rounds
use agendao_orchestrator::{OrchestrationCore, PromptExecutionOptions, SessionStore};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ── Shared test infrastructure ────────────────────────────

struct MockBoundaryProvider {
    responses: Vec<String>,
    call_count: AtomicUsize,
}

impl MockBoundaryProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl agendao_provider::Provider for MockBoundaryProvider {
    fn id(&self) -> &str {
        "mock-boundary"
    }
    fn name(&self) -> &str {
        "Mock Boundary Provider"
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        vec![agendao_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-boundary".to_string(),
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
        let i = self.call_count.fetch_add(1, Ordering::SeqCst);
        let text = self
            .responses
            .get(i % self.responses.len())
            .cloned()
            .unwrap_or_else(|| "Default".to_string());
        Ok(agendao_provider::ChatResponse {
            id: format!("resp-{}", i),
            model: "mock-model".to_string(),
            choices: vec![agendao_provider::Choice {
                index: 0,
                message: agendao_provider::Message::assistant(&text),
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
        Err(agendao_provider::ProviderError::ApiError(
            "streaming not implemented".into(),
        ))
    }
}

fn default_options() -> PromptExecutionOptions {
    PromptExecutionOptions {
        model: Some("mock-boundary:mock-model".to_string()),
        ..Default::default()
    }
}

async fn build_core() -> OrchestrationCore {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();
    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockBoundaryProvider::new(vec![
            "Hello!".into(),
            "How can I help?".into(),
            "Sure thing.".into(),
            "Done.".into(),
        ])));
    }
    core
}

// ── Test 1: continue_last pure continue (empty text) ──────

#[tokio::test]
async fn test_continue_last_empty_text_pure_continue() {
    let core = build_core().await;
    let sid = "boundary-continue-last";

    // First turn: normal prompt.
    let r1 = core
        .execute_prompt(sid, "Hi", default_options())
        .await
        .unwrap();
    assert_eq!(r1.text, "Hello!");

    // Second turn: continue_last + empty text — must NOT add a user message,
    // must still call the LLM (which returns the next response).
    let opts = PromptExecutionOptions {
        model: Some("mock-boundary:mock-model".to_string()),
        continue_last: true,
        ..Default::default()
    };
    let r2 = core.execute_prompt(sid, "", opts).await.unwrap();
    assert_eq!(r2.text, "How can I help?");

    // The session must have exactly 3 messages: user "Hi", assistant "Hello!", assistant "How can I help?"
    // (no empty user message inserted by continue_last).
    let detail = core.get_session(sid).await.unwrap();
    assert_eq!(
        detail.messages.len(),
        3,
        "Expected 3 messages (user + assistant + assistant), got {}: {:?}",
        detail.messages.len(),
        detail.messages
    );
}

// ── Test 2: tool result visibility after unified authority ─

#[tokio::test]
async fn test_tool_result_visible_in_session() {
    let core = build_core().await;
    let sid = "boundary-tool-vis";

    // Seed the session with a user message and a tool result via the
    // shared SessionManager (simulating what happens in a tool loop).
    {
        let mut sessions = core.sessions().lock().await;
        let s = sessions.ensure_session(sid);
        s.add_user_message("run tool");
        s.add_tool_result("tc-1", "tool output", false);
    }

    // list_sessions should see the session.
    let sessions = core.list_sessions().await.unwrap();
    assert!(
        sessions.iter().any(|s| s.id == sid),
        "Session not visible in list"
    );

    // get_session should see the messages.
    let detail = core.get_session(sid).await.unwrap();
    assert_eq!(detail.messages.len(), 2, "Expected 2 messages");
    assert_eq!(detail.messages[0].role, "User");
    assert!(
        detail.messages[1].role.contains("Tool") || detail.messages[1].role == "Tool",
        "Second message should be Tool role"
    );
}

// ── Test 3: concurrent requests on same session, history consistency ─

#[tokio::test]
async fn test_concurrent_same_session_history_consistency() {
    let core = Arc::new(build_core().await);
    let sid = "boundary-concurrent-consistency";

    // Fire 3 concurrent prompts on the same session.
    let mut handles = Vec::new();
    for i in 0..3 {
        let core = Arc::clone(&core);
        let sid = sid.to_string();
        let opts = default_options();
        handles.push(tokio::spawn(async move {
            core.execute_prompt(&sid, &format!("msg-{}", i), opts).await
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }

    // All should succeed.
    for (i, r) in results.iter().enumerate() {
        assert!(r.is_ok(), "Request {} failed: {:?}", i, r.as_ref().err());
    }

    // After all complete, the session must have a consistent message count.
    // 3 user messages + 3 assistant responses = 6 messages.
    let detail = core.get_session(sid).await.unwrap();
    assert_eq!(
        detail.messages.len(),
        6,
        "Expected 6 messages (3 user + 3 assistant), got {}",
        detail.messages.len()
    );
}

// ── Test 4: session state consistency across rounds (multi-turn) ─

#[tokio::test]
async fn test_session_state_consistent_across_rounds() {
    let core = build_core().await;
    let sid = "boundary-consistency-rounds";

    // Round 1
    let _ = core
        .execute_prompt(sid, "first", default_options())
        .await
        .unwrap();
    let d1 = core.get_session(sid).await.unwrap();
    assert_eq!(d1.messages.len(), 2); // user + assistant

    // Round 2
    let _ = core
        .execute_prompt(sid, "second", default_options())
        .await
        .unwrap();
    let d2 = core.get_session(sid).await.unwrap();
    assert_eq!(d2.messages.len(), 4); // +2

    // Round 3
    let _ = core
        .execute_prompt(sid, "third", default_options())
        .await
        .unwrap();
    let d3 = core.get_session(sid).await.unwrap();
    assert_eq!(d3.messages.len(), 6);

    // All user messages must appear in order.
    let user_texts: Vec<String> = d3
        .messages
        .iter()
        .filter(|m| m.role == "User")
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(user_texts, vec!["first", "second", "third"]);
}

// ── Test 5: provider hot-reload — registration visible without restart ─

#[tokio::test]
async fn test_provider_hot_reload_visible_to_prompt() {
    let config = agendao_config::Config::default();
    let config_store = Arc::new(agendao_config::ConfigStore::new(config.clone()));
    let sessions = Arc::new(tokio::sync::Mutex::new(
        agendao_session_core::SessionManager::new(),
    ));
    let providers = Arc::new(tokio::sync::RwLock::new(
        agendao_provider::ProviderRegistry::new(),
    ));
    let tools = Arc::new(tokio::sync::RwLock::new(agendao_tool::ToolRegistry::new()));

    let core = agendao_orchestrator::OrchestrationCore::<
        agendao_session_core::SessionManager,
    >::new_with_shared_authorities(
        Arc::clone(&config_store),
        Arc::clone(&sessions),
        Arc::clone(&providers),
        Arc::clone(&tools),
    );

    // Before registration — prompt must fail (provider not found).
    let opts = PromptExecutionOptions {
        model: Some("mock-boundary:mock-model".to_string()),
        ..Default::default()
    };
    let result = core
        .execute_prompt("test-hot-reload", "ping", opts.clone())
        .await;
    assert!(
        result.is_err(),
        "Expected error before provider registration"
    );

    // Register provider via the shared Arc.
    {
        let mut p = providers.write().await;
        p.register_arc(Arc::new(MockBoundaryProvider::new(vec![
            "Hot-reload works!".into(),
        ])));
    }

    // After registration — prompt must succeed without restart.
    let result = core.execute_prompt("test-hot-reload", "ping", opts).await;
    assert!(
        result.is_ok(),
        "Expected success after hot-reload, got: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().text, "Hot-reload works!");
}

// ── Message source metadata round-trip ────────────────────

#[tokio::test]
async fn test_user_message_carries_canonical_source_metadata() {
    let core = build_core().await;
    let sid = "boundary-source-metadata";

    let opts = PromptExecutionOptions {
        model: Some("mock-boundary:mock-model".to_string()),
        source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
        source_surface: Some(agendao_types::MessageSourceSurface::Tui),
        ..Default::default()
    };
    core.execute_prompt(sid, "source test", opts).await.unwrap();

    // Read back via get_session — must find the session.
    let _detail = core.get_session(sid).await.unwrap();

    // Verify metadata round-trips through SessionRecord.
    let sessions = core.sessions().lock().await;
    let s = sessions.get(sid).unwrap();
    let record = s.record();
    let first_msg = &record.messages[0];
    let origin = agendao_types::message_source_origin(&first_msg.metadata);
    let surface = agendao_types::message_source_surface(&first_msg.metadata);
    assert_eq!(origin, Some(agendao_types::MessageSourceOrigin::Operator));
    assert_eq!(surface, Some(agendao_types::MessageSourceSurface::Tui));

    // Admission/authority derived from origin.
    let admission = agendao_types::message_admission_context(&first_msg.metadata);
    let authority = agendao_types::message_authority_class(&first_msg.metadata);
    assert_eq!(
        admission,
        Some(agendao_types::MessageAdmissionContext::Authenticated)
    );
    assert_eq!(authority, Some(agendao_types::MessageAuthorityClass::User));
}

// ── Direct Transport production-path consistency ──────────
// Tests use OrchestrationCore directly (available in this test crate).
// DirectTransport<agendao_session::SessionManager> wiring is tested at the
// agendao-client crate level (test_transport_selector_fallback.rs).

#[tokio::test]
async fn test_unified_authority_multi_turn_consistency_direct() {
    // Simulates TUI Direct mode: OrchestrationCore with unified session authority.
    let core = build_core().await;
    let sid = "direct-authority-multi-turn";

    let r1 = core
        .execute_prompt(sid, "hello", default_options())
        .await
        .unwrap();
    assert_eq!(r1.text, "Hello!");

    let r2 = core
        .execute_prompt(sid, "again", default_options())
        .await
        .unwrap();
    assert_eq!(r2.text, "How can I help?");

    // Verify shared session visibility.
    let detail = core.get_session(sid).await.unwrap();
    assert!(detail.messages.len() >= 4);
}

#[tokio::test]
async fn test_unified_authority_continue_last_direct() {
    let core = build_core().await;
    let sid = "direct-authority-continue";

    core.execute_prompt(sid, "hi", default_options())
        .await
        .unwrap();

    let opts = PromptExecutionOptions {
        model: Some("mock-boundary:mock-model".to_string()),
        continue_last: true,
        ..Default::default()
    };
    let r2 = core.execute_prompt(sid, "", opts).await.unwrap();
    assert_eq!(r2.text, "How can I help?");

    // No empty user message inserted by continue_last.
    let detail = core.get_session(sid).await.unwrap();
    assert_eq!(
        detail.messages.len(),
        3,
        "Expected 1 user + 2 asst, got {}",
        detail.messages.len()
    );
}
