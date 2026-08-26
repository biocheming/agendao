//! 三 transport parity 测试 —— Direct(local 直调) / Unix socket / HTTP 的
//! 同输入行为一致性证明（docs/plans 核心收口计划 Phase 4）。
//!
//! 证明边界（审计后精确化）：本测试证明 **handler ingress + internal
//! frontend bus parity** —— 同一输入经三种 transport 编码路径进入同一
//! handler 链，产出同构的 assistant 终态与同构的 frontend bus 事件类别
//! 序列，且 provider 实际收到的 prompt 一致。它不证明：HTTP/SSE 线缆
//! 序列化、subscription tier、event coalescing、auth/header 边界、unix
//! 事件协议编码（`subscribe_events` 流模式）——这些是登记的后续增量，
//! 见 docs/execution-authorities.md 第七节。
//!
//! 覆盖三个场景（成功 / provider failure / cancellation）：

use crate::test_support::{target_fixture_root, text_turn, ScriptedProvider, ScriptedTurn};
use crate::ServerState;
use agendao_provider::ModelInfo;
use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 三路共用 prompt：parity 断言 provider 实际收到的输入一致，
/// 输入在任一 transport 被丢失/改写/误解析都会在此暴露。
const PARITY_PROMPT: &str = "parity probe: finish the assigned work";

/// 一路 transport 的完整观测结果。
struct TransportObservation {
    /// (finish, 可见文本, 排序后的 metadata 键集合, 错误分类文本)
    outcome: (String, String, Vec<String>, Option<String>),
    /// frontend bus 事件类别序列（收集自订阅起点至 run settle 后静默）。
    events: Vec<String>,
    /// provider 每次调用实际收到的最后一条 user text。
    provider_user_texts: Vec<String>,
    provider: Arc<ScriptedProvider>,
}

/// 注册带可解析 model 的 ScriptedProvider 并返回 (state, model 串, provider)。
async fn scripted_state(
    _test: &str,
    script: Vec<ScriptedTurn>,
) -> (Arc<ServerState>, String, Arc<ScriptedProvider>) {
    // Unix-domain sockets have a small platform path limit (`SUN_LEN`). Keep
    // the fixture component short; target_fixture_root adds pid+sequence so
    // parallel scenarios remain unique without making the socket path too
    // long on CI workspaces with deep absolute paths.
    let fixture = target_fixture_root("parity");
    let state = Arc::new(ServerState::new_for_workspace(fixture.clone()));
    let provider = ScriptedProvider::new(script).with_model_info(scripted_model_info());
    state.providers.write().await.register_arc(provider.clone());
    (state, "scripted/scripted-1".to_string(), provider)
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

/// 等待最新 assistant message 的 finish 达到期望值（"stop"/"error"/"cancelled"）。
async fn wait_for_run_finish(
    state: &Arc<ServerState>,
    session_id: &str,
    expected: &str,
) -> agendao_session::Session {
    for _ in 0..300 {
        let sessions = state.sessions.lock().await;
        if let Some(session) = sessions.get(session_id) {
            let done = session
                .record()
                .messages
                .iter()
                .rev()
                .find(|message| message.role == agendao_session::MessageRole::Assistant)
                .map(|message| message.finish.as_deref() == Some(expected))
                .unwrap_or(false);
            if done {
                return session.clone();
            }
        }
        drop(sessions);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("run did not reach finish={expected} within 30s for session {session_id}");
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

/// 事件类别：穷尽列出 FrontendEvent 全部变体（去掉通配兜底），新增变体时
/// 编译器强制本表更新 —— 防止未知事件被折叠成同一类别而隐藏三路差异。
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
        FrontendEvent::ToolCallUpsert { .. } => "tool_call.upsert".to_string(),
        FrontendEvent::SandboxExecutionUpsert { .. } => "sandbox.execution.upsert".to_string(),
        FrontendEvent::SandboxExecutionRemoved { .. } => "sandbox.execution.removed".to_string(),
        FrontendEvent::DiffReplaced { .. } => "diff.replaced".to_string(),
        FrontendEvent::TodoReplaced { .. } => "todo.replaced".to_string(),
        FrontendEvent::TaskLedgerReplaced { .. } => "task-ledger.replaced".to_string(),
        FrontendEvent::ConfigUpdated => "config.updated".to_string(),
        FrontendEvent::SessionError { .. } => "session.error".to_string(),
    }
}

/// 终态形状提取：三 transport 应产出一致的
/// (finish, 文本, 元数据键集合, 错误分类文本)。
fn assistant_outcome_shape(
    session: &agendao_session::Session,
) -> (String, String, Vec<String>, Option<String>) {
    let assistant = session
        .record()
        .messages
        .iter()
        .rev()
        .find(|message| message.role == agendao_session::MessageRole::Assistant)
        .expect("assistant message must exist");
    let mut metadata_keys = assistant.metadata.keys().cloned().collect::<Vec<_>>();
    metadata_keys.sort();
    let error_text = assistant
        .metadata
        .get("error")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    (
        assistant.finish.clone().unwrap_or_default(),
        assistant_visible_text(assistant),
        metadata_keys,
        error_text,
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

/// 起一个测试 unix socket server。返回 socket path 与 accept-loop 的
/// JoinHandle —— 测试必须 abort 它，否则遗留后台 task、listener 与
/// socket 文件（server Drop 会清理 socket 文件，abort 触发 drop）。
async fn spawn_unix_server(
    state: &Arc<ServerState>,
) -> (std::path::PathBuf, tokio::task::JoinHandle<()>) {
    // socket parent 必须由 bind() 自己创建：prepare_socket_parent 对已存在
    // 目录要求 022 位干净，而宿主 umask（如 0002）会让预创建目录带
    // group-w 被拒；新建路径会被显式 chmod 0700，不受环境影响。
    let socket_path = state.workspace_root().join("sock").join("s.sock");
    let server = crate::unix_socket::UnixSocketServer::new(
        state.clone(),
        socket_path.to_string_lossy().into_owned(),
    );
    let listener = server.bind().expect("test socket must bind");
    let handle = tokio::spawn(async move {
        let _ = server.serve_bound(listener).await;
    });
    (socket_path, handle)
}

/// 经 unix transport 发一条 prompt（JSON-RPC 行协议），返回响应 Value。
async fn unix_prompt(
    socket_path: &std::path::Path,
    session_id: &str,
    model: &str,
) -> serde_json::Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "prompt",
        "params": {
            "session_id": session_id,
            "text": PARITY_PROMPT,
            "model": model,
            // 显式 Direct 模板：parity 对象是 transport 编码层，不是 planner
            // 的 AI 选择；三路统一绕开 Auto 的 planning 模型调用。
            "scheduler": { "kind": "template", "template": "direct" },
        }
    });
    unix_rpc(socket_path, request).await
}

/// 经 unix transport 请求 abort（原生取消入口：handle_abort_session）。
async fn unix_abort(socket_path: &std::path::Path, session_id: &str) {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "abort_session",
        "params": { "session_id": session_id }
    });
    let response = unix_rpc(socket_path, request).await;
    assert!(response.get("error").is_none(), "unix abort: {response}");
}

async fn unix_rpc(socket_path: &std::path::Path, request: serde_json::Value) -> serde_json::Value {
    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .expect("connect to test unix socket");
    let (reader, mut writer) = stream.into_split();
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
                "message": PARITY_PROMPT,
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

/// 经 HTTP transport 请求 abort（原生取消入口：POST /session/{id}/abort）。
async fn http_abort(state: &Arc<ServerState>, session_id: &str) {
    use axum::body::Body;
    use tower::ServiceExt;

    let router = crate::routes::router().with_state(state.clone());
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(format!("/session/{session_id}/abort"))
        .body(Body::empty())
        .expect("build http abort request");
    let response = router.oneshot(request).await.expect("abort must respond");
    assert_eq!(
        response.status(),
        axum::http::StatusCode::OK,
        "http abort must be accepted"
    );
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
}

/// 等模型调用真正 in-flight（cancellation 场景：取消必须作用于运行中执行体）。
async fn wait_for_in_flight(provider: &Arc<ScriptedProvider>) {
    for _ in 0..250 {
        if provider.calls() >= 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("scripted model call must become in-flight");
}

/// 取消静止窗口：取消返回后 provider 不得再被调用（禁止取消后的重试、
/// 重规划或补偿性模型调用 —— 与 scheduler 契约测试同一硬断言）。
async fn assert_provider_quiet_after_settle(provider: &Arc<ScriptedProvider>) {
    let calls_at_settle = provider.calls();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        provider.calls(),
        calls_at_settle,
        "provider must stay quiet after the run settles"
    );
}

/// 相邻去重（保留类别流顺序，压缩时间性重复；跨 transport 的同类事件
/// 重复次数天然抖动，语义顺序才是比对对象）。
fn dedup_adjacent(sequence: &[String]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for category in sequence {
        if result.last().map(|last| last != category).unwrap_or(true) {
            result.push(category.clone());
        }
    }
    result
}

/// 收尾观测：settle 后收集 provider 输入与终态形状。
async fn observe(
    state: &Arc<ServerState>,
    session_id: &str,
    expected_finish: &str,
    events: Vec<String>,
    provider: Arc<ScriptedProvider>,
) -> TransportObservation {
    let session = wait_for_run_finish(state, session_id, expected_finish).await;
    TransportObservation {
        outcome: assistant_outcome_shape(&session),
        events,
        provider_user_texts: provider.received_user_texts(),
        provider,
    }
}

/// 三路 parity 断言：终态形状、事件类别序列（相邻去重保序）、provider
/// 实际输入全部一致。
fn assert_three_way_parity(label: &str, observations: &[TransportObservation; 3]) {
    let [local, unix, http] = observations;

    // — 输入 parity：provider 实际收到的 prompt 三路一致且即 PARITY_PROMPT —
    let expected_input = [PARITY_PROMPT.to_string()];
    assert_eq!(
        local.provider_user_texts, expected_input,
        "{label}: local must deliver the exact parity prompt to the provider"
    );
    assert_eq!(
        unix.provider_user_texts, local.provider_user_texts,
        "{label}: unix provider input must match local"
    );
    assert_eq!(
        http.provider_user_texts, local.provider_user_texts,
        "{label}: http provider input must match local"
    );

    // — 终态形状 parity —
    assert_eq!(
        local.outcome, unix.outcome,
        "{label}: local and unix assistant outcomes must be identical"
    );
    assert_eq!(
        unix.outcome, http.outcome,
        "{label}: unix and http assistant outcomes must be identical"
    );

    // — 事件类别序列 parity —
    let local_events = dedup_adjacent(&local.events);
    let unix_events = dedup_adjacent(&unix.events);
    let http_events = dedup_adjacent(&http.events);
    assert_eq!(
        local_events, unix_events,
        "{label}: local and unix event category sequences must match:\nlocal={local_events:?}\nunix={unix_events:?}"
    );
    assert_eq!(
        unix_events, http_events,
        "{label}: unix and http event category sequences must match:\nunix={unix_events:?}\nhttp={http_events:?}"
    );
}

// ---------------------------------------------------------------------------
// 场景 1：成功路径 —— 单测级往返 + 三路比对
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unix_transport_prompt_roundtrip_produces_contract_outcome() {
    let (state, model, _provider) =
        scripted_state("parity-unix", vec![text_turn("parity result")]).await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();

    let (socket_path, server) = spawn_unix_server(&state).await;
    let response = unix_prompt(&socket_path, &session_id, &model).await;
    server.abort();
    let _ = server.await;
    assert!(
        response.get("error").is_none(),
        "unix prompt must not error: {response}"
    );

    let session = wait_for_run_finish(&state, &session_id, "stop").await;
    let (finish, text, _, _) = assistant_outcome_shape(&session);
    assert_eq!(finish, "stop");
    assert_eq!(text, "parity result");
}

#[cfg(feature = "http")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_transport_prompt_roundtrip_produces_contract_outcome() {
    let (state, model, _provider) =
        scripted_state("parity-http", vec![text_turn("parity result")]).await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();

    http_prompt(&state, &session_id, &model).await;

    let session = wait_for_run_finish(&state, &session_id, "stop").await;
    let (finish, text, _, _) = assistant_outcome_shape(&session);
    assert_eq!(finish, "stop");
    assert_eq!(text, "parity result");
}

/// 场景驱动：prompt 发出后（可选）中途回收 run。每段独立 state + session，
/// 返回该路的完整观测。
enum MidRunAction {
    None,
    Abort,
}

#[cfg(all(unix, feature = "http"))]
async fn drive_local(label: &str, action: MidRunAction) -> TransportObservation {
    let (state, model, provider) =
        scripted_state(&format!("parity-{label}-local"), vec![turn_for(label)]).await;
    maybe_hang(&provider, &action);
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    let request = agendao_api::PromptRequest {
        message: Some(PARITY_PROMPT.to_string()),
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

    if matches!(action, MidRunAction::Abort) {
        wait_for_in_flight(&provider).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            crate::local_abort_session(state.clone(), &session_id),
        )
        .await
        .expect("local abort must return promptly")
        .expect("local abort must be accepted");
    }
    let events = collect_event_categories(&state, &session_id, &mut receiver).await;
    observe(
        &state,
        &session_id,
        expected_finish(&action),
        events,
        provider,
    )
    .await
}

#[cfg(all(unix, feature = "http"))]
async fn drive_unix(label: &str, action: MidRunAction) -> TransportObservation {
    let (state, model, provider) =
        scripted_state(&format!("parity-{label}-unix"), vec![turn_for(label)]).await;
    maybe_hang(&provider, &action);
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    let (socket_path, server) = spawn_unix_server(&state).await;
    let response = if matches!(action, MidRunAction::Abort) {
        // The prompt RPC intentionally waits for the run's terminal message.
        // Send it in the background so a second connection can issue abort.
        let prompt_socket = socket_path.clone();
        let prompt_session = session_id.clone();
        let prompt_model = model.clone();
        let prompt_task = tokio::spawn(async move {
            unix_prompt(&prompt_socket, &prompt_session, &prompt_model).await
        });
        wait_for_in_flight(&provider).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            unix_abort(&socket_path, &session_id),
        )
        .await
        .expect("unix abort must return promptly");
        prompt_task
            .await
            .expect("unix prompt task must join cleanly")
    } else {
        unix_prompt(&socket_path, &session_id, &model).await
    };
    assert!(response.get("error").is_none(), "unix prompt: {response}");
    let events = collect_event_categories(&state, &session_id, &mut receiver).await;
    let observation = observe(
        &state,
        &session_id,
        expected_finish(&action),
        events,
        provider,
    )
    .await;
    server.abort();
    let _ = server.await;
    observation
}

#[cfg(all(unix, feature = "http"))]
async fn drive_http(label: &str, action: MidRunAction) -> TransportObservation {
    let (state, model, provider) =
        scripted_state(&format!("parity-{label}-http"), vec![turn_for(label)]).await;
    maybe_hang(&provider, &action);
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    http_prompt(&state, &session_id, &model).await;
    if matches!(action, MidRunAction::Abort) {
        wait_for_in_flight(&provider).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            http_abort(&state, &session_id),
        )
        .await
        .expect("http abort must return promptly");
    }
    let events = collect_event_categories(&state, &session_id, &mut receiver).await;
    observe(
        &state,
        &session_id,
        expected_finish(&action),
        events,
        provider,
    )
    .await
}

/// 场景脚本：成功场景回固定文本；取消场景挂起流（等待取消）。
/// 失败场景由 `drive_failure_*` 单独构造（见下）。
#[cfg(all(unix, feature = "http"))]
fn turn_for(label: &str) -> ScriptedTurn {
    match label {
        "success" => text_turn("parity result"),
        "cancel" => text_turn("never finishing turn"),
        _ => unreachable!("unknown scenario label {label}"),
    }
}

#[cfg(all(unix, feature = "http"))]
fn maybe_hang(provider: &Arc<ScriptedProvider>, action: &MidRunAction) {
    if matches!(action, MidRunAction::Abort) {
        provider.hang();
    }
}

#[cfg(all(unix, feature = "http"))]
fn expected_finish(action: &MidRunAction) -> &'static str {
    match action {
        MidRunAction::None => "stop",
        MidRunAction::Abort => "cancelled",
    }
}

/// 三路共用的显式 Direct 模板选择（绕开 Auto 的 planning 模型调用，
/// parity 对象是 transport 编码层而非 AI 选择）。
#[cfg(all(unix, feature = "http"))]
fn direct_template_choice() -> agendao_orchestrator::selector::SchedulerChoice {
    agendao_orchestrator::selector::SchedulerChoice::Template {
        template: agendao_orchestrator::templates::TemplateId::Direct,
    }
}

#[cfg(all(unix, feature = "http"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_transports_produce_identical_outcome_and_event_categories() {
    let observations = [
        drive_local("success", MidRunAction::None).await,
        drive_unix("success", MidRunAction::None).await,
        drive_http("success", MidRunAction::None).await,
    ];
    assert_three_way_parity("success", &observations);
    assert_eq!(observations[0].outcome.0, "stop");
    assert_eq!(observations[0].outcome.1, "parity result");
}

// ---------------------------------------------------------------------------
// 场景 2：provider failure —— 三路失败终态与错误分类一致
// ---------------------------------------------------------------------------

#[cfg(all(unix, feature = "http"))]
async fn drive_local_failure() -> TransportObservation {
    let (state, model, provider) = scripted_state(
        "parity-failure-local",
        vec![ScriptedTurn::Fail(
            agendao_provider::ProviderError::InvalidRequest("parity: provider refused".to_string()),
        )],
    )
    .await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    let request = agendao_api::PromptRequest {
        message: Some(PARITY_PROMPT.to_string()),
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
    observe(&state, &session_id, "error", events, provider).await
}

#[cfg(all(unix, feature = "http"))]
async fn drive_unix_failure() -> TransportObservation {
    let (state, model, provider) = scripted_state(
        "parity-failure-unix",
        vec![ScriptedTurn::Fail(
            agendao_provider::ProviderError::InvalidRequest("parity: provider refused".to_string()),
        )],
    )
    .await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    let (socket_path, server) = spawn_unix_server(&state).await;
    let response = unix_prompt(&socket_path, &session_id, &model).await;
    assert!(response.get("error").is_none(), "unix prompt: {response}");
    let events = collect_event_categories(&state, &session_id, &mut receiver).await;
    let observation = observe(&state, &session_id, "error", events, provider).await;
    server.abort();
    let _ = server.await;
    observation
}

#[cfg(all(unix, feature = "http"))]
async fn drive_http_failure() -> TransportObservation {
    let (state, model, provider) = scripted_state(
        "parity-failure-http",
        vec![ScriptedTurn::Fail(
            agendao_provider::ProviderError::InvalidRequest("parity: provider refused".to_string()),
        )],
    )
    .await;
    let session_id = seed_session(&state).await;
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();

    http_prompt(&state, &session_id, &model).await;
    let events = collect_event_categories(&state, &session_id, &mut receiver).await;
    observe(&state, &session_id, "error", events, provider).await
}

#[cfg(all(unix, feature = "http"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_transports_produce_identical_provider_failure_outcomes() {
    let observations = [
        drive_local_failure().await,
        drive_unix_failure().await,
        drive_http_failure().await,
    ];
    assert_three_way_parity("failure", &observations);
    // 失败分类 parity：三路 assistant metadata 的 error 文本携带同一 provider
    // 因果（终态错误分类的唯一来源沿 Err 路径传播）。
    assert_eq!(observations[0].outcome.0, "error");
    assert!(
        observations[0]
            .outcome
            .3
            .as_deref()
            .unwrap_or_default()
            .contains("parity: provider refused"),
        "error classification must preserve the provider failure cause, got: {:?}",
        observations[0].outcome.3
    );
    for observation in &observations {
        assert_provider_quiet_after_settle(&observation.provider).await;
    }
}

// ---------------------------------------------------------------------------
// 场景 3：cancellation —— 三路中途回收的终态、静止与事件序列一致
// ---------------------------------------------------------------------------

#[cfg(all(unix, feature = "http"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_transports_produce_identical_cancellation_outcomes() {
    let observations = [
        drive_local("cancel", MidRunAction::Abort).await,
        drive_unix("cancel", MidRunAction::Abort).await,
        drive_http("cancel", MidRunAction::Abort).await,
    ];
    assert_three_way_parity("cancel", &observations);
    assert_eq!(observations[0].outcome.0, "cancelled");
    // 取消静止契约跨 transport 同构：回收后 provider 零后续调用。
    for observation in &observations {
        assert_provider_quiet_after_settle(&observation.provider).await;
    }
}
