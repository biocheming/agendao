//! Canonical frontend event receivers for in-process transports.
//!
//! Local frontends subscribe to the same `FrontendEvent` bus as HTTP and Unix
//! transports; this module owns only receiver lifecycle and stream coalescing.

use std::sync::Arc;

use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
use agendao_types::{LiveMessagePartIdentity, LivePartPhase};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::live_snapshot::{coalesced_text_field, LiveSnapshotAccumulator};
use crate::ServerState;

/// Spawn a local frontend receiver for one session.
pub fn spawn_local_session_events(
    state: Arc<ServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        local_session_event_loop(&state, &session_id, tx, cancel).await;
    });
    rx
}

/// Spawn a local receiver for the full canonical frontend bus.
///
/// Consumers that need dynamic client-side routing should subscribe once here
/// and apply their own session filter locally. This avoids the "unsubscribe old
/// session / subscribe new session" race where early FrontendEvents for a newly
/// selected session can be dropped before the new subscription is live.
pub fn spawn_local_frontend_events(
    state: Arc<ServerState>,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        local_frontend_event_loop(&state, tx, cancel).await;
    });
    rx
}

/// Recover an owned FrontendEvent from the shared bus envelope. In-process
/// consumers never touch the JSON wire text.
fn into_owned_event(bus: Arc<FrontendBusEvent>) -> FrontendEvent {
    match Arc::try_unwrap(bus) {
        Ok(envelope) => envelope.into_event(),
        Err(shared) => shared.event().clone(),
    }
}

async fn local_session_event_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = LiveSnapshotAccumulator::default();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            recv = event_rx.recv() => {
                let Ok(bus) = recv else {
                    break;
                };
                // 类型安全的进程内通道:直接在 borrowed 事件上做 session 过滤,
                // 只有命中的事件才取回 owned 值进入 coalesce —— 不再像旧实现
                // 那样先全文 JSON parse 再过滤丢弃。
                if frontend_event_session_id(bus.event()) != Some(session_id) {
                    continue;
                }
                let frontend_event =
                    coalesce_live_output_block(into_owned_event(bus), &mut live_output_accum);
                // 接收端已 drop（前端断开）:继续运行只会作为僵尸订阅挂在
                // bus 上,立即退出。
                if tx.send(frontend_event).is_err() {
                    break;
                }
            }
        }
    }
}

async fn local_frontend_event_loop(
    state: &Arc<ServerState>,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = LiveSnapshotAccumulator::default();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            recv = event_rx.recv() => {
                let Ok(bus) = recv else {
                    break;
                };
                let frontend_event =
                    coalesce_live_output_block(into_owned_event(bus), &mut live_output_accum);
                if tx.send(frontend_event).is_err() {
                    break;
                }
            }
        }
    }
}

fn frontend_event_session_id(event: &FrontendEvent) -> Option<&str> {
    match event {
        FrontendEvent::SessionRuntimeReplaced { session_id, .. }
        | FrontendEvent::SessionProjectionReplaced { session_id, .. }
        | FrontendEvent::QuestionUpsert { session_id, .. }
        | FrontendEvent::QuestionRemoved { session_id, .. }
        | FrontendEvent::PermissionUpsert { session_id, .. }
        | FrontendEvent::PermissionRemoved { session_id, .. }
        | FrontendEvent::ToolCallUpsert { session_id, .. }
        | FrontendEvent::DiffReplaced { session_id, .. }
        | FrontendEvent::TodoReplaced { session_id, .. }
        | FrontendEvent::SessionError { session_id, .. }
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
        FrontendEvent::ConfigUpdated => None,
    }
}

fn coalesce_live_output_block(
    event: FrontendEvent,
    accum: &mut LiveSnapshotAccumulator,
) -> FrontendEvent {
    let FrontendEvent::OutputBlockAppended {
        session_id,
        mut block,
        id,
        live_identity,
    } = event
    else {
        return event;
    };

    let Some(identity) = live_identity.as_ref() else {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    };

    let Some(text_field) = coalesced_text_field(identity) else {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    };

    let text = block
        .get(text_field)
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let Some(accumulated) = accum.update(&session_id, identity, text) else {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    };

    if let Some(obj) = block.as_object_mut() {
        obj.insert(
            text_field.to_string(),
            serde_json::Value::String(accumulated),
        );
        obj.insert("phase".to_string(), serde_json::json!("full"));
    }

    FrontendEvent::OutputBlockAppended {
        session_id,
        block,
        id,
        live_identity: Some(LiveMessagePartIdentity {
            phase: LivePartPhase::Snapshot,
            ..identity.clone()
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::live_snapshot::LiveSnapshotAccumulator;
    use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
    use agendao_server_core::runtime_events::ToolCallPhase;
    use agendao_types::{
        LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase, ASSISTANT_TEXT_MAIN_PART_KEY,
    };

    #[test]
    fn frontend_event_session_id_extracts_session_scoped_variants() {
        let event = FrontendEvent::ToolCallUpsert {
            session_id: "ses_direct".to_string(),
            tool_call_id: "tool_1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        };

        assert_eq!(super::frontend_event_session_id(&event), Some("ses_direct"));
    }

    #[test]
    fn coalesces_live_message_append_into_full_snapshot() {
        let mut accum = LiveSnapshotAccumulator::default();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
        };

        let first = super::coalesce_live_output_block(
            FrontendEvent::OutputBlockAppended {
                session_id: "ses_direct".to_string(),
                block: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": "hel"
                }),
                id: Some("msg_1".to_string()),
                live_identity: Some(identity.clone()),
            },
            &mut accum,
        );
        let second = super::coalesce_live_output_block(
            FrontendEvent::OutputBlockAppended {
                session_id: "ses_direct".to_string(),
                block: serde_json::json!({
                    "kind": "message",
                    "phase": "delta",
                    "text": "lo"
                }),
                id: Some("msg_1".to_string()),
                live_identity: Some(identity),
            },
            &mut accum,
        );

        let FrontendEvent::OutputBlockAppended { block, .. } = first else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hel");
        assert_eq!(block["phase"], "full");

        let FrontendEvent::OutputBlockAppended { block, .. } = second else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hello");
        assert_eq!(block["phase"], "full");
    }

    #[test]
    fn snapshot_fragments_do_not_collapse_to_last_token() {
        let mut accum = LiveSnapshotAccumulator::default();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
        };

        let fragments = ["你", "好", "，世界"];
        let mut last = None;
        for fragment in fragments {
            last = Some(super::coalesce_live_output_block(
                FrontendEvent::OutputBlockAppended {
                    session_id: "ses_direct".to_string(),
                    block: serde_json::json!({
                        "kind": "message",
                        "phase": "full",
                        "text": fragment
                    }),
                    id: Some("msg_1".to_string()),
                    live_identity: Some(identity.clone()),
                },
                &mut accum,
            ));
        }

        let FrontendEvent::OutputBlockAppended { block, .. } = last.expect("coalesced event")
        else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "你好，世界");
        assert_eq!(block["phase"], "full");
    }

    // ── Bus-level: typed transport, filter/order/content/coalesce ──────

    fn bus_envelope(event: FrontendEvent) -> Arc<FrontendBusEvent> {
        Arc::new(FrontendBusEvent::new(event))
    }

    fn tool_upsert(session_id: &str, tool_call_id: &str) -> FrontendEvent {
        FrontendEvent::ToolCallUpsert {
            session_id: session_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        }
    }

    /// 等 bridge 任务完成订阅后再发送,避免 broadcast 的"发送时无接收者则
    /// 丢弃"语义造成测试竞态。
    async fn wait_bridge_subscribed(state: &crate::ServerState, receivers_before: usize) {
        for _ in 0..1000 {
            if state.frontend_bus.receiver_count() > receivers_before {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("local frontend receiver did not subscribe to frontend_bus in time");
    }

    /// local receiver 的 session 过滤、事件顺序与事件内容在 typed 传输下
    /// 逐字节不变;其它 session 的事件在 borrowed 阶段即被过滤(零克隆)。
    #[tokio::test]
    async fn local_session_events_filter_and_preserve_order_and_content() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut rx = super::spawn_local_session_events(
            Arc::clone(&state),
            "ses_a".to_string(),
            cancel.clone(),
        );
        wait_bridge_subscribed(&state, receivers_before).await;

        let envelopes = vec![
            bus_envelope(tool_upsert("ses_a", "tc_1")),
            bus_envelope(tool_upsert("ses_b", "tc_2")),
            bus_envelope(FrontendEvent::OutputBlockAppended {
                session_id: "ses_a".to_string(),
                block: serde_json::json!({"kind": "message", "text": "hello"}),
                id: Some("msg_1".to_string()),
                live_identity: None,
            }),
        ];
        for envelope in &envelopes {
            state
                .frontend_bus
                .send(Arc::clone(envelope))
                .expect("send to frontend bus");
        }

        let first = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("first event within timeout")
            .expect("channel open");
        match first {
            FrontendEvent::ToolCallUpsert {
                session_id,
                tool_call_id,
                ..
            } => {
                assert_eq!(session_id, "ses_a");
                assert_eq!(tool_call_id, "tc_1");
            }
            other => panic!("expected ToolCallUpsert, got {:?}", other),
        }

        let second = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("second event within timeout")
            .expect("channel open");
        match second {
            FrontendEvent::OutputBlockAppended {
                session_id, block, ..
            } => {
                assert_eq!(session_id, "ses_a");
                assert_eq!(block["text"], "hello");
            }
            other => panic!("expected OutputBlockAppended, got {:?}", other),
        }

        // ses_b 的事件必须被过滤,没有第三条。
        assert!(
            tokio::time::timeout(Duration::from_millis(150), rx.recv())
                .await
                .is_err(),
            "events for other sessions must be filtered out"
        );

        // 探针:整条 direct 链路不允许物化 JSON 文本。
        for envelope in &envelopes {
            assert!(
                !envelope.is_json_materialized(),
                "local frontend receiver must not materialize JSON in-process"
            );
        }
        cancel.cancel();
    }

    /// 全总线变体不过滤 session,顺序与内容不变,且 live output 的
    /// coalesce 语义经 typed 传输后逐字节不变。
    #[tokio::test]
    async fn local_frontend_events_forward_all_sessions_with_coalesce_intact() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut rx = super::spawn_local_frontend_events(Arc::clone(&state), cancel.clone());
        wait_bridge_subscribed(&state, receivers_before).await;

        let identity = || LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
        };
        let live_chunk = |session: &str, text: &str| FrontendEvent::OutputBlockAppended {
            session_id: session.to_string(),
            block: serde_json::json!({"kind": "message", "phase": "delta", "text": text}),
            id: Some("msg_1".to_string()),
            live_identity: Some(identity()),
        };

        let envelopes = vec![
            bus_envelope(live_chunk("ses_a", "hel")),
            bus_envelope(tool_upsert("ses_b", "tc_9")),
            bus_envelope(live_chunk("ses_a", "lo")),
        ];
        for envelope in &envelopes {
            state
                .frontend_bus
                .send(Arc::clone(envelope))
                .expect("send to frontend bus");
        }

        let mut received = Vec::new();
        for _ in 0..3 {
            received.push(
                tokio::time::timeout(Duration::from_secs(2), rx.recv())
                    .await
                    .expect("event within timeout")
                    .expect("channel open"),
            );
        }

        // 顺序:ses_a chunk-1 → ses_b upsert → ses_a chunk-2。
        let FrontendEvent::OutputBlockAppended { block, .. } = &received[0] else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hel");
        assert_eq!(block["phase"], "full");

        assert!(matches!(
            &received[1],
            FrontendEvent::ToolCallUpsert { session_id, .. } if session_id == "ses_b"
        ));

        let FrontendEvent::OutputBlockAppended {
            block,
            live_identity,
            ..
        } = &received[2]
        else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "hello");
        assert_eq!(block["phase"], "full");
        assert_eq!(
            live_identity.as_ref().map(|identity| identity.phase),
            Some(LivePartPhase::Snapshot),
            "coalesced frame must carry Snapshot phase"
        );

        for envelope in &envelopes {
            assert!(
                !envelope.is_json_materialized(),
                "local frontend receiver must not materialize JSON in-process"
            );
        }
        cancel.cancel();
    }

    /// 客户端断开（rx 被 drop）后,bus bridge 任务必须在下一个事件到达时
    /// 退出并退订 frontend_bus,不再作为僵尸订阅累积。
    #[tokio::test]
    async fn local_frontend_task_exits_after_receiver_dropped() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let rx = super::spawn_local_frontend_events(Arc::clone(&state), cancel);
        wait_bridge_subscribed(&state, receivers_before).await;

        // 模拟客户端断开:writer 循环退出,rx 被 drop。
        drop(rx);

        // 下一个事件触发 send 失败,bridge 任务必须退出并退订。
        state
            .frontend_bus
            .send(bus_envelope(tool_upsert("ses_a", "tc_1")))
            .expect("send to frontend bus");

        for _ in 0..1000 {
            if state.frontend_bus.receiver_count() == receivers_before {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("bridge task did not exit after receiver was dropped");
    }
}
