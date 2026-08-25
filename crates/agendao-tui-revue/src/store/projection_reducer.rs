use agendao_server_core::runtime_events::ServerEvent;
use agendao_server_core::SequencedServerEvent;
use agendao_types::{
    ActiveTurnSnapshot, QueuedInputSnapshot, SessionRuntimeSnapshot, SteeringSnapshot, TurnOutcome,
};

/// 流纪元标识（防止服务端重启后序列号回绕引发错乱）
pub type StreamEpoch = u64;
/// 单调递增事件序列号
pub type EventSequence = u64;
/// 队列版本号
pub type QueueRevision = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionVersion {
    pub stream_epoch: StreamEpoch,
    pub last_sequence: EventSequence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveTurnProjection {
    pub turn_id: String,
    pub phase: String,
    pub started_at_ms: i64,
    pub active_tool_call_id: Option<String>,
    pub blocker_id: Option<String>,
    pub interrupt_requested: bool,
}

impl From<ActiveTurnSnapshot> for ActiveTurnProjection {
    fn from(s: ActiveTurnSnapshot) -> Self {
        Self {
            turn_id: s.turn_id,
            phase: s.phase,
            started_at_ms: s.started_at_ms,
            active_tool_call_id: s.active_tool_call_id,
            blocker_id: s.blocker_id,
            interrupt_requested: s.interrupt_requested,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedInputProjection {
    pub item_id: String,
    pub client_request_id: String,
    pub content_preview: String,
    pub position: u32,
    pub created_at_ms: i64,
}

impl From<QueuedInputSnapshot> for QueuedInputProjection {
    fn from(s: QueuedInputSnapshot) -> Self {
        Self {
            item_id: s.item_id,
            client_request_id: s.client_request_id,
            content_preview: s.content,
            position: s.position,
            created_at_ms: s.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteeringProjection {
    pub steering_id: String,
    pub target_turn_id: String,
    pub content_preview: String,
    pub deliver_at: String,
    pub enqueued_at_ms: i64,
}

impl From<SteeringSnapshot> for SteeringProjection {
    fn from(s: SteeringSnapshot) -> Self {
        Self {
            steering_id: s.steering_id,
            target_turn_id: s.target_turn_id,
            content_preview: s.content,
            deliver_at: s.deliver_at,
            enqueued_at_ms: s.enqueued_at_ms,
        }
    }
}

/// 会话服务端事实投影 (SessionProjection)
/// 纯只读数据模型，仅由 Reducer 基于 Snapshot 和 Events 演进生成。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionProjection {
    pub session_id: String,
    pub version: ProjectionVersion,
    pub queue_revision: QueueRevision,
    pub active_turn: Option<ActiveTurnProjection>,
    pub queued_inputs: Vec<QueuedInputProjection>,
    pub pending_steering: Vec<SteeringProjection>,
    pub last_turn_outcome: Option<TurnOutcome>,
}

impl SessionProjection {
    /// 从全量快照构建初始投影
    pub fn from_snapshot(snapshot: &SessionRuntimeSnapshot, stream_epoch: StreamEpoch) -> Self {
        Self {
            session_id: snapshot.session_id.clone(),
            version: ProjectionVersion {
                stream_epoch,
                last_sequence: snapshot.last_event_sequence,
            },
            queue_revision: snapshot.queue_revision,
            active_turn: snapshot.active_turn.clone().map(Into::into),
            queued_inputs: snapshot
                .queued_inputs
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
            pending_steering: snapshot
                .pending_steering
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
            last_turn_outcome: snapshot.last_turn_outcome.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResnapshotReason {
    SequenceGap {
        expected: EventSequence,
        received: EventSequence,
    },
    StreamEpochChanged {
        current: StreamEpoch,
        received: StreamEpoch,
    },
    SessionIdMismatch {
        current: String,
        received: String,
    },
    QueueRevisionConflict,
    UnsupportedEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReduceResult {
    /// 成功应用新状态
    Applied(SessionProjection),
    /// 重复事件（幂等忽略，不改动状态）
    DuplicateIgnored,
    /// 收到陈旧快照（忽略）
    StaleSnapshotIgnored,
    /// 发生缺口或异常，协调器需触发快照重新拉取
    ResnapshotRequired { reason: ResnapshotReason },
}

/// 纯函数式 Reducer: apply_event
/// 先校验 session_id、epoch、sequence 与领域前置条件，全部合法后才生成新 SessionProjection。
pub fn apply_event(
    current: &SessionProjection,
    event: &SequencedServerEvent,
    event_epoch: StreamEpoch,
) -> ReduceResult {
    // 1. 基础一致性校验
    if event.session_id != current.session_id {
        return ReduceResult::ResnapshotRequired {
            reason: ResnapshotReason::SessionIdMismatch {
                current: current.session_id.clone(),
                received: event.session_id.clone(),
            },
        };
    }
    if event_epoch != current.version.stream_epoch {
        return ReduceResult::ResnapshotRequired {
            reason: ResnapshotReason::StreamEpochChanged {
                current: current.version.stream_epoch,
                received: event_epoch,
            },
        };
    }

    // 2. 序列号水位判断 (仅在相同 Epoch 下校验)
    if event.sequence <= current.version.last_sequence {
        return ReduceResult::DuplicateIgnored;
    }
    if event.sequence > current.version.last_sequence + 1 {
        return ReduceResult::ResnapshotRequired {
            reason: ResnapshotReason::SequenceGap {
                expected: current.version.last_sequence + 1,
                received: event.sequence,
            },
        };
    }

    // 3. 领域事件状态演进（原子复制并修改）
    let mut next = current.clone();
    next.version.last_sequence = event.sequence;

    match &event.event {
        ServerEvent::QueueChanged {
            queue_revision,
            queued_count: _,
            ..
        } => {
            next.queue_revision = *queue_revision;
        }
        ServerEvent::SteeringApplied {
            steering_id,
            target_turn_id: _,
            applied_at: _,
            ..
        } => {
            next.pending_steering
                .retain(|s| &s.steering_id != steering_id);
        }
        ServerEvent::SteeringRejected { steering_id, .. } => {
            next.pending_steering
                .retain(|s| &s.steering_id != steering_id);
        }
        _ => {
            // 其他事件不影响当前核心投影，正常推进 last_sequence
        }
    }

    ReduceResult::Applied(next)
}

/// 纯函数式 Reducer: apply_snapshot
/// 跨纪元规则：
/// - 若 snapshot_epoch > current.epoch: 允许原子替换，即使 sequence 更低（如服务端重启 sequence 归零）；
/// - 若 snapshot_epoch < current.epoch: 属于过时纪元快照，忽略；
/// - 若 snapshot_epoch == current.epoch: 仅当 snapshot.sequence >= current.sequence 时原子替换，否则忽略。
pub fn apply_snapshot(
    current: &SessionProjection,
    snapshot: &SessionRuntimeSnapshot,
    snapshot_epoch: StreamEpoch,
) -> ReduceResult {
    if snapshot.session_id != current.session_id {
        return ReduceResult::ResnapshotRequired {
            reason: ResnapshotReason::SessionIdMismatch {
                current: current.session_id.clone(),
                received: snapshot.session_id.clone(),
            },
        };
    }

    if snapshot_epoch < current.version.stream_epoch {
        return ReduceResult::StaleSnapshotIgnored;
    }

    if snapshot_epoch == current.version.stream_epoch
        && snapshot.last_event_sequence < current.version.last_sequence
    {
        return ReduceResult::StaleSnapshotIgnored;
    }

    // 原子替换
    let next = SessionProjection::from_snapshot(snapshot, snapshot_epoch);
    ReduceResult::Applied(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(session_id: &str, last_seq: u64, rev: u64) -> SessionRuntimeSnapshot {
        SessionRuntimeSnapshot {
            session_id: session_id.to_string(),
            runtime_revision: rev,
            queue_revision: rev,
            last_event_sequence: last_seq,
            active_turn: Some(ActiveTurnSnapshot {
                turn_id: "turn_1".into(),
                phase: "running".into(),
                started_at_ms: 1000,
                active_tool_call_id: None,
                blocker_id: None,
                interrupt_requested: false,
            }),
            queued_inputs: vec![QueuedInputSnapshot {
                item_id: "q_1".into(),
                client_request_id: "req_1".into(),
                content: "queued text".into(),
                position: 0,
                created_at_ms: 1050,
            }],
            pending_steering: vec![SteeringSnapshot {
                steering_id: "st_1".into(),
                target_turn_id: "turn_1".into(),
                content: "steer text".into(),
                deliver_at: "next_tool_boundary".into(),
                enqueued_at_ms: 1060,
            }],
            last_turn_outcome: None,
        }
    }

    #[test]
    fn test_snapshot_epoch_transition_rules() {
        // 当前为 Epoch 1, seq 1000
        let snap_epoch_1 = sample_snapshot("sess_1", 1000, 10);
        let p1 = SessionProjection::from_snapshot(&snap_epoch_1, 1);

        // 同一 Epoch，收到更老的快照 (seq 900) -> 忽略
        let stale_same_epoch = sample_snapshot("sess_1", 900, 9);
        assert_eq!(
            apply_snapshot(&p1, &stale_same_epoch, 1),
            ReduceResult::StaleSnapshotIgnored
        );

        // 来自更老 Epoch 0 的快照 -> 忽略
        let old_epoch_snap = sample_snapshot("sess_1", 2000, 20);
        assert_eq!(
            apply_snapshot(&p1, &old_epoch_snap, 0),
            ReduceResult::StaleSnapshotIgnored
        );

        // 服务端重启，Epoch 变成 2，sequence 归零重新从 1 开始 -> 成功替换！
        let new_epoch_restarted_snap = sample_snapshot("sess_1", 1, 1);
        let res_restarted = apply_snapshot(&p1, &new_epoch_restarted_snap, 2);
        if let ReduceResult::Applied(next_p) = res_restarted {
            assert_eq!(next_p.version.stream_epoch, 2);
            assert_eq!(next_p.version.last_sequence, 1);
        } else {
            panic!("Expected Applied on epoch increment");
        }
    }
}
