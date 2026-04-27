use super::skill_reflection::{
    augment_system_prompt_with_skill_reflection, extract_tool_call_history,
    prepare_skill_reflection, update_skill_reflection_metadata, SkillReflectionData,
    SkillUsageSummary, ToolCallSummary,
};
use super::*;
use crate::message::MessagePart;
use crate::SessionMessage;
use async_trait::async_trait;
use futures::stream;
use rocode_execution_types::CompiledExecutionRequest;
use rocode_provider::{
    ChatRequest, ChatResponse, ModelInfo, ProviderError, StreamEvent, StreamResult, StreamUsage,
};
use rocode_skill::RuntimeInstructionSource;
use rocode_tool::{Tool, ToolContext, ToolError, ToolResult};
use std::fs;
use std::sync::Mutex as StdMutex;
use tempfile::tempdir;

struct StaticModelProvider {
    model: Option<ModelInfo>,
}

impl StaticModelProvider {
    fn with_model(model_id: &str, context_window: u64, max_output_tokens: u64) -> Self {
        Self {
            model: Some(ModelInfo {
                id: model_id.to_string(),
                name: "Static Model".to_string(),
                provider: "mock".to_string(),
                context_window,
                max_input_tokens: None,
                max_output_tokens,
                supports_vision: false,
                supports_tools: false,
                cost_per_million_input: 0.0,
                cost_per_million_output: 0.0,
            }),
        }
    }
}

#[async_trait]
impl Provider for StaticModelProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.model.clone().into_iter().collect()
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        self.model.as_ref().filter(|model| model.id == id)
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::empty()))
    }
}

struct ScriptedStreamProvider {
    model: ModelInfo,
    events: Vec<StreamEvent>,
}

fn write_methodology_skill(
    root: &std::path::Path,
    name: &str,
    template: rocode_skill::SkillMethodologyTemplate,
) {
    let skill_dir = root.join(".rocode/skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let body = rocode_skill::render_methodology_skill_body(name, &template).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test skill\n---\n\n{body}\n",),
    )
    .unwrap();
}

#[async_trait]
impl Provider for ScriptedStreamProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.model.clone()]
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        if self.model.id == id {
            Some(&self.model)
        } else {
            None
        }
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        Ok(Box::pin(stream::iter(
            self.events
                .clone()
                .into_iter()
                .map(Result::<StreamEvent, ProviderError>::Ok),
        )))
    }
}

struct MultiTurnScriptedProvider {
    model: ModelInfo,
    turns: Arc<StdMutex<std::collections::VecDeque<Vec<StreamEvent>>>>,
    request_count: Arc<StdMutex<usize>>,
}

impl MultiTurnScriptedProvider {
    fn new(model: ModelInfo, turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            model,
            turns: Arc::new(StdMutex::new(turns.into())),
            request_count: Arc::new(StdMutex::new(0)),
        }
    }
}

#[async_trait]
impl Provider for MultiTurnScriptedProvider {
    fn id(&self) -> &str {
        "mock"
    }

    fn name(&self) -> &str {
        "Mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.model.clone()]
    }

    fn get_model(&self, id: &str) -> Option<&ModelInfo> {
        if self.model.id == id {
            Some(&self.model)
        } else {
            None
        }
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::InvalidRequest(
            "chat() not used in this test".to_string(),
        ))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<StreamResult, ProviderError> {
        {
            let mut count = self
                .request_count
                .lock()
                .expect("request_count lock should not poison");
            *count += 1;
        }

        let events = self
            .turns
            .lock()
            .expect("turns lock should not poison")
            .pop_front()
            .ok_or_else(|| {
                ProviderError::InvalidRequest(
                    "no scripted response left for chat_stream".to_string(),
                )
            })?;

        Ok(Box::pin(stream::iter(
            events
                .into_iter()
                .map(Result::<StreamEvent, ProviderError>::Ok),
        )))
    }
}

struct NoArgEchoTool;

#[async_trait]
impl Tool for NoArgEchoTool {
    fn id(&self) -> &str {
        "noarg_echo"
    }

    fn description(&self) -> &str {
        "Echoes input for tests"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::simple("NoArg Echo", args.to_string()))
    }
}

struct AlwaysInvalidArgsTool;

#[async_trait]
impl Tool for AlwaysInvalidArgsTool {
    fn id(&self) -> &str {
        "needs_path"
    }

    fn description(&self) -> &str {
        "Fails validation for tests"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string" }
            },
            "required": ["filePath"]
        })
    }

    fn validate(&self, _args: &serde_json::Value) -> Result<(), ToolError> {
        Err(ToolError::InvalidArguments(
            "filePath is required".to_string(),
        ))
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionError(
            "validate should prevent execute".to_string(),
        ))
    }
}
#[test]
fn insert_reminders_adds_plan_prompt_for_plan_agent() {
    let messages = vec![SessionMessage::user("ses_test", "plan this")];
    let output = insert_reminders(&messages, "plan", false);
    let last = output.last().unwrap();
    let injected = last
        .parts
        .iter()
        .filter_map(|p| match &p.part_type {
            PartType::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(injected.contains("You are in PLAN mode"));
}

#[test]
fn insert_reminders_adds_build_switch_after_plan() {
    let mut user = SessionMessage::user("ses_test", "execute this");
    user.metadata
        .insert("agent".to_string(), serde_json::json!("plan"));
    let output = insert_reminders(&[user], "build", true);
    let last = output.last().unwrap();
    let injected = last
        .parts
        .iter()
        .filter_map(|p| match &p.part_type {
            PartType::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(injected.contains("The user has approved your plan"));
}

#[tokio::test]
async fn prompt_with_update_hook_emits_incremental_snapshots() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let provider = Arc::new(ScriptedStreamProvider {
        model: ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "mock".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 1024,
            supports_vision: false,
            supports_tools: false,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
        },
        events: vec![
            StreamEvent::Start,
            StreamEvent::TextDelta("Hel".to_string()),
            StreamEvent::TextDelta("lo".to_string()),
            StreamEvent::FinishStep {
                finish_reason: Some("stop".to_string()),
                usage: StreamUsage {
                    prompt_tokens: 3,
                    completion_tokens: 2,
                    ..Default::default()
                },
                provider_metadata: None,
            },
            StreamEvent::Done,
        ],
    });

    let snapshots = Arc::new(StdMutex::new(Vec::<Session>::new()));
    let snapshot_sink = snapshots.clone();
    let hook: SessionUpdateHook = Arc::new(move |snapshot| {
        snapshot_sink
            .lock()
            .expect("snapshot lock should not poison")
            .push(snapshot.clone());
    });

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: Some(ModelRef {
            provider_id: "mock".to_string(),
            model_id: "test-model".to_string(),
        }),
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "Say hello".to_string(),
        }],
        tools: None,
    };

    prompt
        .prompt_with_update_hook(
            input,
            &mut session,
            PromptRequestContext {
                provider,
                system_prompt: None,
                memory_prefetch: None,
                tools: Vec::new(),
                compiled_request: CompiledExecutionRequest::default(),
                hooks: PromptHooks {
                    update_hook: Some(hook),
                    ..Default::default()
                },
            },
        )
        .await
        .expect("prompt_with_update_hook should succeed");

    let snapshots_guard = snapshots.lock().expect("snapshot lock should not poison");
    assert!(snapshots_guard.len() >= 3);
    let saw_partial = snapshots_guard.iter().any(|snap| {
        snap.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, MessageRole::Assistant))
            .map(|m| m.get_text() == "Hel")
            .unwrap_or(false)
    });
    assert!(
        saw_partial,
        "expected at least one streamed partial assistant snapshot"
    );
    drop(snapshots_guard);

    let final_text = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .unwrap_or_default();
    assert_eq!(final_text, "Hello");
}

#[tokio::test]
async fn prompt_continues_after_tool_calls_without_finish_step_reason() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let temp_dir = tempfile::tempdir().expect("tempdir should create");
    let file_path = temp_dir.path().join("sample.txt");
    tokio::fs::write(&file_path, "alpha\nbeta")
        .await
        .expect("file should write");
    let file_path = file_path.to_string_lossy().to_string();

    let scripted = MultiTurnScriptedProvider::new(
        ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "mock".to_string(),
            context_window: 8192,
            max_input_tokens: None,
            max_output_tokens: 1024,
            supports_vision: false,
            supports_tools: true,
            cost_per_million_input: 0.0,
            cost_per_million_output: 0.0,
        },
        vec![
            vec![
                StreamEvent::Start,
                StreamEvent::ToolCallStart {
                    id: "tool-call-0".to_string(),
                    name: "read".to_string(),
                },
                StreamEvent::ToolCallEnd {
                    id: "tool-call-0".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({ "file_path": file_path }),
                },
                StreamEvent::Done,
            ],
            vec![
                StreamEvent::Start,
                StreamEvent::TextDelta("Read complete".to_string()),
                StreamEvent::FinishStep {
                    finish_reason: Some("stop".to_string()),
                    usage: StreamUsage::default(),
                    provider_metadata: None,
                },
                StreamEvent::Done,
            ],
        ],
    );
    let request_count = scripted.request_count.clone();
    let provider: Arc<dyn Provider> = Arc::new(scripted);

    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: Some(ModelRef {
            provider_id: "mock".to_string(),
            model_id: "test-model".to_string(),
        }),
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        parts: vec![PartInput::Text {
            text: "Read the file and summarize".to_string(),
        }],
        tools: None,
    };

    prompt
        .prompt_with_update_hook(
            input,
            &mut session,
            PromptRequestContext {
                provider,
                system_prompt: None,
                memory_prefetch: None,
                tools: Vec::new(),
                compiled_request: CompiledExecutionRequest::default(),
                hooks: PromptHooks::default(),
            },
        )
        .await
        .expect("prompt_with_update_hook should succeed");

    let request_count = *request_count
        .lock()
        .expect("request_count lock should not poison");
    assert_eq!(request_count, 2, "expected a second model round");

    let final_text = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(SessionMessage::get_text)
        .unwrap_or_default();
    assert_eq!(final_text, "Read complete");
}

#[tokio::test]
async fn create_user_message_persists_pending_subtask_payload() {
    let prompt = SessionPrompt::default();
    let mut session = Session::new("proj", ".");
    let input = PromptInput {
        session_id: session.id.clone(),
        message_id: None,
        model: None,
        agent: None,
        no_reply: false,
        system: None,
        variant: None,
        tools: None,
        parts: vec![PartInput::Subtask {
            prompt: "Inspect codegen path".to_string(),
            description: Some("Inspect codegen".to_string()),
            agent: "explore".to_string(),
        }],
    };

    prompt
        .create_user_message(&input, &mut session)
        .await
        .expect("create_user_message should succeed");

    let msg = session.messages.last().expect("user message should exist");
    let pending = msg
        .metadata
        .get("pending_subtasks")
        .and_then(|v| v.as_array())
        .expect("pending_subtasks metadata should exist");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].get("agent").and_then(|v| v.as_str()),
        Some("explore")
    );
    assert_eq!(
        pending[0].get("prompt").and_then(|v| v.as_str()),
        Some("Inspect codegen path")
    );
    assert!(msg.parts.iter().any(|p| match &p.part_type {
        PartType::Subtask { status, .. } => status == "pending",
        _ => false,
    }));
}
#[test]
fn shell_exec_uses_zsh_login_invocation() {
    let invocation = resolve_shell_invocation(Some("/bin/zsh"), "echo hello");
    assert_eq!(invocation.program, "/bin/zsh");
    assert_eq!(invocation.args[0], "-c");
    assert_eq!(invocation.args[1], "-l");
    assert!(invocation.args[2].contains(".zshenv"));
    assert!(invocation.args[2].contains("eval"));
}

#[test]
fn shell_exec_uses_bash_login_invocation() {
    let invocation = resolve_shell_invocation(Some("/bin/bash"), "echo hello");
    assert_eq!(invocation.program, "/bin/bash");
    assert_eq!(invocation.args[0], "-c");
    assert_eq!(invocation.args[1], "-l");
    assert!(invocation.args[2].contains("shopt -s expand_aliases"));
    assert!(invocation.args[2].contains(".bashrc"));
}

#[tokio::test]
async fn resolve_tools_with_mcp_registry_includes_mcp_tools() {
    let tool_registry = rocode_tool::create_default_registry().await;
    let mcp_registry = rocode_mcp::McpToolRegistry::new();
    mcp_registry
        .register(rocode_mcp::McpTool::new(
            "github",
            "search",
            Some("Search GitHub".to_string()),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        ))
        .await;

    let tools = resolve_tools_with_mcp_registry(&tool_registry, Some(&mcp_registry)).await;
    assert!(tools.iter().any(|t| t.name == "github_search"));
}

#[tokio::test]
async fn execute_tool_calls_ignores_empty_tool_name() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run tools"));

    let mut assistant = SessionMessage::assistant(sid);
    assistant.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::ToolCall {
            id: "call_empty".to_string(),
            name: " ".to_string(),
            input: serde_json::json!({}),
            status: crate::ToolCallStatus::Running,
            raw: None,
            state: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.add_tool_call("call_ok", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let result_ids: Vec<&str> = tool_msg
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(result_ids, vec!["call_ok"]);
}

#[tokio::test]
async fn execute_tool_calls_runs_no_arg_tool() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run noarg"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.add_tool_call("call_noarg", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");

    let (content, is_error) = tool_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } if tool_call_id == "call_noarg" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("noarg result should exist");

    assert!(!is_error);
    assert_eq!(content, "{}");
}

#[tokio::test]
async fn execute_tool_calls_routes_invalid_arguments_to_invalid_tool() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(AlwaysInvalidArgsTool).await;
    tool_registry
        .register(rocode_tool::invalid::InvalidTool)
        .await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run invalid"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.add_tool_call("call_invalid", "needs_path", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let assistant_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .expect("assistant message should exist");
    let tool_call = assistant_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolCall {
                id,
                name,
                input,
                status,
                ..
            } if id == "call_invalid" => Some((name, input, status)),
            _ => None,
        })
        .expect("tool call should exist");
    assert_eq!(tool_call.0, "invalid");
    assert_eq!(
        tool_call.1.get("tool").and_then(|v| v.as_str()),
        Some("needs_path")
    );
    assert!(tool_call.1.get("receivedArgs").is_none());
    assert!(matches!(tool_call.2, crate::ToolCallStatus::Completed));

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let (content, is_error) = tool_msg
        .parts
        .iter()
        .find_map(|part| match &part.part_type {
            PartType::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } if tool_call_id == "call_invalid" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("invalid fallback result should exist");
    assert!(!is_error);
    assert!(content.contains("The arguments provided to the tool are invalid:"));
}

#[tokio::test]
async fn execute_tool_calls_only_runs_running_tool_calls() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();
    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "run running only"));
    let mut assistant = SessionMessage::assistant(sid);
    assistant.parts.push(crate::MessagePart {
        id: format!("prt_{}", uuid::Uuid::new_v4()),
        part_type: PartType::ToolCall {
            id: "call_pending".to_string(),
            name: "noarg_echo".to_string(),
            input: serde_json::json!({}),
            status: crate::ToolCallStatus::Pending,
            raw: Some("{".to_string()),
            state: Some(crate::ToolState::Pending {
                input: serde_json::json!({}),
                raw: "{".to_string(),
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.add_tool_call("call_running", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msg = session
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Tool))
        .expect("tool message should exist");
    let result_ids: Vec<&str> = tool_msg
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            PartType::ToolResult { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(result_ids, vec!["call_running"]);
}

#[tokio::test]
async fn execute_tool_calls_reused_call_id_in_new_turn_still_executes() {
    let tool_registry = Arc::new(rocode_tool::ToolRegistry::new());
    tool_registry.register(NoArgEchoTool).await;

    let mut session = Session::new("proj", ".");
    let sid = session.id.clone();

    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "turn one"));
    let mut assistant_1 = SessionMessage::assistant(sid.clone());
    assistant_1.add_tool_call("tool-call-0", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant_1);
    let mut tool_msg_1 = SessionMessage::tool(sid.clone());
    tool_msg_1.add_tool_result("tool-call-0", "{}", false);
    session.messages_mut().push(tool_msg_1);

    session
        .messages_mut()
        .push(SessionMessage::user(sid.clone(), "turn two"));
    let mut assistant_2 = SessionMessage::assistant(sid);
    assistant_2.add_tool_call("tool-call-0", "noarg_echo", serde_json::json!({}));
    session.messages_mut().push(assistant_2);

    let provider: Arc<dyn Provider> =
        Arc::new(StaticModelProvider::with_model("test-model", 8192, 1024));
    let ctx = ToolContext::new(session.id.clone(), "msg_test".to_string(), ".".to_string());

    SessionPrompt::execute_tool_calls(
        &mut session,
        tool_registry,
        ctx,
        provider,
        "mock",
        "test-model",
    )
    .await
    .expect("execute_tool_calls should succeed");

    let tool_msgs: Vec<&SessionMessage> = session
        .messages
        .iter()
        .filter(|m| matches!(m.role, MessageRole::Tool))
        .collect();
    assert!(
        tool_msgs.len() >= 2,
        "expected a second tool message for the new turn"
    );

    let last_tool_msg = tool_msgs.last().expect("latest tool message should exist");
    let second_turn_result_count = last_tool_msg
        .parts
        .iter()
        .filter(|part| {
            matches!(
                &part.part_type,
                PartType::ToolResult { tool_call_id, .. } if tool_call_id == "tool-call-0"
            )
        })
        .count();
    assert_eq!(second_turn_result_count, 1);
}

// ── PartInput serde round-trip tests ──

#[test]
fn part_input_text_round_trip() {
    let part = PartInput::Text {
        text: "hello".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "text");
    assert_eq!(json["text"], "hello");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Text { text } if text == "hello"));
}

#[test]
fn part_input_file_round_trip() {
    let part = PartInput::File {
        url: "file:///tmp/test.rs".to_string(),
        filename: Some("test.rs".to_string()),
        mime: Some("text/plain".to_string()),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "file");
    assert_eq!(json["url"], "file:///tmp/test.rs");
    assert_eq!(json["filename"], "test.rs");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::File { url, .. } if url == "file:///tmp/test.rs"));
}

#[test]
fn part_input_agent_round_trip() {
    let part = PartInput::Agent {
        name: "explore".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "agent");
    assert_eq!(json["name"], "explore");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Agent { name } if name == "explore"));
}

#[test]
fn part_input_subtask_round_trip() {
    let part = PartInput::Subtask {
        prompt: "do stuff".to_string(),
        description: Some("stuff".to_string()),
        agent: "build".to_string(),
    };
    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["type"], "subtask");
    assert_eq!(json["agent"], "build");

    let back: PartInput = serde_json::from_value(json).unwrap();
    assert!(matches!(back, PartInput::Subtask { agent, .. } if agent == "build"));
}

#[test]
fn part_input_try_from_value() {
    let val = serde_json::json!({"type": "text", "text": "hi"});
    let part = PartInput::try_from(val).unwrap();
    assert!(matches!(part, PartInput::Text { text } if text == "hi"));
}

#[test]
fn part_input_try_from_invalid_value() {
    let val = serde_json::json!({"type": "unknown", "data": 42});
    assert!(PartInput::try_from(val).is_err());
}

#[test]
fn part_input_parse_array_mixed() {
    let arr = serde_json::json!([
        {"type": "text", "text": "hello"},
        {"type": "agent", "name": "explore"},
        {"type": "bogus"},
        {"type": "file", "url": "file:///x", "filename": "x", "mime": "text/plain"}
    ]);
    let parts = PartInput::parse_array(&arr);
    assert_eq!(parts.len(), 3); // bogus entry skipped
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "hello"));
    assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
    assert!(matches!(&parts[2], PartInput::File { url, .. } if url == "file:///x"));
}

#[test]
fn part_input_parse_array_non_array() {
    let val = serde_json::json!("not an array");
    assert!(PartInput::parse_array(&val).is_empty());
}

#[test]
fn part_input_file_skips_none_fields_in_json() {
    let part = PartInput::File {
        url: "file:///tmp/x".to_string(),
        filename: None,
        mime: None,
    };
    let json = serde_json::to_value(&part).unwrap();
    assert!(json.get("filename").is_none());
    assert!(json.get("mime").is_none());
}

// ── resolve_prompt_parts tests ──

#[tokio::test]
async fn resolve_prompt_parts_plain_text() {
    let parts = resolve_prompt_parts("just plain text", std::path::Path::new("/tmp"), &[]).await;
    assert_eq!(parts.len(), 1);
    assert!(matches!(&parts[0], PartInput::Text { text } if text == "just plain text"));
}

#[tokio::test]
async fn resolve_prompt_parts_agent_fallback() {
    // @explore doesn't exist as a file, but is a known agent
    let agents = vec!["explore".to_string(), "build".to_string()];
    let parts = resolve_prompt_parts(
        "check @explore for details",
        std::path::Path::new("/tmp"),
        &agents,
    )
    .await;
    assert_eq!(parts.len(), 2);
    assert!(matches!(&parts[0], PartInput::Text { .. }));
    assert!(matches!(&parts[1], PartInput::Agent { name } if name == "explore"));
}

#[tokio::test]
async fn resolve_prompt_parts_deduplicates() {
    let parts = resolve_prompt_parts(
        "see @explore and @explore again",
        std::path::Path::new("/tmp"),
        &["explore".to_string()],
    )
    .await;
    // text + one agent (deduplicated)
    assert_eq!(parts.len(), 2);
}

#[tokio::test]
async fn resolve_prompt_parts_real_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.rs");
    tokio::fs::write(&file, "fn main() {}").await.unwrap();

    let parts = resolve_prompt_parts("look at @test.rs", dir.path(), &[]).await;
    assert_eq!(parts.len(), 2);
    assert!(
        matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("text/plain"))
    );
}

#[tokio::test]
async fn resolve_prompt_parts_directory() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("src");
    tokio::fs::create_dir(&sub).await.unwrap();

    let parts = resolve_prompt_parts("look at @src", dir.path(), &[]).await;
    assert_eq!(parts.len(), 2);
    assert!(
        matches!(&parts[1], PartInput::File { mime, .. } if mime.as_deref() == Some("application/x-directory"))
    );
}

/// Regression test for the prompt loop early-exit bug:
/// When the assistant message has text + tool calls and finish="tool-calls",
/// the loop must NOT break at the top-of-loop check.
/// Previously, the check used `has_finish = !text.is_empty()` which caused
/// premature exit when models emit text before tool calls.
#[test]
fn early_exit_does_not_break_on_tool_calls_finish() {
    // Simulate: user message at index 0, assistant at index 1
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    // Assistant has text content (model explained before calling tools)
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "Let me read those files for you.".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    // finish_reason is "tool-calls" — loop should continue, not break
    assistant.finish = Some("tool-calls".to_string());

    let messages = vec![user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    // The early-exit check from the prompt loop
    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(
        !should_break,
        "early-exit must NOT trigger when finish='tool-calls'"
    );
}

#[test]
fn current_context_estimate_ignores_completed_scheduler_stage_metadata() {
    let mut assistant = SessionMessage::assistant("s1");
    assistant.metadata.insert(
        "scheduler_stage_status".to_string(),
        serde_json::json!("done"),
    );
    assistant.metadata.insert(
        "scheduler_stage_context_tokens".to_string(),
        serde_json::json!(1_388_907_u64),
    );
    assistant.metadata.insert(
        "scheduler_stage_prompt_tokens".to_string(),
        serde_json::json!(1_388_907_u64),
    );
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "done".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    let estimated = estimate_current_context_tokens(&[assistant]).unwrap();

    assert!(
        estimated < 1_000,
        "completed scheduler stage peak telemetry must not drive live context pressure: {estimated}"
    );
}

/// Verify that the early-exit check DOES break when finish is terminal
/// (e.g. "stop") and assistant is after the last user message.
#[test]
fn early_exit_breaks_on_terminal_finish() {
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "Here is my response.".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.finish = Some("stop".to_string());

    let messages = vec![user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(should_break, "early-exit MUST trigger when finish='stop'");
}

/// Verify that the early-exit check does NOT break when finish is None
/// (assistant message still streaming / no FinishStep received yet).
#[test]
fn early_exit_does_not_break_when_finish_is_none() {
    let user = SessionMessage::user("s1", "hello");
    let mut assistant = SessionMessage::assistant("s1");
    assistant.parts.push(MessagePart {
        id: "prt_text".to_string(),
        part_type: PartType::Text {
            text: "partial response...".to_string(),
            synthetic: None,
            ignored: None,
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    // finish is None — still streaming
    assistant.finish = None;

    let messages = vec![user, assistant];

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User))
        .unwrap();
    let last_assistant_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant));

    let should_break = if let Some(assistant_idx) = last_assistant_idx {
        let assistant = &messages[assistant_idx];
        let is_terminal = assistant
            .finish
            .as_deref()
            .is_some_and(|f| !matches!(f, "tool-calls" | "tool_calls" | "unknown"));
        is_terminal && last_user_idx < assistant_idx
    } else {
        false
    };

    assert!(
        !should_break,
        "early-exit must NOT trigger when finish is None"
    );
}

#[test]
fn chat_message_hook_not_triggered_on_user_message_creation() {
    let source = include_str!("mod.rs");
    let create_user_fn = source
        .find("async fn create_user_message")
        .expect("create_user_message should exist");
    let rest = &source[create_user_fn..];
    let next_method = rest[1..]
        .find("\n    async fn ")
        .or_else(|| rest[1..].find("\n    pub async fn "))
        .map(|offset| offset + 1)
        .unwrap_or(rest.len());
    let create_user_section = &rest[..next_method];
    assert!(
        !create_user_section.contains("HookEvent::ChatMessage"),
        "ChatMessage hook should not be in create_user_message"
    );
}

#[test]
fn runtime_skill_save_suggestion_skips_turns_that_are_only_complex() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("optimize this workflow");
    let assistant = session.add_assistant_message();
    assistant.finish = Some("stop".to_string());
    let tool = SessionMessage::tool(session.id.clone());
    session.messages_mut().push(tool);

    let tool_msg = session.messages_mut().last_mut().unwrap();
    for index in 0..3 {
        tool_msg.parts.push(MessagePart {
            id: format!("prt_tool_{index}"),
            part_type: PartType::ToolResult {
                tool_call_id: format!("call_{index}"),
                content: "ok".to_string(),
                is_error: false,
                title: None,
                metadata: None,
                attachments: None,
            },
            created_at: chrono::Utc::now(),
            message_id: None,
        });
    }

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    assert!(!session.messages.iter().any(|message| {
        matches!(message.role, MessageRole::Assistant)
            && message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())
                == Some("skill_save_suggestion")
    }));
}

#[test]
fn runtime_skill_save_suggestion_triggers_for_methodology_shaped_turns() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("fix the failing parser and verify it");

    let assistant = session.add_assistant_message();
    assistant.finish = Some("tool-calls".to_string());
    assistant.parts.push(MessagePart {
        id: "prt_tool_edit".to_string(),
        part_type: PartType::ToolCall {
            id: "call_edit".to_string(),
            name: "edit".to_string(),
            input: serde_json::json!({
                "file_path": "src/parser.rs",
                "old_string": "broken()",
                "new_string": "fixed()"
            }),
            raw: None,
            status: crate::ToolCallStatus::Completed,
            state: Some(crate::ToolState::Completed {
                input: serde_json::json!({
                    "file_path": "src/parser.rs",
                    "old_string": "broken()",
                    "new_string": "fixed()"
                }),
                output: "patched parser".to_string(),
                title: "Edited parser".to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::CompletedTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                    compacted: None,
                },
                attachments: None,
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });
    assistant.parts.push(MessagePart {
        id: "prt_tool_bash_failed".to_string(),
        part_type: PartType::ToolCall {
            id: "call_bash_failed".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({
                "command": "cargo test parser",
                "description": "Run parser tests"
            }),
            raw: None,
            status: crate::ToolCallStatus::Error,
            state: Some(crate::ToolState::Error {
                input: serde_json::json!({
                    "command": "cargo test parser",
                    "description": "Run parser tests"
                }),
                error: "parser test still failing".to_string(),
                metadata: None,
                time: crate::ErrorTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                },
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    let followup = session.add_assistant_message();
    followup.finish = Some("stop".to_string());
    followup.add_text("Patched the parser and verified the failure mode; this flow is reusable.");

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    let note = session
        .messages
        .iter()
        .find(|message| {
            message
                .metadata
                .get("runtime_hint")
                .and_then(|value| value.as_str())
                == Some("skill_save_suggestion")
        })
        .expect("runtime hint note should be appended");

    assert!(note.parts.iter().any(|part| {
        matches!(
            &part.part_type,
            PartType::Text { text, .. }
                if text.contains("reusable triggers, steps, validation, and boundaries")
        )
    }));
}

#[test]
fn extract_tool_call_history_from_session_parts() {
    let mut session = Session::new("proj", ".");
    let assistant = session.add_assistant_message();
    SessionPrompt::upsert_tool_call_part(
        assistant,
        "call_bash",
        Some("bash"),
        Some(serde_json::json!({"command": "cargo test"})),
        None,
        Some(crate::ToolCallStatus::Completed),
        None,
    );

    let mut tool_message = SessionMessage::tool(session.id.clone());
    SessionPrompt::push_tool_result_part(
        &mut tool_message,
        "call_bash".to_string(),
        "ok".to_string(),
        false,
        None,
        None,
        None,
    );
    session.messages_mut().push(tool_message);

    let history = extract_tool_call_history(&session);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].tool_name, "bash");
    assert_eq!(history[0].tool_input_summary, "command=cargo test");
    assert_eq!(history[0].tool_result_summary, "ok");
}

#[test]
fn prepare_skill_reflection_returns_none_without_runtime_skills() {
    let session = Session::new("proj", ".");
    assert!(prepare_skill_reflection(None, &session).is_none());
}

#[test]
fn prepare_skill_reflection_loads_methodology_from_runtime_skill_names() {
    let dir = tempdir().unwrap();
    write_methodology_skill(
        dir.path(),
        "runtime-check",
        rocode_skill::SkillMethodologyTemplate {
            when_to_use: vec!["Use when runtime checks should be repeated.".to_string()],
            when_not_to_use: vec!["Do not use for ad-hoc notes.".to_string()],
            prerequisites: vec![],
            core_steps: vec![rocode_skill::SkillMethodologyStep {
                title: "Check".to_string(),
                action: "Run the runtime check.".to_string(),
                outcome: Some("Runtime state is known.".to_string()),
                experienced_tools: vec!["cargo".to_string()],
            }],
            success_criteria: vec!["The runtime state is visible.".to_string()],
            validation: vec!["Repeat the check after changes.".to_string()],
            pitfalls: vec!["Do not patch production configs while checking.".to_string()],
            references: vec![],
        },
    );

    let mut session = Session::new("proj", dir.path().to_string_lossy().to_string());
    session.insert_metadata(
        "runtime_skill_instructions",
        serde_json::to_value(vec![RuntimeInstructionSource {
            path: dir.path().join("AGENTS.md"),
            content: r#"
1. For the harness protocol itself
   - target workspace skill: `runtime-check`
   - target path: `.rocode/skills/runtime-check/SKILL.md`
   - description: `Reusable runtime verification workflow.`
"#
            .to_string(),
        }])
        .unwrap(),
    );

    let reflection = prepare_skill_reflection(None, &session).expect("reflection should exist");
    assert_eq!(reflection.skills_used.len(), 1);
    assert_eq!(reflection.skills_used[0].name, "runtime-check");
    assert_eq!(
        reflection.skills_used[0]
            .methodology
            .as_ref()
            .expect("methodology should load")
            .core_steps[0]
            .experienced_tools,
        vec!["cargo".to_string()]
    );
}

#[test]
fn update_skill_reflection_metadata_removes_stale_value_when_none() {
    let mut session = Session::new("proj", ".");
    session.insert_metadata("skill_reflection", serde_json::json!({"stale": true}));

    update_skill_reflection_metadata(None, &mut session);
    assert!(!session.metadata.contains_key("skill_reflection"));
}

#[test]
fn augment_system_prompt_with_skill_reflection_consumes_metadata_once() {
    let mut session = Session::new("proj", ".");
    session.insert_metadata(
        "skill_reflection",
        serde_json::to_value(SkillReflectionData {
            skills_used: vec![SkillUsageSummary {
                name: "runtime-check".to_string(),
                methodology: None,
            }],
            tool_calls: vec![ToolCallSummary {
                tool_name: "bash".to_string(),
                tool_input_summary: "command=cargo test".to_string(),
                tool_result_summary: "ok".to_string(),
            }],
        })
        .unwrap(),
    );

    let first = augment_system_prompt_with_skill_reflection(&mut session, Some("BASE".to_string()))
        .expect("prompt should exist");
    assert!(first.contains("BASE"));
    assert!(first.contains("## Skill Usage Reflection"));
    assert!(!session.metadata.contains_key("skill_reflection"));

    let second =
        augment_system_prompt_with_skill_reflection(&mut session, Some("BASE".to_string()))
            .expect("base prompt should remain");
    assert_eq!(second, "BASE");
}

#[test]
fn tool_is_validation_accepts_generic_validation_signals() {
    assert!(tool_is_validation(
        "health_check",
        &serde_json::json!({ "operation": "status" })
    ));
    assert!(tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "kubectl apply -f deploy.yaml --dry-run=client"
        })
    ));
    assert!(tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "acme-cli verify dataset"
        })
    ));
}

#[test]
fn tool_is_validation_rejects_plain_output_commands() {
    assert!(!tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "echo verify deployment"
        })
    ));
    assert!(!tool_is_validation(
        "bash",
        &serde_json::json!({
            "command": "cat verify.txt"
        })
    ));
}

#[test]
fn runtime_skill_save_suggestion_skips_turns_that_already_used_skill_manage() {
    let mut session = Session::new("proj", ".");
    session.add_user_message("optimize this workflow");
    let assistant = session.add_assistant_message();
    assistant.finish = Some("stop".to_string());
    assistant.parts.push(MessagePart {
        id: "prt_tool_call".to_string(),
        part_type: PartType::ToolCall {
            id: "call_skill".to_string(),
            name: "skill_manage".to_string(),
            input: serde_json::json!({ "action": "create" }),
            raw: None,
            status: crate::ToolCallStatus::Completed,
            state: Some(crate::ToolState::Completed {
                input: serde_json::json!({ "action": "create" }),
                output: "created".to_string(),
                title: "Skill created".to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::CompletedTime {
                    start: chrono::Utc::now().timestamp_millis(),
                    end: chrono::Utc::now().timestamp_millis(),
                    compacted: None,
                },
                attachments: None,
            }),
        },
        created_at: chrono::Utc::now(),
        message_id: None,
    });

    SessionPrompt::maybe_append_runtime_skill_save_suggestion(&mut session, 0);

    assert!(!session.messages.iter().any(|message| {
        message
            .metadata
            .get("runtime_hint")
            .and_then(|value| value.as_str())
            == Some("skill_save_suggestion")
    }));
}
