//! Shared Direct-mode event bridge.
//!
//! In Direct (in-process) mode there is no HTTP server and no SSE transport.
//! Frontends subscribe to the canonical `FrontendEvent` bus directly.

use std::collections::HashMap;
use std::sync::Arc;

use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
use agendao_types::{LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ServerState;

/// Spawn a Direct-mode event loop for one session. Returns a receiver that the
/// frontend consumes.
pub fn spawn_direct_event_loop(
    state: Arc<ServerState>,
    session_id: String,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_event_subscription_loop(&state, &session_id, tx, cancel).await;
    });
    rx
}

/// Spawn a Direct-mode event loop for the full canonical frontend bus.
///
/// Consumers that need dynamic client-side routing should subscribe once here
/// and apply their own session filter locally. This avoids the "unsubscribe old
/// session / subscribe new session" race where early FrontendEvents for a newly
/// selected session can be dropped before the new subscription is live.
pub fn spawn_direct_event_bus(
    state: Arc<ServerState>,
    cancel: CancellationToken,
) -> mpsc::UnboundedReceiver<FrontendEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        direct_event_bus_loop(&state, tx, cancel).await;
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

async fn direct_event_subscription_loop(
    state: &Arc<ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = HashMap::<String, String>::new();

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

async fn direct_event_bus_loop(
    state: &Arc<ServerState>,
    tx: mpsc::UnboundedSender<FrontendEvent>,
    cancel: CancellationToken,
) {
    let mut event_rx = state.frontend_bus.subscribe();
    let mut live_output_accum = HashMap::<String, String>::new();

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
    accum: &mut HashMap<String, String>,
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

    let key = format!(
        "{}:{}:{}",
        session_id, identity.message_id, identity.part_key
    );

    if identity.phase == LivePartPhase::End {
        accum.remove(&key);
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    }

    if !matches!(identity.phase, LivePartPhase::Append | LivePartPhase::Snapshot) {
        return FrontendEvent::OutputBlockAppended {
            session_id,
            block,
            id,
            live_identity,
        };
    }

    let text = block
        .get(text_field)
        .and_then(|value| value.as_str())
        .unwrap_or("");

    // 原地更新累积缓冲（append 追加 / snapshot 就地归并），每个 chunk 只做
    // O(chunk) 增量工作；仅为即将发出的 full 帧克隆一次全文（full 帧本身必须
    // 携带累积全文，这一次拷贝是线上契约的下界）。旧实现每 chunk 做
    // clone + serde_json::json! 两份全文拷贝，随回答长度呈 O(n²) 总量。
    let entry = accum.entry(key).or_default();
    if identity.phase == LivePartPhase::Append {
        entry.reserve(text.len());
        entry.push_str(text);
    } else {
        merge_snapshot_text_in_place(entry, text);
    }
    let accumulated = entry.clone();

    if let Some(obj) = block.as_object_mut() {
        obj.insert(text_field.to_string(), serde_json::Value::String(accumulated));
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

fn coalesced_text_field(identity: &LiveMessagePartIdentity) -> Option<&'static str> {
    match identity.part_kind {
        LiveMessagePartKind::AssistantText | LiveMessagePartKind::AssistantReasoning => {
            Some("text")
        }
        LiveMessagePartKind::ToolCall => Some("detail"),
        _ => None,
    }
}

/// 原地版快照归并：逐字节等价于旧的"分配新 String 返回"版本
/// （`merged` 在所有分支下都以 `existing` 为前缀或与 `existing` 相同，
/// 因此只需 append 增量即可，无需每 chunk 重分配全文）。
///
/// 三态语义保持不变：
///   - 累积快照：incoming 以 existing 为前缀 → 追加增量（等价于替换为 incoming）。
///   - 重复/陈旧快照：existing 以 incoming 为前缀 → 保留 existing（去重）。
///   - 逐 chunk 片段：无前缀关系 → 去重叠后拼接（existing 原样保留，仅追加尾部）。
fn merge_snapshot_text_in_place(existing: &mut String, incoming: &str) {
    if incoming.is_empty() {
        return;
    }
    if existing.is_empty() {
        existing.reserve(incoming.len());
        existing.push_str(incoming);
        return;
    }
    if incoming.starts_with(existing.as_str()) {
        existing.push_str(&incoming[existing.len()..]);
        return;
    }
    if existing.starts_with(incoming) {
        return;
    }

    let overlap = suffix_prefix_overlap(existing, incoming);
    existing.reserve(incoming.len() - overlap);
    existing.push_str(&incoming[overlap..]);
}

fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
    let max = existing.len().min(incoming.len());
    for size in (1..=max).rev() {
        if existing.is_char_boundary(existing.len() - size)
            && incoming.is_char_boundary(size)
            && existing[existing.len() - size..] == incoming[..size]
        {
            return size;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use agendao_server_core::frontend_events::{FrontendBusEvent, FrontendEvent};
    use agendao_server_core::runtime_events::ToolCallPhase;
    use agendao_types::{
        LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase,
        ASSISTANT_TEXT_MAIN_PART_KEY,
    };

    #[test]
    fn frontend_event_session_id_extracts_session_scoped_variants() {
        let event = FrontendEvent::ToolCallUpsert {
            session_id: "ses_direct".to_string(),
            tool_call_id: "tool_1".to_string(),
            tool_name: "bash".to_string(),
            phase: ToolCallPhase::Start,
        };

        assert_eq!(
            super::frontend_event_session_id(&event),
            Some("ses_direct")
        );
    }

    #[test]
    fn coalesces_live_message_append_into_full_snapshot() {
        let mut accum = HashMap::new();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
            legacy_block_id: None,
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
        let mut accum = HashMap::new();
        let identity = LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: None,
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

        let FrontendEvent::OutputBlockAppended { block, .. } =
            last.expect("coalesced event")
        else {
            panic!("expected output block");
        };
        assert_eq!(block["text"], "你好，世界");
        assert_eq!(block["phase"], "full");
    }

    /// 旧的分配式参考实现（golden，逐字节保留修复前行为），仅用于等价性断言。
    fn reference_merge_snapshot_text(existing: Option<&str>, incoming: &str) -> String {
        let Some(existing) = existing.filter(|value| !value.is_empty()) else {
            return incoming.to_string();
        };
        if incoming.is_empty() {
            return existing.to_string();
        }
        if incoming.starts_with(existing) {
            return incoming.to_string();
        }
        if existing.starts_with(incoming) {
            return existing.to_string();
        }
        let overlap = super::suffix_prefix_overlap(existing, incoming);
        if overlap > 0 {
            let mut merged = String::with_capacity(existing.len() + incoming.len() - overlap);
            merged.push_str(existing);
            merged.push_str(&incoming[overlap..]);
            return merged;
        }
        let mut merged = String::with_capacity(existing.len() + incoming.len());
        merged.push_str(existing);
        merged.push_str(incoming);
        merged
    }

    /// 确定性伪随机 op 流生成器（不依赖外部 crates）。
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
            xs[(self.next() as usize) % xs.len()]
        }
    }

    const COALESCE_FRAGS: [&str; 8] = ["The", " answer", " is", " ", "ab", "你好", "，", "x"];

    fn live_identity(phase: LivePartPhase) -> LiveMessagePartIdentity {
        LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase,
            legacy_block_id: None,
        }
    }

    fn live_output_event(text: &str, block_phase: &str, phase: LivePartPhase) -> FrontendEvent {
        FrontendEvent::OutputBlockAppended {
            session_id: "ses_direct".to_string(),
            block: serde_json::json!({
                "kind": "message",
                "phase": block_phase,
                "text": text
            }),
            id: Some("msg_1".to_string()),
            live_identity: Some(live_identity(phase)),
        }
    }

    fn output_block_text(event: &FrontendEvent) -> Option<String> {
        let FrontendEvent::OutputBlockAppended { block, .. } = event else {
            return None;
        };
        block.get("text").and_then(|v| v.as_str()).map(str::to_string)
    }

    /// bridge 级 golden：混合 Append delta / Snapshot 累积 / 陈旧 / 重叠 /
    /// 片段 / End 重置的 op 流，逐帧断言发出的 full 快照文本与参考模型
    /// 逐字节一致，且 phase 语义不变（block "full" + identity Snapshot）。
    #[test]
    fn bridge_emitted_frames_match_reference_model_byte_for_byte() {
        let mut rng = Lcg(0xABCD_EF01_2345_6789);
        let mut accum = HashMap::new();
        let mut reference = String::new();
        let mut truth = String::new();

        for step in 0..400 {
            let event = match rng.next() % 8 {
                // Append delta：累积器纯追加。
                0 | 1 | 2 => {
                    let frag = rng.pick(&COALESCE_FRAGS);
                    truth.push_str(frag);
                    reference.push_str(frag);
                    live_output_event(frag, "delta", LivePartPhase::Append)
                }
                // 累积快照。
                3 | 4 => {
                    truth.push_str(rng.pick(&COALESCE_FRAGS));
                    let incoming = truth.clone();
                    reference = reference_merge_snapshot_text(Some(&reference), &incoming);
                    live_output_event(&incoming, "full", LivePartPhase::Snapshot)
                }
                // 陈旧快照（前一截前缀，按 char 边界截断）。
                5 => {
                    let mut cut = truth.len() / 2;
                    while !truth.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    let incoming = truth[..cut].to_string();
                    reference = reference_merge_snapshot_text(Some(&reference), &incoming);
                    live_output_event(&incoming, "full", LivePartPhase::Snapshot)
                }
                // 重叠片段快照。
                6 => {
                    let tail = rng.pick(&COALESCE_FRAGS);
                    let head = rng.pick(&COALESCE_FRAGS);
                    truth.push_str(head);
                    let incoming = format!("{tail}{head}");
                    reference = reference_merge_snapshot_text(Some(&reference), &incoming);
                    live_output_event(&incoming, "full", LivePartPhase::Snapshot)
                }
                // End：清零累积器，pass-through，不发 full 帧。
                _ => {
                    reference.clear();
                    truth.clear();
                    let out = super::coalesce_live_output_block(
                        live_output_event("", "end", LivePartPhase::End),
                        &mut accum,
                    );
                    assert!(
                        accum.is_empty(),
                        "step {step}: End must clear accumulated state"
                    );
                    let FrontendEvent::OutputBlockAppended { live_identity, .. } = &out else {
                        panic!("expected output block");
                    };
                    assert_eq!(
                        live_identity.as_ref().map(|identity| identity.phase),
                        Some(LivePartPhase::End),
                        "step {step}: End frame passes through with End phase"
                    );
                    continue;
                }
            };

            let out = super::coalesce_live_output_block(event, &mut accum);
            assert_eq!(
                output_block_text(&out),
                Some(reference.clone()),
                "step {step}: emitted full snapshot diverged from reference model"
            );
            let FrontendEvent::OutputBlockAppended {
                block, live_identity, ..
            } = &out
            else {
                panic!("expected output block");
            };
            assert_eq!(block["phase"], "full", "step {step}");
            assert_eq!(
                live_identity.as_ref().map(|identity| identity.phase),
                Some(LivePartPhase::Snapshot),
                "step {step}: emitted frame must carry Snapshot phase"
            );
        }
    }

    /// 分配量级断言（Append 流）：full 帧契约要求每帧携带累积全文，因此
    /// 每帧一次全文拷贝（Σ frame_len）是下界；旧实现为 clone + json! 两份
    /// （≈2× 下界）。新实现必须压在 1.5× 下界以内。
    #[test]
    fn bridge_append_stream_allocates_one_frame_copy_per_chunk() {
        const CHUNKS: usize = 400;
        const CHUNK: &str = "abcdefghijklmnopqrstuvwxy"; // 25 bytes
        let events: Vec<FrontendEvent> = (0..CHUNKS)
            .map(|_| live_output_event(CHUNK, "delta", LivePartPhase::Append))
            .collect();
        // 每帧一次全文拷贝的下界：Σ i*25。
        let inherent_floor: usize = (1..=CHUNKS).sum::<usize>() * CHUNK.len();

        let guard = crate::test_alloc::AllocGuard::start();
        let mut accum = HashMap::new();
        let mut last = None;
        for event in events {
            last = Some(super::coalesce_live_output_block(event, &mut accum));
        }
        let allocated = guard.bytes();
        drop(guard);

        assert_eq!(
            output_block_text(&last.expect("final frame")).map(|s| s.len()),
            Some(CHUNKS * CHUNK.len())
        );
        assert!(
            allocated < inherent_floor * 3 / 2,
            "append coalescing must allocate ~1 frame copy per chunk \
             (allocated {allocated} bytes; inherent floor {inherent_floor} bytes; \
             old clone+json! implementation ≈ 2× floor)"
        );
    }

    /// 分配量级断言（Snapshot 流）：旧实现每帧 merge 分配 + clone 入累积器
    /// + json! 共三份全文（≈3× 下界）；原地归并 + 单次克隆必须压在
    /// 1.5× 下界以内。
    #[test]
    fn bridge_snapshot_stream_allocates_one_frame_copy_per_chunk() {
        const CHUNKS: usize = 400;
        const CHUNK: &str = "abcdefghijklmnopqrstuvwxy"; // 25 bytes
        let mut truth = String::new();
        let events: Vec<FrontendEvent> = (0..CHUNKS)
            .map(|_| {
                truth.push_str(CHUNK);
                live_output_event(&truth, "full", LivePartPhase::Snapshot)
            })
            .collect();
        let inherent_floor: usize = (1..=CHUNKS).sum::<usize>() * CHUNK.len();

        let guard = crate::test_alloc::AllocGuard::start();
        let mut accum = HashMap::new();
        let mut last = None;
        for event in events {
            last = Some(super::coalesce_live_output_block(event, &mut accum));
        }
        let allocated = guard.bytes();
        drop(guard);

        assert_eq!(
            output_block_text(&last.expect("final frame")).map(|s| s.len()),
            Some(CHUNKS * CHUNK.len())
        );
        assert!(
            allocated < inherent_floor * 3 / 2,
            "snapshot coalescing must allocate ~1 frame copy per chunk \
             (allocated {allocated} bytes; inherent floor {inherent_floor} bytes; \
             old merge+clone+json! implementation ≈ 3× floor)"
        );
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
        panic!("direct bridge did not subscribe to frontend_bus in time");
    }

    /// direct bridge 的 session 过滤、事件顺序与事件内容在 typed 传输下
    /// 逐字节不变;其它 session 的事件在 borrowed 阶段即被过滤(零克隆)。
    #[tokio::test]
    async fn direct_event_loop_filters_session_and_preserves_order_and_content() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut rx = super::spawn_direct_event_loop(
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
                "direct bridge must not materialize JSON in-process"
            );
        }
        cancel.cancel();
    }

    /// 全总线变体不过滤 session,顺序与内容不变,且 live output 的
    /// coalesce 语义经 typed 传输后逐字节不变。
    #[tokio::test]
    async fn direct_event_bus_forwards_all_sessions_with_coalesce_intact() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut rx = super::spawn_direct_event_bus(Arc::clone(&state), cancel.clone());
        wait_bridge_subscribed(&state, receivers_before).await;

        let identity = || LiveMessagePartIdentity {
            message_id: "msg_1".to_string(),
            part_key: ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Append,
            legacy_block_id: None,
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
                "direct bridge must not materialize JSON in-process"
            );
        }
        cancel.cancel();
    }

    /// 客户端断开（rx 被 drop）后,bus bridge 任务必须在下一个事件到达时
    /// 退出并退订 frontend_bus,不再作为僵尸订阅累积。
    #[tokio::test]
    async fn direct_event_bus_task_exits_after_receiver_dropped() {
        let state = Arc::new(crate::ServerState::new());
        let receivers_before = state.frontend_bus.receiver_count();
        let cancel = tokio_util::sync::CancellationToken::new();
        let rx = super::spawn_direct_event_bus(Arc::clone(&state), cancel);
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
