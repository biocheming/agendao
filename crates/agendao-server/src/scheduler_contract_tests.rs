//! `run_scheduler` 契约测试 —— scheduler composition authority 的行为证明。
//!
//! 证明边界（见 docs/execution-authorities.md 与 docs/plans 核心收口计划 Phase 1）：
//! 这些测试证明 `run_scheduler` 在给定输入和控制序列下产生约定的执行事实与
//! 终态；它们不证明 provider 质量、所有平台实现或恢复/持久化 schema。
//!
//! 契约要点：
//! 1. Direct 模板：单文本 turn -> `Ok(SchedulerRunOutput)`，终态字段完整；
//! 2. provider 失败：错误信息沿 Err 路径完整传播（终态错误分类的唯一来源）；
//! 3. cancellation：运行中取消 -> `Err` 且归类为取消，运行在取消点 promptly 终止；
//! 4. 执行事实进入事件总线：scheduler step 事件经 frontend bus 可观测。

use super::{run_scheduler, SchedulerRunInput};
use crate::test_support::{target_fixture_root, ScriptedProvider, ScriptedTurn, text_turn};
use crate::ServerState;
use agendao_execution_types::CompiledExecutionRequest;
use agendao_orchestrator::selector::SchedulerChoice;
use agendao_orchestrator::templates::TemplateId;
use agendao_provider::ProviderError;
use agendao_server_core::frontend_events::FrontendEvent;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// 测试基础设施：state / session 装配（ScriptedProvider 见 test_support）
// ---------------------------------------------------------------------------

/// 构造 contract 测试用的 state + session + optimistic assistant message，
/// 返回 (state, session_id, assistant_message_id)。fixture 落在 CARGO_TARGET_DIR
/// 之下（test_support::target_fixture_root），不留仓库内或 /tmp 产物。
async fn contract_state(test: &str) -> (Arc<ServerState>, String, String) {
    let fixture = target_fixture_root(test);
    let state = Arc::new(ServerState::new_for_workspace(fixture.clone()));
    let mut session = agendao_session::Session::new("project", fixture.to_string_lossy().as_ref());
    let assistant_id = session.add_assistant_message().id.clone();
    let session_id = session.id.clone();
    state.sessions.lock().await.update(session);
    (state, session_id, assistant_id)
}

fn run_input(
    state: Arc<ServerState>,
    session_id: String,
    assistant_message_id: String,
    directory: String,
    provider: Arc<ScriptedProvider>,
    cancellation: CancellationToken,
) -> SchedulerRunInput {
    SchedulerRunInput {
        state,
        session_id,
        assistant_message_id,
        directory,
        goal: "contract: finish the assigned work".to_string(),
        choice: SchedulerChoice::Template {
            template: TemplateId::Direct,
        },
        primary_agent: None,
        provider,
        request: CompiledExecutionRequest::default(),
        conversation_seed: Vec::new(),
        execution_metadata: std::collections::HashMap::new(),
        cancellation,
    }
}

// ---------------------------------------------------------------------------
// 契约 1：Direct 模板正常完成 -> 结构化终态
// ---------------------------------------------------------------------------

#[tokio::test]
async fn direct_template_completes_with_structured_outcome() {
    let (state, session_id, assistant_message_id) =
        contract_state("direct-completes").await;
    let directory = state.project_root().to_string_lossy().to_string();
    let provider = ScriptedProvider::new(vec![text_turn("contract result: done")]);
    let input = run_input(
        state.clone(),
        session_id,
        assistant_message_id,
        directory,
        provider.clone(),
        CancellationToken::new(),
    );

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_scheduler(input),
    )
    .await
    .expect("run must finish within timeout")
    .expect("successful scripted run must return Ok");

    // 终态契约：SchedulerRunOutput 的每个字段都是权威读面，
    // 不允许调用方从错误文本猜测这些值。
    assert_eq!(output.result.summary, "contract result: done");
    assert_eq!(output.usage.model_calls, 1, "one scripted model call");
    assert!(!output.fingerprint.is_empty(), "blueprint fingerprint");
    // Direct 是模板选择（拓扑），blueprint 名是 run_scheduler 组合层的统一命名
    // （scheduler_runner.rs 将 TemplateParameters.name 固定为 "session-scheduler"）。
    assert_eq!(output.blueprint.name.as_str(), "session-scheduler");
    assert_eq!(provider.calls(), 1);
    // 评审信号是回流记账的来源，必须随终态返回。
    assert_eq!(output.review.tool_call_count, 0);
    assert_eq!(output.review.error_tool_call_count, 0);
    // used_skill_names 来自 blueprint 中 agent 节点的 skills；默认 primary
    // agent 携带内置 skill，非空是常态。这里不断言具体集合，只锁定
    // 「skill 记账随终态返回」的回流存在性（字段由 SchedulerRunOutput 携带）。
}

// ---------------------------------------------------------------------------
// 契约 2：provider 失败沿 Err 路径完整传播
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provider_failure_propagates_as_error_outcome() {
    let (state, session_id, assistant_message_id) =
        contract_state("provider-failure").await;
    let directory = state.project_root().to_string_lossy().to_string();
    let provider = ScriptedProvider::new(vec![ScriptedTurn::Fail(
        ProviderError::InvalidRequest("contract: provider refused".to_string()),
    )]);
    let input = run_input(
        state.clone(),
        session_id,
        assistant_message_id,
        directory,
        provider.clone(),
        CancellationToken::new(),
    );

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_scheduler(input),
    )
    .await
    .expect("failing run must finish within timeout");

    let error = match outcome {
        Ok(_) => panic!("provider failure must surface as Err"),
        Err(error) => error,
    };
    assert!(
        error.contains("contract: provider refused"),
        "error classification must preserve the provider failure cause, got: {error}"
    );
}

// ---------------------------------------------------------------------------
// 契约 4：执行事实进入 frontend 事件总线
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_emits_scheduler_step_events_to_frontend_bus() {
    let (state, session_id, assistant_message_id) =
        contract_state("frontend-bus-events").await;
    let directory = state.project_root().to_string_lossy().to_string();
    let provider = ScriptedProvider::new(vec![text_turn("visible work")]);
    // Direct transport 的标准装配：spawn frontend projector 把 ServerEvent
    // 投影到 frontend bus（HTTP 启动路径在 server.rs 内做同一件事）。
    state.ensure_frontend_projector();
    let mut receiver = state.subscribe_frontend_events();
    let input = run_input(
        state.clone(),
        session_id.clone(),
        assistant_message_id,
        directory,
        provider.clone(),
        CancellationToken::new(),
    );

    let run = tokio::spawn(run_scheduler(input));
    // 订阅在 run 之前建立；收集 run 期间到达 frontend bus 的事件。
    //
    // 边界证据（Phase 1 核心结论）：run_scheduler 是 composition authority，
    // 它产出执行事实（scheduler_step 进度卡经 ServerEvent::OutputBlock 直通
    // 投影），但**不注册 session runtime** —— `register_scheduler_run` 与
    // run 状态广播由 prompt route 的 session lifecycle 段负责
    // （prompt.rs:2047-2050；projector 的 SessionRuntimeReplaced 依赖
    // runtime telemetry 快照，见 frontend_projection.rs:430-438）。
    // 因此本测试断言 scheduler_step 可观测，而不断言 session.runtime.replaced ——
    // 那是 route 层 lifecycle 的契约面。
    let mut saw_scheduler_step = false;
    while let Ok(event) =
        tokio::time::timeout(std::time::Duration::from_secs(30), receiver.recv()).await
    {
        let event = event.expect("frontend bus channel must stay open");
        match event.event() {
            FrontendEvent::OutputBlockAppended { block, .. } => {
                if block.get("kind").and_then(|kind| kind.as_str()) == Some("session_event") {
                    saw_scheduler_step = true;
                }
            }
            _ => {}
        }
        if run.is_finished() && saw_scheduler_step {
            break;
        }
        if run.is_finished()
            && tokio::time::timeout(
                std::time::Duration::from_millis(200),
                receiver.recv(),
            )
            .await
            .is_err()
        {
            break;
        }
    }
    let _ = run.await;

    assert!(
        saw_scheduler_step,
        "scheduler step progress must be observable as session_event output blocks"
    );
}

// ---------------------------------------------------------------------------
// 契约 3：cancellation —— 运行中取消的终止与静止语义
// ---------------------------------------------------------------------------

/// 契约前半部分（已实现）：挂起中的模型调用被取消后，run 在取消点
/// promptly 返回归类为取消的 Err。
///
/// 取消静止契约：取消返回后，provider 不得再被调用。
/// 这明确禁止取消后的重试/重规划再触模型；若未来需要优雅收尾调用，
/// 必须显式修改该契约及其测试。
#[tokio::test]
async fn cancelled_run_returns_cancelled_error_and_goes_quiet() {
    let (state, session_id, assistant_message_id) =
        contract_state("cancelled-run").await;
    let directory = state.project_root().to_string_lossy().to_string();
    let provider = ScriptedProvider::new(vec![text_turn("never finishing turn")]);
    provider.hang();
    let cancellation = CancellationToken::new();
    let input = run_input(
        state.clone(),
        session_id,
        assistant_message_id,
        directory,
        provider.clone(),
        cancellation.clone(),
    );

    let run = tokio::spawn(run_scheduler(input));
    // 等模型调用真正 in-flight，再取消 —— 证明取消作用于运行中执行体。
    let mut in_flight = false;
    for _ in 0..100 {
        if provider.calls() >= 1 {
            in_flight = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(in_flight, "scripted model call must become in-flight");

    cancellation.cancel();
    let joined = tokio::time::timeout(std::time::Duration::from_secs(10), run)
        .await
        .expect("cancelled run must return promptly (select! cancellation branch)")
        .expect("run task must join cleanly after cancellation");

    let error = match joined {
        Ok(_) => panic!("cancelled run must surface as Err"),
        Err(error) => error,
    };
    assert!(
        error.to_lowercase().contains("cancel"),
        "error must be classified as cancellation, got: {error}"
    );

    // 取消返回后进入静止窗口：不得发生取消后的重试、重规划或补偿性模型调用。
    let calls_at_cancel = provider.calls();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        provider.calls(),
        calls_at_cancel,
        "provider must stay quiet after cancellation returns"
    );
}
