use agendao_types::{
    ActiveTurnSnapshot, QueueMutationDisposition, QueuedInputSnapshot, SessionRuntimeSnapshot,
    SteeringSnapshot, SubmissionDisposition, SubmissionRejectionReason, TurnOutcome,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

type IdempotencyCache = HashMap<(String, String), (i64, u64, SubmissionDisposition)>;

/// 队列中单项的权威存储模型
#[derive(Clone, Debug)]
pub struct QueuedItem {
    pub id: String,
    pub client_request_id: String,
    pub session_id: String,
    pub content: String,
    pub payload_hash: u64,
    pub created_at_ms: i64,
}

/// 提交权威控制器 (Submission Authority)
/// 保证：
/// 1. 幂等去重：(session_id, client_request_id) 作为主键，校验 payload 一致性；
/// 2. Queue Revision 单调递增与冲突检测；
/// 3. 事件序列流水号 (Monotonic Sequence) 与快照水位线。
#[derive(Debug)]
pub struct SubmissionAuthority {
    /// 幂等缓存：(session_id, client_request_id) -> (created_at, payload_hash, disposition)
    idempotency_cache: Mutex<IdempotencyCache>,
    /// Queue writes use a separate disposition domain but the same scoped
    /// idempotency contract: (session, request id, payload hash).
    queue_mutation_cache: Mutex<HashMap<(String, String), (u64, QueueMutationDisposition)>>,
    queue_mutation_lock: Mutex<()>,
    /// 会话排队列表：session_id -> items
    queued_prompts: Mutex<HashMap<String, Vec<QueuedItem>>>,
    /// 全局或按会话 revision
    queue_revisions: Mutex<HashMap<String, u64>>,
    /// 单调递增事件序列号
    sequence_counter: AtomicU64,
}

impl Default for SubmissionAuthority {
    fn default() -> Self {
        Self::new()
    }
}

impl SubmissionAuthority {
    pub fn new() -> Self {
        Self {
            idempotency_cache: Mutex::new(HashMap::new()),
            queue_mutation_cache: Mutex::new(HashMap::new()),
            queue_mutation_lock: Mutex::new(()),
            queued_prompts: Mutex::new(HashMap::new()),
            queue_revisions: Mutex::new(HashMap::new()),
            sequence_counter: AtomicU64::new(1),
        }
    }

    /// 简单的 payload 签名计算
    pub fn hash_payload(content: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// 获取下个全局单调递增序列号
    pub fn next_sequence(&self) -> u64 {
        self.sequence_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// 当前最高序列号
    pub fn current_sequence(&self) -> u64 {
        self.sequence_counter.load(Ordering::Relaxed)
    }

    /// 获取会话当前的 queue revision
    pub async fn get_queue_revision(&self, session_id: &str) -> u64 {
        let revs = self.queue_revisions.lock().await;
        revs.get(session_id).copied().unwrap_or(0)
    }

    /// 幂等检查：
    /// - 若不存在记录：返回 None（可继续处理）
    /// - 若存在记录且 payload_hash 一致：返回 Ok(Some(disposition))（幂等命中）
    /// - 若存在记录但 payload_hash 不一致：返回 Err(Rejection)（拒绝非法复用ID）
    pub async fn check_idempotent(
        &self,
        session_id: &str,
        client_request_id: &str,
        payload_hash: u64,
    ) -> Result<Option<SubmissionDisposition>, SubmissionRejectionReason> {
        let cache = self.idempotency_cache.lock().await;
        let key = (session_id.to_string(), client_request_id.to_string());
        if let Some((_, cached_hash, disp)) = cache.get(&key) {
            if *cached_hash == payload_hash {
                Ok(Some(disp.clone()))
            } else {
                Err(SubmissionRejectionReason::IdempotencyPayloadMismatch {
                    client_request_id: client_request_id.to_string(),
                })
            }
        } else {
            Ok(None)
        }
    }

    /// 记录幂等结果
    pub async fn record_disposition(
        &self,
        session_id: &str,
        client_request_id: &str,
        payload_hash: u64,
        disposition: SubmissionDisposition,
    ) {
        let mut cache = self.idempotency_cache.lock().await;
        let now = chrono::Utc::now().timestamp_millis();
        let key = (session_id.to_string(), client_request_id.to_string());
        cache.insert(key, (now, payload_hash, disposition));
    }

    /// 进入排队并递增 revision
    pub async fn enqueue_prompt(
        &self,
        session_id: &str,
        client_request_id: String,
        content: String,
    ) -> (String, u32, u64) {
        let mut queues = self.queued_prompts.lock().await;
        let mut revs = self.queue_revisions.lock().await;

        let queue = queues.entry(session_id.to_string()).or_default();
        let item_id = format!("q_{}", uuid::Uuid::new_v4().simple());
        let position = queue.len() as u32;
        let now = chrono::Utc::now().timestamp_millis();
        let payload_hash = Self::hash_payload(&content);

        queue.push(QueuedItem {
            id: item_id.clone(),
            client_request_id,
            session_id: session_id.to_string(),
            content,
            payload_hash,
            created_at_ms: now,
        });

        let new_rev = revs.entry(session_id.to_string()).or_insert(0);
        *new_rev += 1;
        let revision = *new_rev;

        (item_id, position, revision)
    }

    /// 取出会话中的下一个排队项 (FIFO)
    pub async fn pop_queued_prompt(&self, session_id: &str) -> Option<(QueuedItem, u64)> {
        let mut queues = self.queued_prompts.lock().await;
        let mut revs = self.queue_revisions.lock().await;

        let queue = queues.get_mut(session_id)?;
        if queue.is_empty() {
            return None;
        }

        let item = queue.remove(0);
        let new_rev = revs.entry(session_id.to_string()).or_insert(0);
        *new_rev += 1;
        let revision = *new_rev;

        Some((item, revision))
    }

    /// Remove one queued item under the session's optimistic-concurrency
    /// revision.  Queue item IDs are scoped to the owning session: an ID from
    /// another session is indistinguishable from a missing item.
    pub async fn remove_queued_prompt(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
    ) -> Result<(u32, u64), SubmissionRejectionReason> {
        let mut queues = self.queued_prompts.lock().await;
        let mut revs = self.queue_revisions.lock().await;
        let current_revision = revs.get(session_id).copied().unwrap_or(0);
        if expected_revision != current_revision {
            return Err(SubmissionRejectionReason::QueueRevisionConflict {
                expected_revision,
                current_revision,
            });
        }

        let queue = queues.get_mut(session_id).ok_or_else(|| {
            SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            }
        })?;
        let position = queue
            .iter()
            .position(|item| item.id == item_id && item.session_id == session_id)
            .ok_or_else(|| SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            })?;
        queue.remove(position);
        let revision = revs.entry(session_id.to_string()).or_insert(0);
        *revision += 1;
        Ok((position as u32, *revision))
    }

    /// Edit one queued item under the session's optimistic-concurrency
    /// revision. Empty queue content is rejected before any state changes.
    pub async fn edit_queued_prompt(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
        content: String,
    ) -> Result<(u32, u64), SubmissionRejectionReason> {
        if content.trim().is_empty() {
            return Err(SubmissionRejectionReason::EmptyContent);
        }

        let mut queues = self.queued_prompts.lock().await;
        let mut revs = self.queue_revisions.lock().await;
        let current_revision = revs.get(session_id).copied().unwrap_or(0);
        if expected_revision != current_revision {
            return Err(SubmissionRejectionReason::QueueRevisionConflict {
                expected_revision,
                current_revision,
            });
        }

        let queue = queues.get_mut(session_id).ok_or_else(|| {
            SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            }
        })?;
        let position = queue
            .iter()
            .position(|item| item.id == item_id && item.session_id == session_id)
            .ok_or_else(|| SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            })?;
        let item = &mut queue[position];
        item.payload_hash = Self::hash_payload(&content);
        item.content = content;
        let revision = revs.entry(session_id.to_string()).or_insert(0);
        *revision += 1;
        Ok((position as u32, *revision))
    }

    pub async fn edit_queued_prompt_idempotent(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
        content: String,
        client_request_id: &str,
    ) -> QueueMutationDisposition {
        let _serial = self.queue_mutation_lock.lock().await;
        if client_request_id.trim().is_empty() {
            return QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                    client_request_id: "missing queue client_request_id".into(),
                },
                message: "queue mutation rejected: missing client_request_id".into(),
            };
        }
        let payload_hash =
            Self::hash_payload(&format!("edit:{item_id}:{expected_revision}:{content}"));
        let key = (session_id.to_string(), client_request_id.to_string());
        {
            let cache = self.queue_mutation_cache.lock().await;
            if let Some((known_hash, disposition)) = cache.get(&key) {
                return if *known_hash == payload_hash {
                    disposition.clone()
                } else {
                    QueueMutationDisposition::Rejected {
                        reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                            client_request_id: client_request_id.into(),
                        },
                        message: "queue mutation rejected: idempotency payload mismatch".into(),
                    }
                };
            }
        }
        let disposition = match self
            .edit_queued_prompt(session_id, item_id, expected_revision, content)
            .await
        {
            Ok((position, queue_revision)) => QueueMutationDisposition::Applied {
                session_id: session_id.into(),
                item_id: item_id.into(),
                position,
                queue_revision,
            },
            Err(reason) => QueueMutationDisposition::Rejected {
                message: format!("queue mutation rejected: {reason:?}"),
                reason,
            },
        };
        self.queue_mutation_cache
            .lock()
            .await
            .insert(key, (payload_hash, disposition.clone()));
        disposition
    }

    pub async fn remove_queued_prompt_idempotent(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
        client_request_id: &str,
    ) -> QueueMutationDisposition {
        let _serial = self.queue_mutation_lock.lock().await;
        if client_request_id.trim().is_empty() {
            return QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                    client_request_id: "missing queue client_request_id".into(),
                },
                message: "queue mutation rejected: missing client_request_id".into(),
            };
        }
        let hash = Self::hash_payload(&format!("delete:{item_id}:{expected_revision}"));
        let key = (session_id.into(), client_request_id.into());
        {
            let cache = self.queue_mutation_cache.lock().await;
            if let Some((h, d)) = cache.get(&key) {
                return if *h == hash {
                    d.clone()
                } else {
                    QueueMutationDisposition::Rejected {
                        reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                            client_request_id: client_request_id.into(),
                        },
                        message: "queue mutation rejected: idempotency payload mismatch".into(),
                    }
                };
            }
        }
        let d = match self
            .remove_queued_prompt(session_id, item_id, expected_revision)
            .await
        {
            Ok((position, queue_revision)) => QueueMutationDisposition::Applied {
                session_id: session_id.into(),
                item_id: item_id.into(),
                position,
                queue_revision,
            },
            Err(reason) => QueueMutationDisposition::Rejected {
                message: format!("queue mutation rejected: {reason:?}"),
                reason,
            },
        };
        self.queue_mutation_cache
            .lock()
            .await
            .insert(key, (hash, d.clone()));
        d
    }

    pub async fn reorder_queued_prompt_idempotent(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
        new_position: u32,
        client_request_id: &str,
    ) -> QueueMutationDisposition {
        let _serial = self.queue_mutation_lock.lock().await;
        if client_request_id.trim().is_empty() {
            return QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                    client_request_id: "missing queue client_request_id".into(),
                },
                message: "queue mutation rejected: missing client_request_id".into(),
            };
        }
        let hash = Self::hash_payload(&format!(
            "move:{item_id}:{expected_revision}:{new_position}"
        ));
        let key = (session_id.into(), client_request_id.into());
        {
            let cache = self.queue_mutation_cache.lock().await;
            if let Some((h, d)) = cache.get(&key) {
                return if *h == hash {
                    d.clone()
                } else {
                    QueueMutationDisposition::Rejected {
                        reason: SubmissionRejectionReason::IdempotencyPayloadMismatch {
                            client_request_id: client_request_id.into(),
                        },
                        message: "queue mutation rejected: idempotency payload mismatch".into(),
                    }
                };
            }
        }
        let d = match self
            .reorder_queued_prompt(session_id, item_id, expected_revision, new_position)
            .await
        {
            Ok((position, queue_revision)) => QueueMutationDisposition::Applied {
                session_id: session_id.into(),
                item_id: item_id.into(),
                position,
                queue_revision,
            },
            Err(reason) => QueueMutationDisposition::Rejected {
                message: format!("queue mutation rejected: {reason:?}"),
                reason,
            },
        };
        self.queue_mutation_cache
            .lock()
            .await
            .insert(key, (hash, d.clone()));
        d
    }

    /// Move one queued item to an index in the resulting queue.  The target
    /// index must be an existing queue position (`0..queue_len`).
    pub async fn reorder_queued_prompt(
        &self,
        session_id: &str,
        item_id: &str,
        expected_revision: u64,
        new_position: u32,
    ) -> Result<(u32, u64), SubmissionRejectionReason> {
        let mut queues = self.queued_prompts.lock().await;
        let mut revs = self.queue_revisions.lock().await;
        let current_revision = revs.get(session_id).copied().unwrap_or(0);
        if expected_revision != current_revision {
            return Err(SubmissionRejectionReason::QueueRevisionConflict {
                expected_revision,
                current_revision,
            });
        }

        let queue = queues.get_mut(session_id).ok_or_else(|| {
            SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            }
        })?;
        let queue_len = queue.len() as u32;
        if new_position >= queue_len {
            return Err(SubmissionRejectionReason::QueuePositionOutOfRange {
                position: new_position,
                queue_len,
            });
        }
        let position = queue
            .iter()
            .position(|item| item.id == item_id && item.session_id == session_id)
            .ok_or_else(|| SubmissionRejectionReason::QueueItemNotFound {
                session_id: session_id.to_string(),
                item_id: item_id.to_string(),
            })?;
        if position == new_position as usize {
            return Ok((new_position, current_revision));
        }
        let item = queue.remove(position);
        queue.insert(new_position as usize, item);
        let revision = revs.entry(session_id.to_string()).or_insert(0);
        *revision += 1;
        Ok((new_position, *revision))
    }

    /// 查看排队快照列表
    pub async fn list_queued_snapshots(&self, session_id: &str) -> Vec<QueuedInputSnapshot> {
        let queues = self.queued_prompts.lock().await;
        queues
            .get(session_id)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(idx, item)| QueuedInputSnapshot {
                        item_id: item.id.clone(),
                        client_request_id: item.client_request_id.clone(),
                        content: item.content.clone(),
                        position: idx as u32,
                        created_at_ms: item.created_at_ms,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 生成权威会话快照
    pub async fn build_runtime_snapshot(
        &self,
        session_id: &str,
        active_turn: Option<ActiveTurnSnapshot>,
        pending_steering: Vec<SteeringSnapshot>,
        last_turn_outcome: Option<TurnOutcome>,
    ) -> SessionRuntimeSnapshot {
        let queue_revision = self.get_queue_revision(session_id).await;
        let queued_inputs = self.list_queued_snapshots(session_id).await;
        let last_event_sequence = self.current_sequence();

        SessionRuntimeSnapshot {
            session_id: session_id.to_string(),
            runtime_revision: queue_revision, // 或结合其它子系统版本
            queue_revision,
            last_event_sequence,
            active_turn,
            queued_inputs,
            pending_steering,
            last_turn_outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scoped_idempotency_and_mismatch_rejection() {
        let authority = SubmissionAuthority::new();
        let session_1 = "sess_1";
        let session_2 = "sess_2";
        let req_id = "req_shared_123";

        let hash_a = SubmissionAuthority::hash_payload("payload A");
        let hash_b = SubmissionAuthority::hash_payload("payload B");

        // 首次检查：None
        let res = authority.check_idempotent(session_1, req_id, hash_a).await;
        assert_eq!(res, Ok(None));

        // 记录 session 1 的结果
        let disp = SubmissionDisposition::Started {
            turn_id: "turn_1".into(),
            session_id: session_1.into(),
        };
        authority
            .record_disposition(session_1, req_id, hash_a, disp.clone())
            .await;

        // 相同 session + 相同 req_id + 相同 payload -> 命中
        let hit = authority.check_idempotent(session_1, req_id, hash_a).await;
        assert_eq!(hit, Ok(Some(disp)));

        // 相同 session + 相同 req_id + 不同 payload -> 拒绝
        let mismatch = authority.check_idempotent(session_1, req_id, hash_b).await;
        assert!(matches!(
            mismatch,
            Err(SubmissionRejectionReason::IdempotencyPayloadMismatch { .. })
        ));

        // 不同 session + 相同 req_id -> 隔离独立（未命中）
        let other_session = authority.check_idempotent(session_2, req_id, hash_a).await;
        assert_eq!(other_session, Ok(None));
    }

    #[tokio::test]
    async fn test_queue_mutations_use_revision_cas_and_session_scoping() {
        let authority = SubmissionAuthority::new();
        let (first_id, _, rev1) = authority
            .enqueue_prompt("session-a", "req-a".into(), "first".into())
            .await;
        let (second_id, _, rev2) = authority
            .enqueue_prompt("session-a", "req-b".into(), "second".into())
            .await;
        assert_eq!(rev1, 1);
        assert_eq!(rev2, 2);

        let stale = authority
            .remove_queued_prompt("session-a", &first_id, rev1)
            .await;
        assert!(matches!(
            stale,
            Err(SubmissionRejectionReason::QueueRevisionConflict {
                expected_revision: 1,
                current_revision: 2
            })
        ));

        let wrong_session = authority
            .remove_queued_prompt("session-b", &second_id, 0)
            .await;
        assert!(matches!(
            wrong_session,
            Err(SubmissionRejectionReason::QueueItemNotFound { .. })
        ));

        let (position, rev3) = authority
            .edit_queued_prompt("session-a", &first_id, rev2, "updated".into())
            .await
            .expect("edit should apply");
        assert_eq!((position, rev3), (0, 3));

        let (_position, rev4) = authority
            .reorder_queued_prompt("session-a", &first_id, rev3, 1)
            .await
            .expect("reorder should apply");
        assert_eq!(rev4, 4);
        let snapshots = authority.list_queued_snapshots("session-a").await;
        assert_eq!(snapshots[0].item_id, second_id);
        assert_eq!(snapshots[1].item_id, first_id);
        assert_eq!(snapshots[1].content, "updated");

        // Repeating the same request with the old revision is a safe no-op:
        // it is rejected rather than applying a second mutation.
        let repeated = authority
            .reorder_queued_prompt("session-a", &first_id, rev3, 0)
            .await;
        assert!(matches!(
            repeated,
            Err(SubmissionRejectionReason::QueueRevisionConflict {
                expected_revision: 3,
                current_revision: 4
            })
        ));
    }

    #[tokio::test]
    async fn test_queue_edit_idempotency_replays_and_rejects_reuse() {
        let authority = SubmissionAuthority::new();
        let (item, _, revision) = authority
            .enqueue_prompt("s", "r0".into(), "old".into())
            .await;
        let first = authority
            .edit_queued_prompt_idempotent("s", &item, revision, "new".into(), "edit-1")
            .await;
        let replay = authority
            .edit_queued_prompt_idempotent("s", &item, revision, "new".into(), "edit-1")
            .await;
        assert_eq!(first, replay);
        let mismatch = authority
            .edit_queued_prompt_idempotent("s", &item, revision, "other".into(), "edit-1")
            .await;
        assert!(matches!(
            mismatch,
            QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn queue_delete_and_reorder_idempotency_replay_and_reject_reuse() {
        let authority = SubmissionAuthority::new();
        let (first, _, rev1) = authority
            .enqueue_prompt("s", "r1".into(), "one".into())
            .await;
        let (second, _, rev2) = authority
            .enqueue_prompt("s", "r2".into(), "two".into())
            .await;
        assert_eq!(rev1, 1);
        let moved = authority
            .reorder_queued_prompt_idempotent("s", &second, rev2, 0, "move-1")
            .await;
        let moved_replay = authority
            .reorder_queued_prompt_idempotent("s", &second, rev2, 0, "move-1")
            .await;
        assert_eq!(moved, moved_replay);
        let move_mismatch = authority
            .reorder_queued_prompt_idempotent("s", &second, rev2, 1, "move-1")
            .await;
        assert!(matches!(
            move_mismatch,
            QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch { .. },
                ..
            }
        ));
        let current = authority.get_queue_revision("s").await;
        let deleted = authority
            .remove_queued_prompt_idempotent("s", &first, current, "delete-1")
            .await;
        let deleted_replay = authority
            .remove_queued_prompt_idempotent("s", &first, current, "delete-1")
            .await;
        assert_eq!(deleted, deleted_replay);
        let delete_mismatch = authority
            .remove_queued_prompt_idempotent("s", &second, current, "delete-1")
            .await;
        assert!(matches!(
            delete_mismatch,
            QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::IdempotencyPayloadMismatch { .. },
                ..
            }
        ));
        let cross_session = authority
            .remove_queued_prompt_idempotent("other", &second, 0, "delete-1")
            .await;
        assert!(matches!(
            cross_session,
            QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::QueueItemNotFound { .. },
                ..
            }
        ));
        let stale = authority
            .reorder_queued_prompt_idempotent("s", &second, current, 0, "move-stale")
            .await;
        assert!(matches!(
            stale,
            QueueMutationDisposition::Rejected {
                reason: SubmissionRejectionReason::QueueRevisionConflict { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn concurrent_duplicate_edit_has_single_side_effect() {
        let authority = std::sync::Arc::new(SubmissionAuthority::new());
        let (item, _, revision) = authority
            .enqueue_prompt("s", "r".into(), "old".into())
            .await;
        let a = authority.clone();
        let b = authority.clone();
        let item_a = item.clone();
        let item_b = item.clone();
        let (left, right) = tokio::join!(
            async move {
                a.edit_queued_prompt_idempotent("s", &item_a, revision, "new".into(), "same")
                    .await
            },
            async move {
                b.edit_queued_prompt_idempotent("s", &item_b, revision, "new".into(), "same")
                    .await
            },
        );
        assert_eq!(left, right);
        assert_eq!(authority.get_queue_revision("s").await, 2);
    }
}
