//! 三 transport parity 测试 —— Direct(local 直调) / Unix socket / HTTP 的
//! 同输入行为一致性证明（docs/plans 核心收口计划 Phase 4）。
//!
//! parity 的实质对象是 transport 序列化与事件流层：三 transport 共享同一
//! handler 链（Phase 0 已证；unix 的 handle_prompt 内部即调 crate::local_prompt），
//! 本测试在运行时证明同一输入经三种编码路径产生同构的终态与事件类别序列。
//!
//! 证明边界：不证明 SSE 断线重连、TUI/Web projector 的消费细节（后续增量，
//! 见 docs/execution-authorities.md 第七节）。

use crate::test_support::{target_fixture_root, text_turn, ScriptedProvider};
use crate::ServerState;
use agendao_provider::ModelInfo;
use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 注册带可解析 model 的 ScriptedProvider 并返回 (state, model 字符串)。
async fn scripted_state(test: &str) -> (Arc<ServerState>, String) {
    let fixture = target_fixture_root(test);
    let state = Arc::new(ServerState::new_for_workspace(fixture.clone()));
    let provider = ScriptedProvider::new(vec![text_turn("parity result")])
        .with_model_info(scripted_model_info());
    state.providers.write().await.register_arc(provider);
    (state, "scripted/scripted-1".to_string())
}

fn scripted_model_info() -> ModelInfo {
    ModelInfo {
        id: "scripted-1".to_string(),
        name: "Scripted Test Model".to_string(),
        provider: "scripted".to_string(),
        context_window: 8192,
        max_input_tokens: None,
        max_output_tokens: 4096,
        supports_vision: false,
        supports_tools: true,
        cost_per_million_input: 0.0,
        cost_per_million_output: 0.0,
        cost_per_million_cache_read: None,
        cost_per_million_cache_write: None,
    }
}

/// 在 state 里建 session，返回 session_id。
async fn seed_session(state: &Arc<ServerState>) -> String {
    let fixture = state.workspace_root().to_path_buf();
    let session = agendao_session::Session::new("project", fixture.to_string_lossy().as_ref());
    let session_id = session.id.clone();
    state.sessions.lock().await.update(session);
    session_id
}

/// 等待 run 完成：轮询最新 assistant message 的 finish 字段非空。
async fn wait_for_run_settled(
    state: &Arc<ServerState>,
    session_id: &str,
) -> agendao_session::Session {
    for _ in 0..300 {
        let sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get(session_id) {
            let settled = session
                .record()
                .messages
                .iter()
                .rev()
                .find(|message| message.role == agendao_session::MessageRole::Assistant)
                .and_then(|message| message.finish.clone())
                .is_some();
            if settled {
                return session.clone();
            }
        }
        drop(sessions);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("run did not settle within 30s for session {session_id}");
}

/// 从 frontend bus 收集事件类别序列，直到 run settle 后 bus 静默。
async fn collect_event_categories(
    state: &Arc<ServerState>,
    session_id: &str,
    receiver: &mut tokio::sync::broadcast::Receiver<Arc<FrontendBusEvent>>,
) -> Vec<String> {
    let mut categories = Vec::new();
    loop {
        let settled = {
            let sessions = state.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|session| {
                    session
                        .record()
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.role == agendao_session::MessageRole::Assistant)
                        .and_then(|message| message.finish.clone())
                })
                .is_some()
        };
        let next = tokio::time::timeout(
            if settled {
                std::time::Duration::from_millis(500)
            } else {
                std::time::Duration::from_secs(30)
            },
            receiver.recv(),
        )
        .await;
        match next {
            Ok(Ok(event)) => categories.push(category_of(event.event())),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Err(_) | Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
        }
    }
    categories
}

fn category_of(event: &FrontendEvent) -> String {
    match event {
        FrontendEvent::OutputBlockAppended { block, .. } => format!(
            "output_block:{}",
            block.get("kind").and_then(|k| k.as_str()).unwrap_or("?")
        ),
        FrontendEvent::SessionRuntimeReplaced { .. } => "session.runtime.replaced".to_string(),
        FrontendEvent::SessionProjectionReplaced { .. } => {
            "session.projection.replaced".to_string()
        }
        FrontendEvent::QuestionUpsert { .. } => "question.upsert".to_string(),
        FrontendEvent::QuestionRemoved { .. } => "question.removed".to_string(),
        FrontendEvent::PermissionUpsert { .. } => "permission.upsert".to_string(),
        FrontendEvent::PermissionRemoved { .. } => "permission.removed".to_string(),
        FrontendEvent::DiffReplaced { .. } => "diff.replaced".to_string(),
        FrontendEvent::TodoReplaced { .. } => "todo.replaced".to_string(),
        _ => "other".to_string(),
    }
}

/// 终态形状提取：三 transport 应产出一致的 (finish, 文本, 元数据键集合)。
fn assistant_outcome_shape(session: &agendao_session::Session) -> (String, String, Vec<String>) {
    let assistant = session
        .record()
        .messages
        .iter()
        .rev()
        .find(|message| message.role == agendao_session::MessageRole::Assistant)
        .expect("assistant message must exist");
    let mut metadata_keys = assistant.metadata.keys().cloned().collect::<Vec<_>>();
    metadata_keys.sort();
    (
        assistant.finish.clone().unwrap_or_default(),
        assistant_visible_text(assistant),
        metadata_keys,
    )
}

fn assistant_visible_text(message: &agendao_session::SessionMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match &part.part_type {
            agendao_session::PartType::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// 起一个测试 unix socket server，返回 socket path。
async fn spawn_unix_server(state: &Arc<ServerState>) -> std::path::PathBuf {
    // socket parent 必须由 bind() 自己创建：prepare_socket_parent 对已存在
    // 目录要求 022 位干净，而宿主 umask（如 0002）会让预创建目录带
    // group-w 被拒；新建路径会被显式 chmod 0700，不受环境影响。
    let socket_path = state.workspace_root().join("sock").join("s.sock");
    let server = crate::unix_socket::UnixSocketServer::new(
        state.clone(),
        socket_path.to_string_lossy().into_owned(),
    );
    let listener = server.bind().expect("test socket must bind");
    tokio::spawn(async move {
        let _ = server.serve_bound(listener).await;
    });
    socket_path
}

/// 经 unix transport 发一条 prompt（JSON-RPC 行协议），返回响应 Value。
async fn unix_prompt(
    socket_path: &std::path::Path,
    session_id: &str,
    model: &str,
) -> serde_json::Value {
    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .expect("connect to test unix socket");
    let (reader, mut writer) = stream.into_split();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "prompt",
        "params": {
            "session_id": session_id,
            "text": "unix parity probe",
            "model": model,
            // 显式 Direct 模板：parity 对象是 transport 编码层，不是 planner
            // 的 AI 选择；三路统一绕开 Auto 的 planning 模型调用。
            "scheduler": { "kind": "template", "template": "direct" },
        }
    });
    writer
        .write_all(format!("{request}\n").as_bytes())
        .await
        .expect("write json-rpc request");
    writer.flush().await.expect("flush request");
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read response");
    serde_json::from_str(line.trim()).expect("response must be valid json")
}

/// 经 HTTP transport（axum router oneshot）发一条 prompt，断言 200。
async fn http_prompt(state: &Arc<ServerState>, session_id: &str, model: &str) {
    use axum::body::Body;
    use tower::ServiceExt;

    let router = crate::routes::router().with_state(state.clone());
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(format!("/session/{session_id}/prompt"))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "message": "http parity probe",
                "model": model,
                "scheduler": { "kind": "template", "template": "direct" },
            })
            .to_string(),
        ))
        .expect("build http request");
    let response = router.oneshot(request).await.expect("oneshot must respond");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::OK,
        "http prompt must be accepted"
    );
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
}

/// 三路共用的显式 Direct 模板选择（绕开 Auto 的 planning 模型调用，
/// parity 对象是 transport 编码层而非 AI 选择）。
fn direct_template_choice() -> agendao_orchestrator::selector::SchedulerChoice {
    agendao_orchestrator::selector::SchedulerChoice::Template {
        template: agendao_orchestrator::templates::TemplateId::Direct,
    }
}

/// 相邻去重（保留类别流顺序，压缩时间性重复）。
fn dedup_adjacent(sequence: &[String]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for category in sequence {
        if result.last().map(|last| last != category).unwrap_or(true) {
            result.push(category.clone());
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Transport 1：Unix socket 全栈往返（真实 socket + JSON-RPC 行协议）
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unix_transport_prompt_roundtrip_produces_contract_outcome() {
    let (state, model) = scripted_state("parity-unix").await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();

    let socket_path = spawn_unix_server(&state).await;
    let response = unix_prompt(&socket_path, &session_id, &model).await;
    assert!(
        response.get("error").is_none(),
        "unix prompt must not error: {response}"
    );

    let session = wait_for_run_settled(&state, &session_id).await;
    let (finish, text, _) = assistant_outcome_shape(&session);
    assert_eq!(finish, "stop");
    assert_eq!(text, "parity result");
}

// ---------------------------------------------------------------------------
// Transport 2：HTTP（axum router oneshot，同一 handler 绑定）
// ---------------------------------------------------------------------------

#[cfg(feature = "http")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_transport_prompt_roundtrip_produces_contract_outcome() {
    let (state, model) = scripted_state("parity-http").await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();

    http_prompt(&state, &session_id, &model).await;

    let session = wait_for_run_settled(&state, &session_id).await;
    let (finish, text, _) = assistant_outcome_shape(&session);
    assert_eq!(finish, "stop");
    assert_eq!(text, "parity result");
}

// ---------------------------------------------------------------------------
// Transport 3 + 比对：三 transport 同输入的事件类别序列与终态形状一致
// ---------------------------------------------------------------------------

#[cfg(all(unix, feature = "http"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_transports_produce_identical_outcome_and_event_categories() {
    let mut outcomes = Vec::new();
    let mut event_sequences = Vec::new();

    // — Local 直调（无编码层；unix handler 内部即走此路径）—
    {
        let (state, model) = scripted_state("parity-local").await;
        let session_id = seed_session(&state).await;
        state.ensure_frontend_projector();
        let mut receiver = state.subscribe_frontend_events();

        let request = agendao_api::PromptRequest {
            message: Some("local parity probe".to_string()),
            parts: None,
            idempotency_key: None,
            ingress_source: Some("cli".to_string()),
            source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
            source_surface: Some(agendao_types::MessageSourceSurface::Direct),
            agent: None,
            scheduler: Some(direct_template_choice()),
            model: Some(model),
            variant: None,
            reasoning_effort: None,
            command: None,
            arguments: None,
        };
        crate::local_prompt(state.clone(), &session_id, request)
            .await
            .expect("local prompt must be accepted");

        let events = collect_event_categories(&state, &session_id, &mut receiver).await;
        let session = wait_for_run_settled(&state, &session_id).await;
        outcomes.push(assistant_outcome_shape(&session));
        event_sequences.push(events);
    }

    // — Unix —
    {
        let (state, model) = scripted_state("parity-three-unix").await;
        let session_id = seed_session(&state).await;
        state.ensure_frontend_projector();
        let mut receiver = state.subscribe_frontend_events();

        let socket_path = spawn_unix_server(&state).await;
        let response = unix_prompt(&socket_path, &session_id, &model).await;
        assert!(response.get("error").is_none(), "unix: {response}");

        let events = collect_event_categories(&state, &session_id, &mut receiver).await;
        let session = wait_for_run_settled(&state, &session_id).await;
        outcomes.push(assistant_outcome_shape(&session));
        event_sequences.push(events);
    }

    // — HTTP —
    {
        let (state, model) = scripted_state("parity-three-http").await;
        let session_id = seed_session(&state).await;
        state.ensure_frontend_projector();
        let mut receiver = state.subscribe_frontend_events();

        http_prompt(&state, &session_id, &model).await;

        let events = collect_event_categories(&state, &session_id, &mut receiver).await;
        let session = wait_for_run_settled(&state, &session_id).await;
        outcomes.push(assistant_outcome_shape(&session));
        event_sequences.push(events);
    }

    // — parity 断言：终态形状（finish/文本/元数据键集合）三路一致 —
    assert_eq!(outcomes.len(), 3);
    assert_eq!(
        outcomes[0], outcomes[1],
        "local and unix assistant outcomes must be identical"
    );
    assert_eq!(
        outcomes[1], outcomes[2],
        "unix and http assistant outcomes must be identical"
    );
    assert_eq!(outcomes[0].0, "stop", "all transports finish with stop");
    assert_eq!(outcomes[0].1, "parity result", "scripted text everywhere");

    // — parity 断言：事件类别序列（相邻去重保序）三路一致 —
    let local = dedup_adjacent(&event_sequences[0]);
    let unix = dedup_adjacent(&event_sequences[1]);
    let http = dedup_adjacent(&event_sequences[2]);
    assert_eq!(
        local, unix,
        "local and unix event category sequences must match:\nlocal={local:?}\nunix={unix:?}"
    );
    assert_eq!(
        unix, http,
        "unix and http event category sequences must match:\nunix={unix:?}\nhttp={http:?}"
    );
}
