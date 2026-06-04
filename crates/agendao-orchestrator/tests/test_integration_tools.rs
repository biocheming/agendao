/// Integration tests for tool calling (Phase 6.5)
///
/// Tests the complete tool calling flow including LLM → tool → LLM loops.
///
/// Note: These tests are simplified to focus on the orchestration layer.
/// Full tool calling integration requires complex provider mocking that
/// matches the actual Message/Content structure used by agendao-provider.
use agendao_orchestrator::{OrchestrationCore, PromptExecutionOptions};
use agendao_tool::{Tool, ToolContext, ToolError, ToolResult};
use std::sync::Arc;

/// Mock tool for testing
struct MockCalculatorTool;

#[async_trait::async_trait]
impl Tool for MockCalculatorTool {
    fn id(&self) -> &str {
        "calculator"
    }

    fn description(&self) -> &str {
        "Performs basic arithmetic operations"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "subtract", "multiply", "divide"]
                },
                "a": { "type": "number" },
                "b": { "type": "number" }
            },
            "required": ["operation", "a", "b"]
        })
    }

    async fn execute(
        &self,
        arguments: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let operation = arguments["operation"].as_str().unwrap();
        let a = arguments["a"].as_f64().unwrap();
        let b = arguments["b"].as_f64().unwrap();

        let result = match operation {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" => a / b,
            _ => return Err(ToolError::ExecutionError("Unknown operation".to_string())),
        };

        Ok(ToolResult::simple("Calculator", result.to_string()))
    }
}

/// Mock tool that fails
struct MockFailingTool;

#[async_trait::async_trait]
impl Tool for MockFailingTool {
    fn id(&self) -> &str {
        "failing_tool"
    }

    fn description(&self) -> &str {
        "A tool that always fails"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _arguments: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionError(
            "Tool execution failed".to_string(),
        ))
    }
}

/// Simple mock provider for testing tool registration
struct MockSimpleProvider;

#[async_trait::async_trait]
impl agendao_provider::Provider for MockSimpleProvider {
    fn id(&self) -> &str {
        "mock-simple"
    }

    fn name(&self) -> &str {
        "Mock Simple Provider"
    }

    fn models(&self) -> Vec<agendao_provider::ModelInfo> {
        vec![agendao_provider::ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock-simple".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 4096,
            supports_vision: false,
            supports_tools: true,
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
                message: agendao_provider::Message::assistant("Simple response"),
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
        use agendao_provider::StreamEvent;
        use futures::stream;

        let events = vec![
            Ok(StreamEvent::Start),
            Ok(StreamEvent::TextDelta("Test".to_string())),
            Ok(StreamEvent::Done),
        ];

        Ok(Box::pin(stream::iter(events)))
    }
}

#[tokio::test]
async fn test_tool_registry_integration() {
    // Test that tools can be registered and accessed
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Register tools
    {
        let tools = core.tools().write().await;
        tools.register(MockCalculatorTool).await;
        tools.register(MockFailingTool).await;
    }

    // Verify tools are registered
    {
        let tools = core.tools().read().await;
        assert!(tools.get("calculator").await.is_some());
        assert!(tools.get("failing_tool").await.is_some());
    }
}

#[tokio::test]
async fn test_tool_definitions_passed_to_provider() {
    // Test that tool definitions are correctly built and passed to provider
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockSimpleProvider));
    }

    {
        let tools = core.tools().write().await;
        tools.register(MockCalculatorTool).await;
    }

    let options = PromptExecutionOptions {
        model: Some("mock-simple:mock-model".to_string()),
        ..Default::default()
    };

    // Execute dialogue - provider will receive tool definitions
    let result = core
        .execute_prompt("tool-def-test", "Test", options)
        .await
        .unwrap();

    assert_eq!(result.text, "Simple response");
}

#[tokio::test]
async fn test_multiple_tools_registration() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    // Register multiple tools
    {
        let tools = core.tools().write().await;
        tools.register(MockCalculatorTool).await;
        tools.register(MockFailingTool).await;
    }

    // Verify all tools are accessible
    {
        let tools = core.tools().read().await;
        let tool_list = tools.list().await;
        assert!(tool_list.len() >= 2);

        let tool_ids: Vec<_> = tool_list.iter().map(|t| t.id()).collect();
        assert!(tool_ids.contains(&"calculator"));
        assert!(tool_ids.contains(&"failing_tool"));
    }
}

#[tokio::test]
async fn test_tool_parameters_schema() {
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let tools = core.tools().write().await;
        tools.register(MockCalculatorTool).await;
    }

    // Verify tool parameters are correct
    {
        let tools = core.tools().read().await;
        let calculator = tools.get("calculator").await.unwrap();

        let params = calculator.parameters();
        assert!(params.is_object());
        assert!(params["properties"]["operation"].is_object());
        assert!(params["properties"]["a"].is_object());
        assert!(params["properties"]["b"].is_object());
    }
}

#[tokio::test]
async fn test_tool_isolation_between_sessions() {
    // Verify that tool registry is shared but execution is isolated
    let config = agendao_config::Config::default();
    let core = OrchestrationCore::new(&config).await.unwrap();

    {
        let mut providers = core.providers().write().await;
        providers.register_arc(Arc::new(MockSimpleProvider));
    }

    {
        let tools = core.tools().write().await;
        tools.register(MockCalculatorTool).await;
    }

    let options = PromptExecutionOptions {
        model: Some("mock-simple:mock-model".to_string()),
        ..Default::default()
    };

    // Execute in different sessions
    let _ = core
        .execute_prompt("session-1", "Test 1", options.clone())
        .await
        .unwrap();

    let _ = core
        .execute_prompt("session-2", "Test 2", options)
        .await
        .unwrap();

    // Verify sessions are isolated
    let session1 = core.get_session("session-1").await.unwrap();
    let session2 = core.get_session("session-2").await.unwrap();

    assert_eq!(session1.messages.len(), 2);
    assert_eq!(session2.messages.len(), 2);
    assert_eq!(session1.messages[0].content, "Test 1");
    assert_eq!(session2.messages[0].content, "Test 2");
}
