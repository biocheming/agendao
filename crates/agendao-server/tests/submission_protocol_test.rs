use agendao_server::ServerState;
use agendao_server_core::runtime_events::ServerEvent;
use agendao_server_core::submission_authority::SubmissionAuthority;
use agendao_server_core::SequencedServerEvent;
use agendao_types::{
    ActiveTurnSnapshot, SteeringSnapshot, SubmissionDisposition, SubmissionRejectionReason,
};
use std::sync::Arc;

#[tokio::test]
async fn test_submission_authority_scoped_idempotency_and_mismatch() {
    let state = Arc::new(ServerState::new());
    let session_1 = "sess_test_1";
    let session_2 = "sess_test_2";
    let req_id = "req_uuid_123";

    let hash_a = SubmissionAuthority::hash_payload("payload A");
    let hash_b = SubmissionAuthority::hash_payload("payload B");

    // 第一次检查：未处理
    let cached = state
        .submission_authority
        .check_idempotent(session_1, req_id, hash_a)
        .await;
    assert_eq!(cached, Ok(None));

    // 模拟入队
    let (item_id, pos, rev) = state
        .submission_authority
        .enqueue_prompt(session_1, req_id.into(), "payload A".into())
        .await;

    let disposition = SubmissionDisposition::Queued {
        item_id: item_id.clone(),
        session_id: session_1.to_string(),
        position: pos,
        queue_revision: rev,
    };

    state
        .submission_authority
        .record_disposition(session_1, req_id, hash_a, disposition.clone())
        .await;

    // 相同 session + 相同 req_id + 相同 payload -> 幂等命中
    let cached2 = state
        .submission_authority
        .check_idempotent(session_1, req_id, hash_a)
        .await;
    assert_eq!(cached2, Ok(Some(disposition.clone())));

    // 相同 session + 相同 req_id + 不同 payload -> 拒绝并报错
    let mismatch = state
        .submission_authority
        .check_idempotent(session_1, req_id, hash_b)
        .await;
    assert!(matches!(
        mismatch,
        Err(SubmissionRejectionReason::IdempotencyPayloadMismatch { .. })
    ));

    // 不同 session + 相同 req_id -> 互相隔离
    let other = state
        .submission_authority
        .check_idempotent(session_2, req_id, hash_a)
        .await;
    assert_eq!(other, Ok(None));
}

#[tokio::test]
async fn test_runtime_snapshot_and_event_sequence_property() {
    let state = Arc::new(ServerState::new());
    let session_id = "sess_property_test";

    // 产生排队
    let (q_id, pos, rev) = state
        .submission_authority
        .enqueue_prompt(session_id, "req_100".into(), "hello queued".into())
        .await;
    assert_eq!(pos, 0);
    assert_eq!(rev, 1);

    // 产生单调递增事件序列
    let seq_1 = state.submission_authority.next_sequence();
    let seq_2 = state.submission_authority.next_sequence();
    assert!(seq_2 > seq_1);

    let _event_1 = SequencedServerEvent::new(
        session_id,
        seq_1,
        ServerEvent::QueueChanged {
            session_id: session_id.into(),
            queue_revision: rev,
            queued_count: 1,
        },
    );

    // 构建全量权威快照
    let snapshot = state
        .submission_authority
        .build_runtime_snapshot(
            session_id,
            Some(ActiveTurnSnapshot {
                turn_id: "turn_active_1".into(),
                phase: "streaming".into(),
                started_at_ms: 5000,
                active_tool_call_id: None,
                blocker_id: None,
                interrupt_requested: false,
            }),
            vec![SteeringSnapshot {
                steering_id: "steer_1".into(),
                target_turn_id: "turn_active_1".into(),
                content: "steer note".into(),
                deliver_at: "next_tool_boundary".into(),
                enqueued_at_ms: 5100,
            }],
            None,
        )
        .await;

    assert_eq!(snapshot.session_id, session_id);
    assert_eq!(snapshot.queue_revision, 1);
    assert_eq!(snapshot.queued_inputs.len(), 1);
    assert_eq!(snapshot.queued_inputs[0].item_id, q_id);
    assert_eq!(snapshot.pending_steering.len(), 1);
    assert!(snapshot.last_event_sequence >= seq_2);
}
