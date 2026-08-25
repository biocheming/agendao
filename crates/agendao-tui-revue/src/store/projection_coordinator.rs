use agendao_server_core::SequencedServerEvent;
use agendao_types::SessionRuntimeSnapshot;
use std::collections::VecDeque;
use std::sync::RwLock;

use super::projection_reducer::{
    apply_event, ReduceResult, ResnapshotReason, SessionProjection, StreamEpoch,
};

/// 恢复事务状态机
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    Synced,
    Recovering {
        generation: u64,
        required_sequence: u64,
    },
    Degraded {
        error: String,
    },
}

/// 影子比对诊断报告
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowMismatchReport {
    pub session_id: String,
    pub epoch: StreamEpoch,
    pub sequence: u64,
    pub queue_revision: u64,
    pub shadow_queue_count: usize,
    pub legacy_queue_count: usize,
}

/// 投影协调器 (Projection Coordinator)
/// 保证：
/// 1. 恢复事务连续性（排序、去重、连续递增重放，无界缓冲上限防爆内存）；
/// 2. 真实链路 Shadow 比较与诊断（防崩溃，可观测比对统计）；
/// 3. 安全锁策略（锁内无 I/O，不跨 await，快速释放）。
pub struct ProjectionCoordinator {
    current_projection: RwLock<Option<SessionProjection>>,
    current_epoch: RwLock<StreamEpoch>,
    sync_state: RwLock<SyncState>,
    recovery_generation: RwLock<u64>,
    event_buffer_during_recovery: RwLock<VecDeque<SequencedServerEvent>>,
    shadow_comparisons_count: RwLock<u64>,
    shadow_mismatches: RwLock<Vec<ShadowMismatchReport>>,
}

impl Default for ProjectionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectionCoordinator {
    pub const MAX_RECOVERY_BUFFER_SIZE: usize = 1000;

    pub fn new() -> Self {
        Self {
            current_projection: RwLock::new(None),
            current_epoch: RwLock::new(1),
            sync_state: RwLock::new(SyncState::Synced),
            recovery_generation: RwLock::new(0),
            event_buffer_during_recovery: RwLock::new(VecDeque::new()),
            shadow_comparisons_count: RwLock::new(0),
            shadow_mismatches: RwLock::new(Vec::new()),
        }
    }

    /// 获取当前投影快照副本（供 UI 纯读渲染）
    pub fn projection(&self) -> Option<SessionProjection> {
        self.current_projection.read().ok().and_then(|g| g.clone())
    }

    /// 获取当前同步状态
    pub fn sync_state(&self) -> SyncState {
        self.sync_state
            .read()
            .map(|s| s.clone())
            .unwrap_or(SyncState::Synced)
    }

    /// 统计指标：比对次数
    pub fn comparison_stats(&self) -> (u64, usize) {
        let count = self
            .shadow_comparisons_count
            .read()
            .map(|c| *c)
            .unwrap_or(0);
        let mismatches = self.shadow_mismatches.read().map(|m| m.len()).unwrap_or(0);
        (count, mismatches)
    }

    /// 影子比对逻辑：比较 Shadow Projection 与旧 Store 的数值
    pub fn compare_shadow_with_legacy(&self, legacy_queue_count: usize) -> bool {
        if let Ok(mut c) = self.shadow_comparisons_count.write() {
            *c += 1;
        }

        let Some(proj) = self.projection() else {
            return true;
        };

        let shadow_count = proj.queued_inputs.len();
        if shadow_count != legacy_queue_count {
            if let Ok(mut mismatches) = self.shadow_mismatches.write() {
                mismatches.push(ShadowMismatchReport {
                    session_id: proj.session_id.clone(),
                    epoch: proj.version.stream_epoch,
                    sequence: proj.version.last_sequence,
                    queue_revision: proj.queue_revision,
                    shadow_queue_count: shadow_count,
                    legacy_queue_count,
                });
            }
            return false;
        }
        true
    }

    /// 同步重置当前会话投影
    pub fn reset_session(&self) {
        if let Ok(mut proj_guard) = self.current_projection.write() {
            *proj_guard = None;
        }
        if let Ok(mut state) = self.sync_state.write() {
            *state = SyncState::Synced;
        }
        if let Ok(mut buffer) = self.event_buffer_during_recovery.write() {
            buffer.clear();
        }
    }

    /// 开始恢复事务
    pub fn begin_recovery(&self, required_sequence: u64) -> u64 {
        let generation = if let Ok(mut gen) = self.recovery_generation.write() {
            *gen += 1;
            *gen
        } else {
            1
        };
        if let Ok(mut state) = self.sync_state.write() {
            *state = SyncState::Recovering {
                generation,
                required_sequence,
            };
        }
        if let Ok(mut buffer) = self.event_buffer_during_recovery.write() {
            buffer.clear();
        }
        generation
    }

    /// 完成恢复事务并严格连续重放
    pub fn finish_recovery(
        &self,
        generation: u64,
        snapshot: &SessionRuntimeSnapshot,
        epoch: StreamEpoch,
    ) -> bool {
        let current_gen = self.recovery_generation.read().map(|g| *g).unwrap_or(0);
        if generation != current_gen {
            return false;
        }

        let mut proj_guard = match self.current_projection.write() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if let Ok(mut epoch_guard) = self.current_epoch.write() {
            *epoch_guard = epoch;
        }

        let base_proj = SessionProjection::from_snapshot(snapshot, epoch);
        let mut current_p = base_proj;

        // 取出缓冲事件
        let mut events: Vec<SequencedServerEvent> =
            if let Ok(mut buffer) = self.event_buffer_during_recovery.write() {
                buffer.drain(..).collect()
            } else {
                Vec::new()
            };

        // 1. 过滤当前 session
        events.retain(|e| e.session_id == current_p.session_id);
        // 2. 按 sequence 升序排序
        events.sort_by_key(|e| e.sequence);
        // 3. 去重
        events.dedup_by_key(|e| e.sequence);

        // 4. 从 watermark + 1 严格连续重放
        for ev in events {
            if ev.sequence > current_p.version.last_sequence {
                match apply_event(&current_p, &ev, epoch) {
                    ReduceResult::Applied(next) => {
                        current_p = next;
                    }
                    ReduceResult::ResnapshotRequired { .. } => {
                        // 重放时再次发现 gap，保持 Recovering 状态触发新一轮拉取
                        *proj_guard = Some(current_p);
                        return false;
                    }
                    _ => {}
                }
            }
        }

        *proj_guard = Some(current_p);
        if let Ok(mut state) = self.sync_state.write() {
            *state = SyncState::Synced;
        }
        true
    }

    /// 基于全量快照直接初始化或重置
    pub fn install_snapshot(&self, snapshot: &SessionRuntimeSnapshot, epoch: StreamEpoch) {
        if let Ok(mut proj_guard) = self.current_projection.write() {
            if let Ok(mut epoch_guard) = self.current_epoch.write() {
                *epoch_guard = epoch;
            }
            *proj_guard = Some(SessionProjection::from_snapshot(snapshot, epoch));
        }
        if let Ok(mut state) = self.sync_state.write() {
            *state = SyncState::Synced;
        }
        if let Ok(mut buffer) = self.event_buffer_during_recovery.write() {
            buffer.clear();
        }
    }

    /// 处理顺序或乱序增量事件
    pub fn handle_event(&self, event: &SequencedServerEvent) -> ReduceResult {
        let state = self.sync_state();
        if let SyncState::Recovering { .. } = state {
            if let Ok(mut buffer) = self.event_buffer_during_recovery.write() {
                if buffer.len() < Self::MAX_RECOVERY_BUFFER_SIZE {
                    buffer.push_back(event.clone());
                }
            }
            return ReduceResult::DuplicateIgnored;
        }

        let mut proj_guard = match self.current_projection.write() {
            Ok(g) => g,
            Err(_) => {
                return ReduceResult::ResnapshotRequired {
                    reason: ResnapshotReason::UnsupportedEvent,
                }
            }
        };
        let epoch = self.current_epoch.read().map(|e| *e).unwrap_or(1);

        let Some(current) = proj_guard.as_ref() else {
            return ReduceResult::ResnapshotRequired {
                reason: ResnapshotReason::UnsupportedEvent,
            };
        };

        let result = apply_event(current, event, epoch);
        if let ReduceResult::Applied(ref next) = result {
            *proj_guard = Some(next.clone());
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_server_core::runtime_events::ServerEvent;
    use agendao_types::{ActiveTurnSnapshot, QueuedInputSnapshot};

    fn sample_snapshot(session_id: &str, last_seq: u64) -> SessionRuntimeSnapshot {
        SessionRuntimeSnapshot {
            session_id: session_id.to_string(),
            runtime_revision: 1,
            queue_revision: 1,
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
                content: "queued".into(),
                position: 0,
                created_at_ms: 1050,
            }],
            pending_steering: vec![],
            last_turn_outcome: None,
        }
    }

    #[test]
    fn test_shadow_comparison_and_continuous_replay() {
        let coordinator = ProjectionCoordinator::new();
        let snap = sample_snapshot("sess_1", 100);
        coordinator.install_snapshot(&snap, 1);

        // 1. 影子比对验证：初始排队数为 1
        assert!(coordinator.compare_shadow_with_legacy(1));
        let (comps, mismatches) = coordinator.comparison_stats();
        assert_eq!(comps, 1);
        assert_eq!(mismatches, 0);

        // 2. 模拟不一致（旧 store 传入 2，但 shadow 是 1）
        assert!(!coordinator.compare_shadow_with_legacy(2));
        let (comps2, mismatches2) = coordinator.comparison_stats();
        assert_eq!(comps2, 2);
        assert_eq!(mismatches2, 1);

        // 3. 连续重放：乱序有重叠事件入缓冲
        let gen = coordinator.begin_recovery(105);
        let ev103 = SequencedServerEvent::new(
            "sess_1",
            103,
            ServerEvent::QueueChanged {
                session_id: "sess_1".into(),
                queue_revision: 3,
                queued_count: 2,
            },
        );
        let ev102 = SequencedServerEvent::new(
            "sess_1",
            102,
            ServerEvent::QueueChanged {
                session_id: "sess_1".into(),
                queue_revision: 2,
                queued_count: 2,
            },
        );
        coordinator.handle_event(&ev103);
        coordinator.handle_event(&ev102);
        coordinator.handle_event(&ev102); // 重复事件

        // 安装快照 (watermark 101)，finish_recovery 会自动排序、去重、连续从 102 推进到 103
        let snap101 = sample_snapshot("sess_1", 101);
        let ok = coordinator.finish_recovery(gen, &snap101, 1);
        assert!(ok);

        let p = coordinator.projection().unwrap();
        assert_eq!(p.version.last_sequence, 103);
        assert_eq!(p.queue_revision, 3);
        assert_eq!(coordinator.sync_state(), SyncState::Synced);
    }
}
