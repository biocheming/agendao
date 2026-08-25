use serde::{Deserialize, Serialize};

/// 客户端提交模式 (Submission Mode)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionMode {
    /// 服务端原子判断：空闲则启动Turn，繁忙则挂入队列
    Auto,
    /// 明确要求启动新Turn（若当前已有活跃Turn或处于阻塞则报错拒绝）
    StartTurn,
    /// 明确要求进入下一轮会话队列
    Queue,
    /// 明确要求插话至指定Turn的安全边界（若Turn已结束则拒绝）
    Steer { expected_turn_id: String },
}

impl Default for SubmissionMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// 提交命令 payload (Submit Input Command)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitInputCommand {
    /// 客户端请求唯一幂等ID (UUID v4)
    pub client_request_id: String,
    /// 会话ID
    pub session_id: String,
    /// 提交模式
    #[serde(default)]
    pub mode: SubmissionMode,
    /// 提示词或文本内容
    pub content: String,
}

/// 提交拒绝原因
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionRejectionReason {
    TurnConflict {
        active_turn_id: String,
    },
    TargetTurnFinished {
        expected_turn_id: String,
    },
    SessionNotFound {
        session_id: String,
    },
    EmptyContent,
    QueueFull {
        limit: usize,
    },
    InvalidMode {
        detail: String,
    },
    IdempotencyPayloadMismatch {
        client_request_id: String,
    },
    QueueRevisionConflict {
        expected_revision: u64,
        current_revision: u64,
    },
    DuplicateRequestInProgress {
        client_request_id: String,
    },
    QueueItemNotFound {
        session_id: String,
        item_id: String,
    },
    QueuePositionOutOfRange {
        position: u32,
        queue_len: u32,
    },
}

/// 服务端原子分流响应 (Submission Disposition)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum SubmissionDisposition {
    /// 成功启动新Turn
    Started { turn_id: String, session_id: String },
    /// 成功进入下一轮排队
    Queued {
        item_id: String,
        session_id: String,
        position: u32,
        queue_revision: u64,
    },
    /// 成功注册为当前Turn的安全边界插话
    SteeringPending {
        steering_id: String,
        session_id: String,
        target_turn_id: String,
        pending_count: usize,
    },
    /// 请求被拒绝
    Rejected {
        reason: SubmissionRejectionReason,
        message: String,
    },
}

/// 乐观并发控制的队列变更请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueMutationRequest {
    #[serde(default)]
    pub client_request_id: String,
    pub session_id: String,
    pub item_id: String,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEditRequest {
    #[serde(default)]
    pub client_request_id: String,
    pub session_id: String,
    pub item_id: String,
    pub expected_revision: u64,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueReorderRequest {
    #[serde(default)]
    pub client_request_id: String,
    pub session_id: String,
    pub item_id: String,
    pub expected_revision: u64,
    pub new_position: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum QueueMutationDisposition {
    Applied {
        session_id: String,
        item_id: String,
        position: u32,
        queue_revision: u64,
    },
    Rejected {
        reason: SubmissionRejectionReason,
        message: String,
    },
}

// ── M1.5 权威快照与有序事件契约 ─────────────────────────────────────────────

/// 排队输入项快照
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedInputSnapshot {
    pub item_id: String,
    pub client_request_id: String,
    pub content: String,
    pub position: u32,
    pub created_at_ms: i64,
}

/// 待生效插话快照
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteeringSnapshot {
    pub steering_id: String,
    pub target_turn_id: String,
    pub content: String,
    pub deliver_at: String,
    pub enqueued_at_ms: i64,
}

/// 活跃Turn快照
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTurnSnapshot {
    pub turn_id: String,
    pub phase: String,
    pub started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker_id: Option<String>,
    #[serde(default)]
    pub interrupt_requested: bool,
}

/// 中断命令 payload (Interrupt Command)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptCommand {
    /// 客户端请求唯一幂等ID (UUID v4)
    pub client_request_id: String,
    /// 会话ID
    pub session_id: String,
    /// 期望中断的活跃 Turn ID
    pub expected_turn_id: String,
}

/// 中断命令回执 (Interrupt Disposition)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum InterruptDisposition {
    /// 中断已被接收并标记请求
    Interrupted { turn_id: String, session_id: String },
    /// 中断请求被拒绝
    Rejected { reason: String, session_id: String },
}

/// Turn 最终确定性结果
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed {
        completed_at_ms: i64,
    },
    Interrupted {
        interrupted_at_ms: i64,
        reason: Option<String>,
    },
    Failed {
        failed_at_ms: i64,
        error: String,
    },
}

/// 会话运行时权威快照（用于重连与全量对齐）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeSnapshot {
    pub session_id: String,
    pub runtime_revision: u64,
    pub queue_revision: u64,
    /// 水位线：该快照包含截至此 sequence 的所有事件
    pub last_event_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<ActiveTurnSnapshot>,
    pub queued_inputs: Vec<QueuedInputSnapshot>,
    pub pending_steering: Vec<SteeringSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<TurnOutcome>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submission_mode_serialization() {
        let auto = SubmissionMode::Auto;
        let json = serde_json::to_string(&auto).unwrap();
        assert_eq!(json, "\"auto\"");
        let deserialized: SubmissionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, SubmissionMode::Auto);

        let steer = SubmissionMode::Steer {
            expected_turn_id: "turn_123".to_string(),
        };
        let json = serde_json::to_string(&steer).unwrap();
        assert_eq!(json, "{\"steer\":{\"expected_turn_id\":\"turn_123\"}}");
    }

    #[test]
    fn test_submission_disposition_serialization() {
        let disposition = SubmissionDisposition::Queued {
            item_id: "q_1".into(),
            session_id: "sess_1".into(),
            position: 0,
            queue_revision: 42,
        };
        let json = serde_json::to_string(&disposition).unwrap();
        assert!(json.contains("\"disposition\":\"queued\""));
        assert!(json.contains("\"queue_revision\":42"));
    }

    #[test]
    fn test_session_runtime_snapshot_serialization() {
        let snapshot = SessionRuntimeSnapshot {
            session_id: "sess_1".into(),
            runtime_revision: 10,
            queue_revision: 2,
            last_event_sequence: 105,
            active_turn: Some(ActiveTurnSnapshot {
                turn_id: "turn_1".into(),
                phase: "running_tool".into(),
                started_at_ms: 1000,
                active_tool_call_id: Some("call_1".into()),
                blocker_id: None,
                interrupt_requested: false,
            }),
            queued_inputs: vec![QueuedInputSnapshot {
                item_id: "q_1".into(),
                client_request_id: "req_1".into(),
                content: "hello next turn".into(),
                position: 0,
                created_at_ms: 1050,
            }],
            pending_steering: vec![],
            last_turn_outcome: None,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"lastEventSequence\":105"));
        assert!(json.contains("\"queueRevision\":2"));
    }

    #[test]
    fn queue_mutation_requests_roundtrip_client_request_id() {
        let req = QueueEditRequest {
            client_request_id: "edit-1".into(),
            session_id: "s".into(),
            item_id: "q".into(),
            expected_revision: 3,
            content: "new".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("clientRequestId"));
        let decoded: QueueEditRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.client_request_id, "edit-1");
    }
}
