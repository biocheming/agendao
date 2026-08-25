//! # M4.2 Interaction Feedback & Read-Only Projection
//!
//! 只读反馈模型：
//! - `SubmissionDisposition` 是瞬态命令回执，不是长期状态；
//! - 长期状态由服务端的 Snapshot 与有序 Event 权威投影产生；
//! - 严格按稳定 ID (client_request_id, item_id, steering_id, turn_id) 跟踪与去重；
//! - 状态跃迁生成只读通知，重复事件/回执绝不产生重复提示；
//! - 跨 Session 隔离：旧 Session 的回执与事件不得更新当前 Session 视图；
//! - 纯函数状态投影，支持从 `SessionRuntimeSnapshot` 完整对齐重构。

use crate::command_gateway::{ClientRequestId, SessionId};
use crate::interaction_contract::TurnId;
use agendao_types::submission::{
    QueuedInputSnapshot, SessionRuntimeSnapshot, SubmissionDisposition, SubmissionRejectionReason,
    TurnOutcome,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub type QueueItemId = String;
pub type SteeringId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueMutationOperation {
    Delete,
    Edit,
    Move,
}

/// 排队项概要
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSummary {
    pub count: usize,
    pub queue_revision: u64,
    pub head_item: Option<QueuedInputSnapshot>,
    /// 当前会话已观测到的队列项（只读 UI 操作列表）。服务端快照可完整替换它；
    /// 瞬态 Queued 回执则追加最小身份项，绝不猜测服务端内容。
    #[serde(default)]
    pub items: Vec<QueuedInputSnapshot>,
}

/// 插话项概要
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteeringSummary {
    pub pending_count: usize,
    pub target_turn_id: Option<TurnId>,
    pub latest_steering_id: Option<SteeringId>,
}

/// 中断状态概要
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptSummary {
    pub interrupt_requested: bool,
    pub target_turn_id: Option<TurnId>,
    pub last_outcome: Option<TurnOutcome>,
}

/// 执行状态概要
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub active_turn_id: Option<TurnId>,
    pub phase: String,
    pub active_tool: Option<String>,
}

/// 待结算/回执中的命令项
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandStatus {
    InFlight,
    AcceptedStarted {
        turn_id: TurnId,
    },
    AcceptedQueued {
        item_id: QueueItemId,
        position: u32,
        queue_revision: u64,
    },
    AcceptedSteeringPending {
        steering_id: SteeringId,
        target_turn_id: TurnId,
    },
    SteeringApplied {
        steering_id: SteeringId,
    },
    SteeringRejected {
        steering_id: SteeringId,
        reason: String,
    },
    Rejected {
        reason: SubmissionRejectionReason,
        message: String,
    },
    TransportFailed {
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCommandView {
    pub client_request_id: ClientRequestId,
    pub session_id: SessionId,
    pub status: CommandStatus,
    pub updated_at_ms: i64,
}

/// 用户可见的只读状态跃迁通知（用于驱动轻量 Toast/提示，按状态变更产生）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackTransition {
    Started {
        client_request_id: ClientRequestId,
        turn_id: TurnId,
    },
    Queued {
        client_request_id: ClientRequestId,
        item_id: QueueItemId,
        position: u32,
    },
    SteeringPending {
        client_request_id: ClientRequestId,
        steering_id: SteeringId,
        target_turn: TurnId,
    },
    SteeringApplied {
        steering_id: SteeringId,
    },
    SteeringRejected {
        steering_id: SteeringId,
        reason: String,
    },
    InterruptRequested {
        turn_id: TurnId,
    },
    Interrupted {
        turn_id: TurnId,
    },
    Rejected {
        client_request_id: ClientRequestId,
        message: String,
    },
}

/// 只读交互反馈核心结构
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionFeedback {
    pub session_id: SessionId,
    pub pending_commands: HashMap<ClientRequestId, PendingCommandView>,
    pub queue_summary: QueueSummary,
    pub steering_summary: SteeringSummary,
    pub interrupt_summary: InterruptSummary,
    pub execution_summary: ExecutionSummary,
    /// 已观察到的状态指纹集合（用于幂等去重，防重复 Toast）
    pub observed_transitions: HashSet<String>,
}

impl InteractionFeedback {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            pending_commands: HashMap::new(),
            queue_summary: QueueSummary {
                count: 0,
                queue_revision: 0,
                head_item: None,
                items: Vec::new(),
            },
            steering_summary: SteeringSummary {
                pending_count: 0,
                target_turn_id: None,
                latest_steering_id: None,
            },
            interrupt_summary: InterruptSummary {
                interrupt_requested: false,
                target_turn_id: None,
                last_outcome: None,
            },
            execution_summary: ExecutionSummary {
                active_turn_id: None,
                phase: "idle".into(),
                active_tool: None,
            },
            observed_transitions: HashSet::new(),
        }
    }

    /// 从服务端全量权威快照对齐恢复
    pub fn apply_snapshot(&mut self, snapshot: &SessionRuntimeSnapshot) {
        if snapshot.session_id != self.session_id {
            return;
        }

        self.queue_summary = QueueSummary {
            count: snapshot.queued_inputs.len(),
            queue_revision: snapshot.queue_revision,
            head_item: snapshot.queued_inputs.first().cloned(),
            items: snapshot.queued_inputs.clone(),
        };

        let pending_steering_count = snapshot.pending_steering.len();
        let target_turn_id = snapshot
            .pending_steering
            .last()
            .map(|s| s.target_turn_id.clone());
        let latest_steering_id = snapshot
            .pending_steering
            .last()
            .map(|s| s.steering_id.clone());

        self.steering_summary = SteeringSummary {
            pending_count: pending_steering_count,
            target_turn_id,
            latest_steering_id,
        };

        if let Some(ref turn) = snapshot.active_turn {
            self.execution_summary = ExecutionSummary {
                active_turn_id: Some(turn.turn_id.clone()),
                phase: turn.phase.clone(),
                active_tool: turn.active_tool_call_id.clone(),
            };
            self.interrupt_summary.interrupt_requested = turn.interrupt_requested;
            self.interrupt_summary.target_turn_id = Some(turn.turn_id.clone());
        } else {
            self.execution_summary = ExecutionSummary {
                active_turn_id: None,
                phase: "idle".into(),
                active_tool: None,
            };
            self.interrupt_summary.interrupt_requested = false;
        }

        if let Some(ref outcome) = snapshot.last_turn_outcome {
            self.interrupt_summary.last_outcome = Some(outcome.clone());
        }
    }

    /// 应用瞬态命令回执，并返回是否有新的状态跃迁通知
    pub fn apply_submission_disposition(
        &mut self,
        session_id: &str,
        client_request_id: &str,
        disposition: &SubmissionDisposition,
        now_ms: i64,
    ) -> Option<FeedbackTransition> {
        // 旧 Session 回执直接忽略，不得污染当前 Session 反馈
        if self.session_id != session_id {
            return None;
        }

        let (status, transition) = match disposition {
            SubmissionDisposition::Started { turn_id, .. } => (
                CommandStatus::AcceptedStarted {
                    turn_id: turn_id.clone(),
                },
                FeedbackTransition::Started {
                    client_request_id: client_request_id.to_string(),
                    turn_id: turn_id.clone(),
                },
            ),
            SubmissionDisposition::Queued {
                item_id,
                position,
                queue_revision,
                session_id: _,
            } => (
                CommandStatus::AcceptedQueued {
                    item_id: item_id.clone(),
                    position: *position,
                    queue_revision: *queue_revision,
                },
                FeedbackTransition::Queued {
                    client_request_id: client_request_id.to_string(),
                    item_id: item_id.clone(),
                    position: *position,
                },
            ),
            SubmissionDisposition::SteeringPending {
                steering_id,
                target_turn_id,
                ..
            } => (
                CommandStatus::AcceptedSteeringPending {
                    steering_id: steering_id.clone(),
                    target_turn_id: target_turn_id.clone(),
                },
                FeedbackTransition::SteeringPending {
                    client_request_id: client_request_id.to_string(),
                    steering_id: steering_id.clone(),
                    target_turn: target_turn_id.clone(),
                },
            ),
            SubmissionDisposition::Rejected { reason, message } => (
                CommandStatus::Rejected {
                    reason: reason.clone(),
                    message: message.clone(),
                },
                FeedbackTransition::Rejected {
                    client_request_id: client_request_id.to_string(),
                    message: message.clone(),
                },
            ),
        };

        self.pending_commands.insert(
            client_request_id.to_string(),
            PendingCommandView {
                client_request_id: client_request_id.to_string(),
                session_id: session_id.to_string(),
                status,
                updated_at_ms: now_ms,
            },
        );

        if let SubmissionDisposition::Queued {
            item_id,
            position,
            queue_revision,
            session_id,
            ..
        } = disposition
        {
            let item = QueuedInputSnapshot {
                item_id: item_id.clone(),
                client_request_id: client_request_id.to_string(),
                content: String::new(),
                position: *position,
                created_at_ms: now_ms,
            };
            if let Some(existing) = self
                .queue_summary
                .items
                .iter_mut()
                .find(|existing| existing.item_id == *item_id)
            {
                *existing = item;
            } else {
                self.queue_summary.items.push(item);
            }
            self.queue_summary
                .items
                .sort_by_key(|queued| queued.position);
            for (idx, queued) in self.queue_summary.items.iter_mut().enumerate() {
                queued.position = idx as u32;
            }
            self.queue_summary.count = self.queue_summary.items.len();
            self.queue_summary.queue_revision = *queue_revision;
            self.queue_summary.head_item = self.queue_summary.items.first().cloned();
            debug_assert_eq!(session_id, &self.session_id);
        }

        // 幂等去重指纹计算
        let fingerprint = format!("{client_request_id}:{:?}", disposition);
        if self.observed_transitions.insert(fingerprint) {
            Some(transition)
        } else {
            None
        }
    }

    pub fn selected_queue_item(&self, index: usize) -> Option<&QueuedInputSnapshot> {
        self.queue_summary.items.get(index)
    }

    pub fn apply_queue_mutation(
        &mut self,
        _operation: QueueMutationOperation,
        disposition: &agendao_types::submission::QueueMutationDisposition,
    ) {
        let agendao_types::submission::QueueMutationDisposition::Applied {
            session_id,
            item_id,
            position,
            queue_revision,
            ..
        } = disposition
        else {
            return;
        };
        // An ack only proves that the authority accepted a mutation.  It does
        // not contain the complete queue and can arrive after a consumer pop
        // or a newer edit.  Never synthesize a QueueSummary here; the runtime
        // snapshot is the sole projection writer.
        let _ = (session_id, item_id, position, queue_revision);
    }

    /// 应用插话生效或被拒绝的事件
    pub fn apply_steering_resolution(
        &mut self,
        steering_id: &str,
        applied: bool,
        reason: Option<&str>,
    ) -> Option<FeedbackTransition> {
        let fingerprint = format!("steering_res:{steering_id}:{applied}");
        if !self.observed_transitions.insert(fingerprint) {
            return None;
        }

        if applied {
            Some(FeedbackTransition::SteeringApplied {
                steering_id: steering_id.to_string(),
            })
        } else {
            Some(FeedbackTransition::SteeringRejected {
                steering_id: steering_id.to_string(),
                reason: reason.unwrap_or("Unknown reason").to_string(),
            })
        }
    }

    /// 应用中断命令回执
    pub fn apply_interrupt_disposition(
        &mut self,
        session_id: &str,
        disposition: &agendao_types::submission::InterruptDisposition,
        _now_ms: i64,
    ) -> Option<FeedbackTransition> {
        if self.session_id != session_id {
            return None;
        }
        match disposition {
            agendao_types::submission::InterruptDisposition::Interrupted { turn_id, .. } => {
                self.apply_interrupt_event(turn_id, false)
            }
            agendao_types::submission::InterruptDisposition::Rejected { .. } => None,
        }
    }

    /// 应用中断状态事件
    pub fn apply_interrupt_event(
        &mut self,
        turn_id: &str,
        interrupted: bool,
    ) -> Option<FeedbackTransition> {
        let fingerprint = format!("interrupt:{turn_id}:{interrupted}");
        if !self.observed_transitions.insert(fingerprint) {
            return None;
        }

        if interrupted {
            self.interrupt_summary.interrupt_requested = false;
            self.interrupt_summary.last_outcome = Some(TurnOutcome::Interrupted {
                interrupted_at_ms: 0,
                reason: Some("User requested interrupt".into()),
            });
            Some(FeedbackTransition::Interrupted {
                turn_id: turn_id.to_string(),
            })
        } else {
            self.interrupt_summary.interrupt_requested = true;
            Some(FeedbackTransition::InterruptRequested {
                turn_id: turn_id.to_string(),
            })
        }
    }

    /// 格式化渲染 Prompt 上方的一行摘要 (e.g. "Running · 1 tool · Queue 2 · Steering pending")
    pub fn render_prompt_summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref _turn_id) = self.execution_summary.active_turn_id {
            if let Some(ref tool) = self.execution_summary.active_tool {
                parts.push(format!("Running ({tool})"));
            } else {
                parts.push(format!("Running ({})", self.execution_summary.phase));
            }
        } else {
            parts.push("Idle".to_string());
        }

        if self.queue_summary.count > 0 {
            parts.push(format!("Queue {}", self.queue_summary.count));
        }

        if self.steering_summary.pending_count > 0 {
            parts.push(format!(
                "Steering ({})",
                self.steering_summary.pending_count
            ));
        }

        if self.interrupt_summary.interrupt_requested {
            parts.push("Interrupting...".to_string());
        }

        parts.join(" · ")
    }

    /// 格式化渲染底部状态栏 (e.g. "Queued (2) · Steer pending · Interrupting")
    pub fn render_status_bar(&self) -> String {
        let mut parts = Vec::new();

        if self.queue_summary.count > 0 {
            parts.push(format!("Queued ({})", self.queue_summary.count));
        }

        if self.steering_summary.pending_count > 0 {
            parts.push("Steer pending".to_string());
        }

        if self.interrupt_summary.interrupt_requested {
            parts.push("Interrupting".to_string());
        }

        if parts.is_empty() {
            "Ready".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_types::submission::{ActiveTurnSnapshot, SteeringSnapshot};

    #[test]
    fn test_started_disposition_generates_single_transition() {
        let mut feedback = InteractionFeedback::new("s1".into());
        let disp = SubmissionDisposition::Started {
            turn_id: "t1".into(),
            session_id: "s1".into(),
        };

        // 第一次产生跃迁通知
        let trans1 = feedback.apply_submission_disposition("s1", "req_1", &disp, 100);
        assert_eq!(
            trans1,
            Some(FeedbackTransition::Started {
                client_request_id: "req_1".into(),
                turn_id: "t1".into(),
            })
        );

        // 重复到达的回执必须幂等去重，不重复通知
        let trans2 = feedback.apply_submission_disposition("s1", "req_1", &disp, 100);
        assert_eq!(trans2, None);
    }

    #[test]
    fn test_queued_disposition_tracks_item_and_position() {
        let mut feedback = InteractionFeedback::new("s1".into());
        let disp = SubmissionDisposition::Queued {
            item_id: "item_99".into(),
            session_id: "s1".into(),
            position: 2,
            queue_revision: 4,
        };

        let trans = feedback.apply_submission_disposition("s1", "req_2", &disp, 100);
        assert_eq!(
            trans,
            Some(FeedbackTransition::Queued {
                client_request_id: "req_2".into(),
                item_id: "item_99".into(),
                position: 2,
            })
        );

        let cmd = feedback.pending_commands.get("req_2").unwrap();
        assert_eq!(
            cmd.status,
            CommandStatus::AcceptedQueued {
                item_id: "item_99".into(),
                position: 2,
                queue_revision: 4,
            }
        );
    }

    #[test]
    fn test_cross_session_isolation() {
        let mut feedback = InteractionFeedback::new("s1".into());
        let disp = SubmissionDisposition::Started {
            turn_id: "t_other".into(),
            session_id: "s2".into(),
        };

        // 来自旧/其他 Session (s2) 的回执绝不更新 s1
        let trans = feedback.apply_submission_disposition("s2", "req_other", &disp, 100);
        assert_eq!(trans, None);
        assert!(feedback.pending_commands.is_empty());
    }

    #[test]
    fn test_steering_transitions_applied_and_rejected() {
        let mut feedback = InteractionFeedback::new("s1".into());

        let t1 = feedback.apply_steering_resolution("st_1", true, None);
        assert_eq!(
            t1,
            Some(FeedbackTransition::SteeringApplied {
                steering_id: "st_1".into()
            })
        );

        // 重复 resolution 不通知
        let t1_dup = feedback.apply_steering_resolution("st_1", true, None);
        assert_eq!(t1_dup, None);

        let t2 = feedback.apply_steering_resolution("st_2", false, Some("Target turn finished"));
        assert_eq!(
            t2,
            Some(FeedbackTransition::SteeringRejected {
                steering_id: "st_2".into(),
                reason: "Target turn finished".into(),
            })
        );
    }

    #[test]
    fn test_interrupt_lifecycle_and_outcome() {
        let mut feedback = InteractionFeedback::new("s1".into());

        let t1 = feedback.apply_interrupt_event("turn_active", false);
        assert_eq!(
            t1,
            Some(FeedbackTransition::InterruptRequested {
                turn_id: "turn_active".into()
            })
        );
        assert!(feedback.interrupt_summary.interrupt_requested);

        let t2 = feedback.apply_interrupt_event("turn_active", true);
        assert_eq!(
            t2,
            Some(FeedbackTransition::Interrupted {
                turn_id: "turn_active".into()
            })
        );
        assert!(!feedback.interrupt_summary.interrupt_requested);
        assert!(matches!(
            feedback.interrupt_summary.last_outcome,
            Some(TurnOutcome::Interrupted { .. })
        ));
    }

    #[test]
    fn test_reconnect_snapshot_rebuilds_state() {
        let mut feedback = InteractionFeedback::new("sess_1".into());
        let snapshot = SessionRuntimeSnapshot {
            session_id: "sess_1".into(),
            runtime_revision: 10,
            queue_revision: 2,
            last_event_sequence: 105,
            active_turn: Some(ActiveTurnSnapshot {
                turn_id: "turn_1".into(),
                phase: "executing".into(),
                started_at_ms: 1000,
                active_tool_call_id: Some("bash".into()),
                blocker_id: None,
                interrupt_requested: false,
            }),
            queued_inputs: vec![QueuedInputSnapshot {
                item_id: "q_1".into(),
                client_request_id: "req_1".into(),
                content: "queued msg".into(),
                position: 0,
                created_at_ms: 1050,
            }],
            pending_steering: vec![SteeringSnapshot {
                steering_id: "st_1".into(),
                target_turn_id: "turn_1".into(),
                content: "steer msg".into(),
                deliver_at: "tool_boundary".into(),
                enqueued_at_ms: 1060,
            }],
            last_turn_outcome: None,
        };

        feedback.apply_snapshot(&snapshot);

        assert_eq!(feedback.queue_summary.count, 1);
        assert_eq!(feedback.steering_summary.pending_count, 1);
        assert_eq!(
            feedback.execution_summary.active_turn_id,
            Some("turn_1".into())
        );
        assert_eq!(feedback.execution_summary.active_tool, Some("bash".into()));

        // 验证摘要渲染
        assert_eq!(
            feedback.render_prompt_summary(),
            "Running (bash) · Queue 1 · Steering (1)"
        );
        assert_eq!(feedback.render_status_bar(), "Queued (1) · Steer pending");
    }

    #[test]
    fn test_queue_mutation_applies_and_stale_rejection_preserves_summary() {
        let mut feedback = InteractionFeedback::new("s1".into());
        feedback.apply_snapshot(&SessionRuntimeSnapshot {
            session_id: "s1".into(),
            runtime_revision: 3,
            queue_revision: 3,
            last_event_sequence: 3,
            active_turn: None,
            queued_inputs: vec![
                QueuedInputSnapshot {
                    item_id: "q1".into(),
                    client_request_id: "r1".into(),
                    content: "one".into(),
                    position: 0,
                    created_at_ms: 1,
                },
                QueuedInputSnapshot {
                    item_id: "q2".into(),
                    client_request_id: "r2".into(),
                    content: "two".into(),
                    position: 1,
                    created_at_ms: 2,
                },
            ],
            pending_steering: vec![],
            last_turn_outcome: None,
        });
        feedback.apply_queue_mutation(
            QueueMutationOperation::Move,
            &agendao_types::submission::QueueMutationDisposition::Applied {
                session_id: "s1".into(),
                item_id: "q2".into(),
                position: 0,
                queue_revision: 4,
            },
        );
        // Mutation acks are incomplete and may race a consumer pop.  They
        // are never allowed to synthesize the queue projection.
        assert_eq!(feedback.queue_summary.items[0].item_id, "q1");
        assert_eq!(feedback.queue_summary.queue_revision, 3);
        let before = feedback.queue_summary.items.clone();

        feedback.apply_queue_mutation(
            QueueMutationOperation::Delete,
            &agendao_types::submission::QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::QueueRevisionConflict {
                    expected_revision: 3,
                    current_revision: 4,
                },
                message: "stale revision".into(),
            },
        );
        assert_eq!(feedback.queue_summary.items, before);
        assert_eq!(feedback.queue_summary.queue_revision, 3);

        feedback.apply_snapshot(&SessionRuntimeSnapshot {
            session_id: "s1".into(),
            runtime_revision: 4,
            queue_revision: 4,
            last_event_sequence: 4,
            active_turn: None,
            queued_inputs: vec![
                QueuedInputSnapshot {
                    item_id: "q2".into(),
                    client_request_id: "r2".into(),
                    content: "two".into(),
                    position: 0,
                    created_at_ms: 2,
                },
                QueuedInputSnapshot {
                    item_id: "q1".into(),
                    client_request_id: "r1".into(),
                    content: "one".into(),
                    position: 1,
                    created_at_ms: 1,
                },
            ],
            pending_steering: vec![],
            last_turn_outcome: None,
        });
        assert_eq!(feedback.queue_summary.items[0].item_id, "q2");
        assert_eq!(feedback.queue_summary.queue_revision, 4);

        let before = feedback.queue_summary.items.clone();

        // Cross-session, stale and malformed Applied receipts are untrusted
        // input: each must leave the current projection byte-for-byte intact.
        for disposition in [
            agendao_types::submission::QueueMutationDisposition::Applied {
                session_id: "other".into(),
                item_id: "q2".into(),
                position: 1,
                queue_revision: 5,
            },
            agendao_types::submission::QueueMutationDisposition::Applied {
                session_id: "s1".into(),
                item_id: "q2".into(),
                position: 1,
                queue_revision: 3,
            },
            agendao_types::submission::QueueMutationDisposition::Applied {
                session_id: "s1".into(),
                item_id: "q2".into(),
                position: 99,
                queue_revision: 5,
            },
        ] {
            feedback.apply_queue_mutation(QueueMutationOperation::Move, &disposition);
            assert_eq!(feedback.queue_summary.items, before);
            assert_eq!(feedback.queue_summary.queue_revision, 4);
        }
    }
}
