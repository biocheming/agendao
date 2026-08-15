use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

/// Metadata about the execution record that triggered a topology change.
#[derive(Debug, Clone)]
pub struct TopologyChangeContext {
    pub session_id: String,
    pub execution_id: String,
    pub stage_id: Option<String>,
}

/// Callback fired when the execution topology changes.
/// Receives the session_id, triggering execution_id, and its stage_id.
pub type TopologyChangedCallback = Arc<dyn Fn(&TopologyChangeContext) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    PromptRun,
    SchedulerRun,
    SchedulerNode,
    ToolCall,
    Question,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Waiting,
    Cancelling,
    Retry,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub id: String,
    pub session_id: String,
    pub kind: ExecutionKind,
    pub status: ExecutionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Optional grouping identifier for child execution records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_event: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExecutionNode {
    pub id: String,
    pub kind: ExecutionKind,
    pub status: ExecutionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_event: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub children: Vec<SessionExecutionNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExecutionTopology {
    pub session_id: String,
    pub active_count: usize,
    pub done_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub cancelling_count: usize,
    pub retry_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub roots: Vec<SessionExecutionNode>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionPatch {
    pub status: Option<ExecutionStatus>,
    pub label: FieldUpdate<String>,
    pub waiting_on: FieldUpdate<String>,
    pub recent_event: FieldUpdate<String>,
    pub metadata: FieldUpdate<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub enum FieldUpdate<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum SessionRunStatus {
    #[default]
    Idle,
    Busy,
    Compacting,
    Retry {
        attempt: u32,
        message: String,
        next: i64,
    },
    /// Session is blocked waiting for an external condition.
    Blocked {
        reason: Option<String>,
        /// Epoch millis when to recheck; None = blocked indefinitely.
        recheck_at: Option<i64>,
    },
    /// Session is intentionally sleeping.
    Sleeping {
        reason: Option<String>,
        /// Epoch millis when to wake; None = sleeping indefinitely.
        wake_at: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionOptionInfo {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionItemInfo {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOptionInfo>,
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionInfo {
    pub id: String,
    pub session_id: String,
    pub items: Vec<QuestionItemInfo>,
}

#[derive(Debug)]
pub enum QuestionReply {
    Answers(Vec<Vec<String>>),
    Rejected,
    Cancelled,
}

pub struct RuntimeControlRegistry {
    executions: RwLock<HashMap<String, ExecutionRecord>>,
    scheduler_tokens: Mutex<HashMap<String, CancellationToken>>,
    /// Cancellation tokens for tool calls and agent tasks, keyed by execution ID.
    execution_tokens: Mutex<HashMap<String, CancellationToken>>,
    question_waiters: Mutex<HashMap<String, oneshot::Sender<QuestionReply>>>,
    on_topology_changed: Option<TopologyChangedCallback>,
}

#[cfg(test)]
impl Default for RuntimeControlRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeControlRegistry {
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            executions: RwLock::new(HashMap::new()),
            scheduler_tokens: Mutex::new(HashMap::new()),
            execution_tokens: Mutex::new(HashMap::new()),
            question_waiters: Mutex::new(HashMap::new()),
            on_topology_changed: None,
        }
    }
    /// Create a registry with a callback that fires whenever the execution
    /// topology is mutated (upsert, update, or finish).
    pub fn with_topology_callback(callback: TopologyChangedCallback) -> Self {
        Self {
            executions: RwLock::new(HashMap::new()),
            scheduler_tokens: Mutex::new(HashMap::new()),
            execution_tokens: Mutex::new(HashMap::new()),
            question_waiters: Mutex::new(HashMap::new()),
            on_topology_changed: Some(callback),
        }
    }

    pub async fn set_session_run_status(&self, session_id: &str, status: SessionRunStatus) {
        let execution_id = prompt_execution_id(session_id);
        match status {
            SessionRunStatus::Idle => {
                self.finish_execution(&execution_id).await;
                // Clean up all Done records when the prompt run ends.
                self.cleanup_done_executions(session_id).await;
            }
            SessionRunStatus::Busy => {
                self.upsert_execution(ExecutionRecord {
                    id: execution_id,
                    session_id: session_id.to_string(),
                    kind: ExecutionKind::PromptRun,
                    status: ExecutionStatus::Running,
                    label: Some("Prompt run".to_string()),
                    parent_id: None,
                    stage_id: None,
                    waiting_on: None,
                    recent_event: Some("Prompt run started".to_string()),
                    started_at: now_millis(),
                    updated_at: now_millis(),
                    metadata: None,
                })
                .await;
            }
            SessionRunStatus::Compacting => {
                self.upsert_execution(ExecutionRecord {
                    id: execution_id,
                    session_id: session_id.to_string(),
                    kind: ExecutionKind::PromptRun,
                    status: ExecutionStatus::Waiting,
                    label: Some("Prompt run".to_string()),
                    parent_id: None,
                    stage_id: None,
                    waiting_on: Some("compaction".to_string()),
                    recent_event: Some("Compacting context".to_string()),
                    started_at: now_millis(),
                    updated_at: now_millis(),
                    metadata: None,
                })
                .await;
            }
            SessionRunStatus::Retry {
                attempt,
                message,
                next,
            } => {
                self.upsert_execution(ExecutionRecord {
                    id: execution_id,
                    session_id: session_id.to_string(),
                    kind: ExecutionKind::PromptRun,
                    status: ExecutionStatus::Retry,
                    label: Some("Prompt run".to_string()),
                    parent_id: None,
                    stage_id: None,
                    waiting_on: Some("retry_backoff".to_string()),
                    recent_event: Some(message.clone()),
                    started_at: now_millis(),
                    updated_at: now_millis(),
                    metadata: Some(serde_json::json!({
                        "attempt": attempt,
                        "message": message,
                        "next": next,
                    })),
                })
                .await;
            }
            SessionRunStatus::Blocked { reason, recheck_at } => {
                let mut meta = serde_json::json!({});
                if let Some(r) = reason.as_deref() {
                    meta["reason"] = serde_json::json!(r);
                }
                if let Some(ts) = recheck_at {
                    meta["recheck_at"] = serde_json::json!(ts);
                }
                self.upsert_execution(ExecutionRecord {
                    id: execution_id,
                    session_id: session_id.to_string(),
                    kind: ExecutionKind::PromptRun,
                    status: ExecutionStatus::Waiting,
                    label: Some("Prompt run".to_string()),
                    parent_id: None,
                    stage_id: None,
                    waiting_on: Some("blocked".to_string()),
                    recent_event: Some(reason.unwrap_or_else(|| "Blocked".to_string())),
                    started_at: now_millis(),
                    updated_at: now_millis(),
                    metadata: Some(meta),
                })
                .await;
            }
            SessionRunStatus::Sleeping { reason, wake_at } => {
                let mut meta = serde_json::json!({});
                if let Some(r) = reason.as_deref() {
                    meta["reason"] = serde_json::json!(r);
                }
                if let Some(ts) = wake_at {
                    meta["wake_at"] = serde_json::json!(ts);
                }
                self.upsert_execution(ExecutionRecord {
                    id: execution_id,
                    session_id: session_id.to_string(),
                    kind: ExecutionKind::PromptRun,
                    status: ExecutionStatus::Waiting,
                    label: Some("Prompt run".to_string()),
                    parent_id: None,
                    stage_id: None,
                    waiting_on: Some("sleeping".to_string()),
                    recent_event: Some(reason.unwrap_or_else(|| "Sleeping".to_string())),
                    started_at: now_millis(),
                    updated_at: now_millis(),
                    metadata: Some(meta),
                })
                .await;
            }
        }
    }

    pub async fn session_run_statuses(&self) -> HashMap<String, SessionRunStatus> {
        let executions = self.executions.read().await;
        executions
            .values()
            .filter(|record| {
                matches!(record.kind, ExecutionKind::PromptRun)
                    && record.status != ExecutionStatus::Done
            })
            .map(|record| {
                let status = match record.status {
                    ExecutionStatus::Running | ExecutionStatus::Cancelling => {
                        SessionRunStatus::Busy
                    }
                    ExecutionStatus::Waiting => match record.waiting_on.as_deref() {
                        Some("compaction") => SessionRunStatus::Compacting,
                        Some("blocked") => {
                            let metadata = record.metadata.as_ref();
                            SessionRunStatus::Blocked {
                                reason: metadata
                                    .and_then(|v| v.get("reason"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                recheck_at: metadata
                                    .and_then(|v| v.get("recheck_at"))
                                    .and_then(|v| v.as_i64()),
                            }
                        }
                        Some("sleeping") => {
                            let metadata = record.metadata.as_ref();
                            SessionRunStatus::Sleeping {
                                reason: metadata
                                    .and_then(|v| v.get("reason"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                wake_at: metadata
                                    .and_then(|v| v.get("wake_at"))
                                    .and_then(|v| v.as_i64()),
                            }
                        }
                        _ => SessionRunStatus::Busy,
                    },
                    ExecutionStatus::Retry => {
                        let metadata = record.metadata.as_ref();
                        SessionRunStatus::Retry {
                            attempt: metadata
                                .and_then(|value| value.get("attempt"))
                                .and_then(|value| value.as_u64())
                                .map(|value| value as u32)
                                .unwrap_or(1),
                            message: metadata
                                .and_then(|value| value.get("message"))
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            next: metadata
                                .and_then(|value| value.get("next"))
                                .and_then(|value| value.as_i64())
                                .unwrap_or_default(),
                        }
                    }
                    // Done is filtered out above, but satisfy exhaustiveness.
                    ExecutionStatus::Done => SessionRunStatus::Idle,
                };
                (record.session_id.clone(), status)
            })
            .collect()
    }

    pub async fn session_run_status(&self, session_id: &str) -> SessionRunStatus {
        self.session_run_statuses()
            .await
            .remove(session_id)
            .unwrap_or_default()
    }

    pub async fn has_prompt_run(&self, session_id: &str) -> bool {
        let executions = self.executions.read().await;
        executions
            .get(&prompt_execution_id(session_id))
            .map(|r| r.status != ExecutionStatus::Done)
            .unwrap_or(false)
    }

    pub async fn register_scheduler_run(
        &self,
        session_id: &str,
        token: CancellationToken,
        label: Option<String>,
    ) {
        self.scheduler_tokens
            .lock()
            .await
            .insert(session_id.to_string(), token);
        let execution_id = scheduler_execution_id(session_id);
        self.upsert_execution(ExecutionRecord {
            id: execution_id,
            session_id: session_id.to_string(),
            kind: ExecutionKind::SchedulerRun,
            status: ExecutionStatus::Running,
            label: label.or_else(|| Some("Scheduler run".to_string())),
            parent_id: Some(prompt_execution_id(session_id)),
            stage_id: None,
            waiting_on: Some("model".to_string()),
            recent_event: Some("Scheduler orchestration started".to_string()),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: None,
        })
        .await;
    }

    pub async fn request_scheduler_cancel(&self, session_id: &str) -> bool {
        let token = {
            let tokens = self.scheduler_tokens.lock().await;
            tokens.get(session_id).cloned()
        };
        let Some(token) = token else {
            return false;
        };
        token.cancel();
        self.update_execution(
            &scheduler_execution_id(session_id),
            ExecutionPatch {
                status: Some(ExecutionStatus::Cancelling),
                recent_event: FieldUpdate::Set("Cancellation requested".to_string()),
                ..ExecutionPatch::default()
            },
        )
        .await;
        true
    }

    pub async fn finish_scheduler_run(&self, session_id: &str) {
        self.scheduler_tokens.lock().await.remove(session_id);
        self.finish_execution(&scheduler_execution_id(session_id))
            .await;
        // Clean up all Done records for this session to prevent unbounded growth.
        self.cleanup_done_executions(session_id).await;
    }

    pub async fn register_scheduler_node(&self, session_id: &str, path: &str) {
        let execution_id = scheduler_node_execution_id(session_id, path);
        self.upsert_execution(ExecutionRecord {
            id: execution_id,
            session_id: session_id.to_string(),
            kind: ExecutionKind::SchedulerNode,
            status: ExecutionStatus::Running,
            label: Some(path.to_string()),
            parent_id: Some(scheduler_execution_id(session_id)),
            stage_id: None,
            waiting_on: Some("model".to_string()),
            recent_event: Some("Node started".to_string()),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: Some(serde_json::json!({ "path": path })),
        })
        .await;
        self.update_execution(
            &scheduler_execution_id(session_id),
            ExecutionPatch {
                recent_event: FieldUpdate::Set(format!("Node started: {path}")),
                waiting_on: FieldUpdate::Set("model".to_string()),
                ..ExecutionPatch::default()
            },
        )
        .await;
    }

    pub async fn update_scheduler_node(&self, session_id: &str, path: &str, patch: ExecutionPatch) {
        self.update_execution(&scheduler_node_execution_id(session_id, path), patch)
            .await;
    }

    pub async fn update_scheduler_run(&self, session_id: &str, patch: ExecutionPatch) {
        self.update_execution(&scheduler_execution_id(session_id), patch)
            .await;
    }

    pub async fn finish_scheduler_node(&self, session_id: &str, path: &str) {
        self.finish_execution(&scheduler_node_execution_id(session_id, path))
            .await;
    }

    // ── ToolCall lifecycle ──

    pub async fn register_tool_call(
        &self,
        tool_call_id: &str,
        session_id: &str,
        tool_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
    ) {
        self.register_tool_call_with_token(
            tool_call_id,
            session_id,
            tool_name,
            parent_id,
            stage_id,
            None,
        )
        .await;
    }

    pub async fn register_tool_call_with_token(
        &self,
        tool_call_id: &str,
        session_id: &str,
        tool_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
        token: Option<CancellationToken>,
    ) {
        let execution_id = Self::tool_call_execution_id(tool_call_id);
        if let Some(token) = token {
            self.execution_tokens
                .lock()
                .await
                .insert(execution_id.clone(), token);
        }
        self.upsert_execution(ExecutionRecord {
            id: execution_id,
            session_id: session_id.to_string(),
            kind: ExecutionKind::ToolCall,
            status: ExecutionStatus::Running,
            label: Some(format!("Tool: {tool_name}")),
            parent_id,
            stage_id,
            waiting_on: Some("tool".to_string()),
            recent_event: Some(format!("{tool_name} running")),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: Some(serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
            })),
        })
        .await;
    }

    pub async fn finish_tool_call(&self, tool_call_id: &str) {
        let execution_id = Self::tool_call_execution_id(tool_call_id);
        self.execution_tokens.lock().await.remove(&execution_id);
        self.finish_execution(&execution_id).await;
    }

    // ── Unified cancel dispatch ──

    /// Cancel any registered execution by ID. Returns the kind that was
    /// cancelled (or `None` if the execution was not found).
    pub async fn cancel_execution(&self, execution_id: &str) -> Option<ExecutionKind> {
        let kind = {
            let executions = self.executions.read().await;
            executions
                .get(execution_id)
                .map(|r| (r.kind.clone(), r.session_id.clone()))
        };
        let (kind, session_id) = kind?;
        match kind {
            ExecutionKind::SchedulerRun => {
                self.request_scheduler_cancel(&session_id).await;
            }
            ExecutionKind::SchedulerNode => {
                self.request_scheduler_cancel(&session_id).await;
            }
            ExecutionKind::Question => {
                self.reject_question(execution_id).await;
            }
            ExecutionKind::ToolCall => {
                // Cancel via stored token if available, then mark as cancelling.
                let token = {
                    let tokens = self.execution_tokens.lock().await;
                    tokens.get(execution_id).cloned()
                };
                if let Some(token) = token {
                    token.cancel();
                }
                self.update_execution(
                    execution_id,
                    ExecutionPatch {
                        status: Some(ExecutionStatus::Cancelling),
                        recent_event: FieldUpdate::Set("Cancellation requested".to_string()),
                        ..ExecutionPatch::default()
                    },
                )
                .await;
            }
            ExecutionKind::PromptRun => {
                // PromptRun cancellation is not supported through this entry point.
            }
        }
        Some(kind)
    }

    pub async fn register_question(
        &self,
        session_id: String,
        questions: Vec<agendao_tool::QuestionDef>,
    ) -> (QuestionInfo, oneshot::Receiver<QuestionReply>) {
        let request_id = format!("question_{}", uuid::Uuid::new_v4().simple());
        let info = QuestionInfo {
            id: request_id.clone(),
            session_id: session_id.clone(),
            items: questions
                .iter()
                .map(|q| QuestionItemInfo {
                    question: q.question.clone(),
                    header: q.header.clone(),
                    options: q
                        .options
                        .iter()
                        .map(|o| QuestionOptionInfo {
                            label: o.label.clone(),
                            description: o.description.clone(),
                        })
                        .collect(),
                    multiple: q.multiple,
                })
                .collect(),
        };
        let (parent_id, stage_id) = {
            let executions = self.executions.read().await;
            let pid = select_question_parent_id(&executions, &session_id);
            // Resolve stage_id from parent's record.
            let sid = pid
                .as_ref()
                .and_then(|pid| executions.get(pid).and_then(|r| r.stage_id.clone()));
            (pid, sid)
        };
        let execution = ExecutionRecord {
            id: request_id.clone(),
            session_id,
            kind: ExecutionKind::Question,
            status: ExecutionStatus::Waiting,
            label: Some(format!("Question ({})", info.items.len())),
            parent_id,
            stage_id,
            waiting_on: Some("user".to_string()),
            recent_event: Some("Waiting for user answer".to_string()),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: Some(serde_json::to_value(&info).unwrap_or(serde_json::Value::Null)),
        };
        let (tx, rx) = oneshot::channel::<QuestionReply>();
        self.executions
            .write()
            .await
            .insert(request_id.clone(), execution);
        self.question_waiters.lock().await.insert(request_id, tx);
        (info, rx)
    }

    pub async fn list_questions(&self) -> Vec<QuestionInfo> {
        let executions = self.executions.read().await;
        let mut result = executions
            .values()
            .filter_map(question_record_to_info)
            .collect::<Vec<_>>();
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    pub async fn list_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        let executions = self.executions.read().await;
        let mut result = executions
            .values()
            .filter(|record| record.session_id == session_id)
            .filter_map(question_record_to_info)
            .collect::<Vec<_>>();
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    pub async fn list_session_execution_topology(
        &self,
        session_id: &str,
    ) -> SessionExecutionTopology {
        build_session_execution_topology(
            session_id.to_string(),
            self.list_session_execution_records(session_id).await,
        )
    }

    pub async fn list_session_execution_records(&self, session_id: &str) -> Vec<ExecutionRecord> {
        let executions = self.executions.read().await;
        executions
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Return all active execution records across every session.
    pub async fn list_all_executions(&self) -> Vec<ExecutionRecord> {
        let executions = self.executions.read().await;
        executions.values().cloned().collect()
    }

    /// Return the set of session IDs that currently have at least one active
    /// (non-Done) execution record.
    pub async fn list_active_session_ids(&self) -> Vec<String> {
        let executions = self.executions.read().await;
        let mut ids: Vec<String> = executions
            .values()
            .filter(|r| r.status != ExecutionStatus::Done)
            .map(|r| r.session_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        ids.sort();
        ids
    }

    #[cfg(test)]
    pub async fn has_cancellation_token(&self, execution_id: &str) -> bool {
        self.execution_tokens
            .lock()
            .await
            .contains_key(execution_id)
    }

    pub async fn answer_question(
        &self,
        id: &str,
        answers: Vec<Vec<String>>,
    ) -> Option<QuestionInfo> {
        let info = self.take_question(id).await?;
        if let Some(waiter) = self.question_waiters.lock().await.remove(id) {
            let _ = waiter.send(QuestionReply::Answers(answers));
        }
        Some(info)
    }

    pub async fn reject_question(&self, id: &str) -> Option<QuestionInfo> {
        let info = self.take_question(id).await?;
        if let Some(waiter) = self.question_waiters.lock().await.remove(id) {
            let _ = waiter.send(QuestionReply::Rejected);
        }
        Some(info)
    }

    pub async fn cancel_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        let ids = {
            let executions = self.executions.read().await;
            executions
                .values()
                .filter(|record| {
                    record.session_id == session_id
                        && matches!(record.kind, ExecutionKind::Question)
                })
                .map(|record| record.id.clone())
                .collect::<Vec<_>>()
        };

        let mut cancelled = Vec::new();
        for id in ids {
            if let Some(info) = self.take_question(&id).await {
                if let Some(waiter) = self.question_waiters.lock().await.remove(&id) {
                    let _ = waiter.send(QuestionReply::Cancelled);
                }
                cancelled.push(info);
            }
        }
        cancelled
    }

    pub async fn drop_question(&self, id: &str) {
        self.finish_execution(id).await;
        self.question_waiters.lock().await.remove(id);
    }

    /// Resolve the `stage_id` for a given execution by looking it up in the registry.
    pub async fn resolve_stage_id(&self, execution_id: &str) -> Option<String> {
        let executions = self.executions.read().await;
        executions
            .get(execution_id)
            .and_then(|r| r.stage_id.clone())
    }

    async fn take_question(&self, id: &str) -> Option<QuestionInfo> {
        let record = self.executions.write().await.remove(id)?;
        question_record_to_info(&record)
    }

    async fn upsert_execution(&self, record: ExecutionRecord) {
        let ctx = TopologyChangeContext {
            session_id: record.session_id.clone(),
            execution_id: record.id.clone(),
            stage_id: record.stage_id.clone(),
        };
        let mut executions = self.executions.write().await;
        let next = match executions.get(&record.id) {
            Some(existing) => ExecutionRecord {
                started_at: existing.started_at,
                ..record
            },
            None => record,
        };
        executions.insert(next.id.clone(), next);
        drop(executions);
        self.notify_topology_changed(&ctx);
    }

    async fn update_execution(&self, id: &str, patch: ExecutionPatch) {
        let mut executions = self.executions.write().await;
        let Some(record) = executions.get_mut(id) else {
            return;
        };

        let ctx = TopologyChangeContext {
            session_id: record.session_id.clone(),
            execution_id: record.id.clone(),
            stage_id: record.stage_id.clone(),
        };
        if let Some(status) = patch.status {
            record.status = status;
        }
        apply_field_update(&mut record.label, patch.label);
        apply_field_update(&mut record.waiting_on, patch.waiting_on);
        apply_field_update(&mut record.recent_event, patch.recent_event);
        apply_field_update(&mut record.metadata, patch.metadata);
        record.updated_at = now_millis();
        drop(executions);
        self.notify_topology_changed(&ctx);
    }

    async fn finish_execution(&self, id: &str) {
        let mut executions = self.executions.write().await;
        let Some(record) = executions.get_mut(id) else {
            return;
        };
        let ctx = TopologyChangeContext {
            session_id: record.session_id.clone(),
            execution_id: record.id.clone(),
            stage_id: record.stage_id.clone(),
        };
        record.status = ExecutionStatus::Done;
        record.waiting_on = None;
        record.updated_at = now_millis();
        drop(executions);
        self.notify_topology_changed(&ctx);
    }

    /// Remove all `Done` execution records for a given session.
    /// Called when a session-level run finishes to prevent unbounded growth.
    async fn cleanup_done_executions(&self, session_id: &str) {
        let mut executions = self.executions.write().await;
        executions.retain(|_, record| {
            !(record.session_id == session_id && record.status == ExecutionStatus::Done)
        });
    }

    fn notify_topology_changed(&self, ctx: &TopologyChangeContext) {
        if let Some(ref callback) = self.on_topology_changed {
            callback(ctx);
        }
    }
}

pub fn build_session_execution_topology(
    session_id: String,
    mut records: Vec<ExecutionRecord>,
) -> SessionExecutionTopology {
    let total_count = records.len();
    let done_count = records
        .iter()
        .filter(|record| matches!(record.status, ExecutionStatus::Done))
        .count();
    let active_count = total_count - done_count;
    let running_count = records
        .iter()
        .filter(|record| matches!(record.status, ExecutionStatus::Running))
        .count();
    let waiting_count = records
        .iter()
        .filter(|record| matches!(record.status, ExecutionStatus::Waiting))
        .count();
    let cancelling_count = records
        .iter()
        .filter(|record| matches!(record.status, ExecutionStatus::Cancelling))
        .count();
    let retry_count = records
        .iter()
        .filter(|record| matches!(record.status, ExecutionStatus::Retry))
        .count();
    let updated_at = records.iter().map(|record| record.updated_at).max();

    records.sort_by(execution_sort_key);

    let mut children_by_parent: HashMap<String, Vec<ExecutionRecord>> = HashMap::new();
    let mut roots = Vec::new();
    let record_ids = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    for record in records {
        let has_parent = record
            .parent_id
            .as_ref()
            .map(|parent_id| record_ids.contains(parent_id.as_str()))
            .unwrap_or(false);
        if has_parent {
            children_by_parent
                .entry(record.parent_id.clone().unwrap_or_default())
                .or_default()
                .push(record);
        } else {
            roots.push(record);
        }
    }

    let roots = roots
        .into_iter()
        .map(|record| build_execution_node(record, &mut children_by_parent))
        .collect::<Vec<_>>();

    SessionExecutionTopology {
        session_id,
        active_count,
        done_count,
        running_count,
        waiting_count,
        cancelling_count,
        retry_count,
        updated_at,
        roots,
    }
}

fn build_execution_node(
    record: ExecutionRecord,
    children_by_parent: &mut HashMap<String, Vec<ExecutionRecord>>,
) -> SessionExecutionNode {
    let mut children = children_by_parent.remove(&record.id).unwrap_or_default();
    children.sort_by(execution_sort_key);
    let children = children
        .into_iter()
        .map(|child| build_execution_node(child, children_by_parent))
        .collect::<Vec<_>>();

    SessionExecutionNode {
        id: record.id,
        kind: record.kind,
        status: record.status,
        label: record.label,
        parent_id: record.parent_id,
        stage_id: record.stage_id,
        waiting_on: record.waiting_on,
        recent_event: record.recent_event,
        started_at: record.started_at,
        updated_at: record.updated_at,
        metadata: record.metadata,
        children,
    }
}

fn execution_sort_key(left: &ExecutionRecord, right: &ExecutionRecord) -> std::cmp::Ordering {
    left.started_at
        .cmp(&right.started_at)
        .then_with(|| kind_rank(&left.kind).cmp(&kind_rank(&right.kind)))
        .then_with(|| left.id.cmp(&right.id))
}

fn kind_rank(kind: &ExecutionKind) -> u8 {
    match kind {
        ExecutionKind::PromptRun => 0,
        ExecutionKind::SchedulerRun => 1,
        ExecutionKind::SchedulerNode => 2,
        ExecutionKind::ToolCall => 3,
        ExecutionKind::Question => 4,
    }
}

fn select_question_parent_id(
    executions: &HashMap<String, ExecutionRecord>,
    session_id: &str,
) -> Option<String> {
    executions
        .values()
        .filter(|record| record.session_id == session_id)
        .filter(|record| !matches!(record.kind, ExecutionKind::Question))
        .filter(|record| record.status != ExecutionStatus::Done)
        .max_by(|left, right| {
            kind_rank(&left.kind)
                .cmp(&kind_rank(&right.kind))
                .then_with(|| left.updated_at.cmp(&right.updated_at))
        })
        .map(|record| record.id.clone())
}

fn prompt_execution_id(session_id: &str) -> String {
    format!("prompt:{session_id}")
}

fn scheduler_execution_id(session_id: &str) -> String {
    format!("scheduler:{session_id}")
}

fn scheduler_node_execution_id(session_id: &str, path: &str) -> String {
    format!("scheduler_node:{session_id}:{path}")
}

impl RuntimeControlRegistry {
    pub fn tool_call_execution_id(tool_call_id: &str) -> String {
        format!("tool_call:{tool_call_id}")
    }

    pub async fn count_active_stage_tools(&self, stage_id: &str) -> u32 {
        let executions = self.executions.read().await;
        executions
            .values()
            .filter(|record| {
                record.kind == ExecutionKind::ToolCall
                    && record.stage_id.as_deref() == Some(stage_id)
                    && record.status != ExecutionStatus::Done
            })
            .count() as u32
    }
}

fn apply_field_update<T>(target: &mut Option<T>, update: FieldUpdate<T>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(value) => *target = Some(value),
        FieldUpdate::Clear => *target = None,
    }
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn question_record_to_info(record: &ExecutionRecord) -> Option<QuestionInfo> {
    if !matches!(record.kind, ExecutionKind::Question) {
        return None;
    }
    serde_json::from_value(record.metadata.clone()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prompt_status_roundtrip_uses_single_registry() {
        let registry = RuntimeControlRegistry::new();
        assert!(!registry.has_prompt_run("ses_1").await);

        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        assert!(registry.has_prompt_run("ses_1").await);
        let statuses = registry.session_run_statuses().await;
        assert!(matches!(
            statuses.get("ses_1"),
            Some(SessionRunStatus::Busy)
        ));

        registry
            .set_session_run_status("ses_1", SessionRunStatus::Idle)
            .await;
        assert!(!registry.has_prompt_run("ses_1").await);
    }

    #[tokio::test]
    async fn scheduler_cancel_updates_registry_state() {
        let registry = RuntimeControlRegistry::new();
        let token = CancellationToken::new();
        registry
            .register_scheduler_run("ses_1", token.clone(), Some("Prometheus".to_string()))
            .await;
        assert!(!token.is_cancelled());
        assert!(registry.request_scheduler_cancel("ses_1").await);
        assert!(token.is_cancelled());
        registry.finish_scheduler_run("ses_1").await;
        assert!(!registry.request_scheduler_cancel("ses_1").await);
    }

    #[tokio::test]
    async fn question_lifecycle_flows_through_registry() {
        let registry = RuntimeControlRegistry::new();
        let questions = vec![agendao_tool::QuestionDef {
            question: "Pick one".to_string(),
            header: Some("Need".to_string()),
            options: vec![agendao_tool::QuestionOption {
                label: "A".to_string(),
                description: Some("first".to_string()),
            }],
            multiple: false,
        }];
        let (info, rx) = registry
            .register_question("ses_1".to_string(), questions)
            .await;
        assert_eq!(registry.list_questions().await.len(), 1);
        let answered = registry
            .answer_question(&info.id, vec![vec!["A".to_string()]])
            .await
            .expect("question exists");
        assert_eq!(answered.id, info.id);
        match rx.await.expect("receiver should resolve") {
            QuestionReply::Answers(values) => {
                assert_eq!(values, vec![vec!["A".to_string()]]);
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(registry.list_questions().await.is_empty());
    }

    #[tokio::test]
    async fn topology_builds_parent_child_graph_for_active_executions() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", CancellationToken::new(), Some("Atlas".to_string()))
            .await;
        registry
            .register_scheduler_node("ses_1", "root/coordination-gate")
            .await;

        let (question, _) = registry
            .register_question(
                "ses_1".to_string(),
                vec![agendao_tool::QuestionDef {
                    question: "Approve?".to_string(),
                    header: Some("Decision".to_string()),
                    options: vec![agendao_tool::QuestionOption {
                        label: "Yes".to_string(),
                        description: None,
                    }],
                    multiple: false,
                }],
            )
            .await;

        let topology = registry.list_session_execution_topology("ses_1").await;
        assert_eq!(topology.active_count, 4);
        assert_eq!(topology.roots.len(), 1);
        let prompt = &topology.roots[0];
        assert!(matches!(prompt.kind, ExecutionKind::PromptRun));
        let scheduler = prompt
            .children
            .iter()
            .find(|node| matches!(node.kind, ExecutionKind::SchedulerRun))
            .expect("scheduler child");
        let node = scheduler
            .children
            .iter()
            .find(|node| matches!(node.kind, ExecutionKind::SchedulerNode))
            .expect("scheduler node child");
        let question_node = node
            .children
            .iter()
            .find(|node| node.id == question.id)
            .expect("question child");
        assert_eq!(question_node.waiting_on.as_deref(), Some("user"));
    }

    #[tokio::test]
    async fn tool_call_lifecycle_register_and_finish() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_tool_call(
                "tc_1",
                "ses_1",
                "read_file",
                Some(prompt_execution_id("ses_1")),
                None,
            )
            .await;

        let records = registry.list_session_execution_records("ses_1").await;
        let tool_record = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::ToolCall))
            .expect("tool call should be registered");
        assert_eq!(tool_record.id, "tool_call:tc_1");
        assert!(matches!(tool_record.status, ExecutionStatus::Running));
        assert_eq!(tool_record.label.as_deref(), Some("Tool: read_file"));
        assert_eq!(
            tool_record.parent_id.as_deref(),
            Some(prompt_execution_id("ses_1").as_str())
        );

        // Topology should include the tool call as a child of PromptRun.
        let topology = registry.list_session_execution_topology("ses_1").await;
        assert_eq!(topology.active_count, 2);
        let prompt = &topology.roots[0];
        let tool_node = prompt
            .children
            .iter()
            .find(|n| matches!(n.kind, ExecutionKind::ToolCall))
            .expect("tool call child");
        assert_eq!(tool_node.id, "tool_call:tc_1");

        // Finish marks the tool call as Done (not removed).
        registry.finish_tool_call("tc_1").await;
        let records = registry.list_session_execution_records("ses_1").await;
        let tool_after = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::ToolCall))
            .expect("tool call should still exist with Done status");
        assert!(
            matches!(tool_after.status, ExecutionStatus::Done),
            "tool call should be Done after finish"
        );
    }

    #[tokio::test]
    async fn cancel_execution_dispatches_to_correct_kind() {
        let registry = RuntimeControlRegistry::new();
        let token = CancellationToken::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", token.clone(), Some("Atlas".to_string()))
            .await;
        registry
            .register_tool_call("tc_x", "ses_1", "write_file", None, None)
            .await;

        // Cancel tool call → marks as Cancelling.
        let kind = registry.cancel_execution("tool_call:tc_x").await;
        assert_eq!(kind, Some(ExecutionKind::ToolCall));
        let records = registry.list_session_execution_records("ses_1").await;
        let tool = records
            .iter()
            .find(|r| r.id == "tool_call:tc_x")
            .expect("tool should exist");
        assert!(matches!(tool.status, ExecutionStatus::Cancelling));

        // Cancel scheduler → cancels token.
        let kind = registry
            .cancel_execution(&scheduler_execution_id("ses_1"))
            .await;
        assert_eq!(kind, Some(ExecutionKind::SchedulerRun));
        assert!(token.is_cancelled());

        // Cancel non-existent → None.
        let kind = registry.cancel_execution("nonexistent").await;
        assert!(kind.is_none());
    }

    #[tokio::test]
    async fn tool_call_appears_under_scheduler_node_in_topology() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", CancellationToken::new(), None)
            .await;
        let node_path = "root/plan";
        registry.register_scheduler_node("ses_1", node_path).await;
        let node_id = scheduler_node_execution_id("ses_1", node_path);
        registry
            .register_tool_call("tc_read", "ses_1", "read_file", Some(node_id), None)
            .await;

        let topology = registry.list_session_execution_topology("ses_1").await;
        assert_eq!(topology.active_count, 4); // prompt + scheduler + node + tool
        let prompt = &topology.roots[0];
        let scheduler = &prompt.children[0];
        let node = &scheduler.children[0];
        let tool = node
            .children
            .iter()
            .find(|n| matches!(n.kind, ExecutionKind::ToolCall))
            .expect("tool call under scheduler node");
        assert_eq!(tool.id, "tool_call:tc_read");
    }

    #[tokio::test]
    async fn topology_callback_receives_enriched_context() {
        use std::sync::{Arc, Mutex as StdMutex};

        type TopologyEvents = Arc<StdMutex<Vec<(String, String, Option<String>)>>>;

        let events: TopologyEvents = Arc::new(StdMutex::new(Vec::new()));
        let events_clone = events.clone();

        let registry = RuntimeControlRegistry::with_topology_callback(Arc::new(
            move |ctx: &TopologyChangeContext| {
                let mut guard = events_clone.lock().unwrap();
                guard.push((
                    ctx.session_id.clone(),
                    ctx.execution_id.clone(),
                    ctx.stage_id.clone(),
                ));
            },
        ));

        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry.register_scheduler_node("ses_1", "root/test").await;
        let node_id = scheduler_node_execution_id("ses_1", "root/test");
        registry
            .register_tool_call("tc_1", "ses_1", "bash", Some(node_id.clone()), None)
            .await;

        let captured = events.lock().unwrap();
        // Find the tool call event.
        let tool_event = captured
            .iter()
            .find(|(_, eid, _)| eid == "tool_call:tc_1")
            .expect("should have captured tool call event");
        assert_eq!(tool_event.0, "ses_1");
        assert_eq!(tool_event.2, None);

        // Find the scheduler node event.
        let node_event = captured
            .iter()
            .find(|(_, eid, _)| eid == &node_id)
            .expect("should have captured scheduler node event");
        assert_eq!(node_event.2, None);
    }

    #[tokio::test]
    async fn cancel_tool_call_triggers_cancellation_token() {
        let registry = RuntimeControlRegistry::new();
        let token = CancellationToken::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_tool_call_with_token(
                "tc_1",
                "ses_1",
                "write_file",
                None,
                None,
                Some(token.clone()),
            )
            .await;
        assert!(!token.is_cancelled());
        assert!(registry.has_cancellation_token("tool_call:tc_1").await);

        let kind = registry.cancel_execution("tool_call:tc_1").await;
        assert_eq!(kind, Some(ExecutionKind::ToolCall));
        assert!(token.is_cancelled(), "token should be cancelled");

        // finish cleans up token
        registry.finish_tool_call("tc_1").await;
        assert!(!registry.has_cancellation_token("tool_call:tc_1").await);
    }

    #[tokio::test]
    async fn cancel_tool_call_without_token_still_marks_cancelling() {
        let registry = RuntimeControlRegistry::new();
        registry
            .register_tool_call("tc_notoken", "ses_1", "read_file", None, None)
            .await;
        let kind = registry.cancel_execution("tool_call:tc_notoken").await;
        assert_eq!(kind, Some(ExecutionKind::ToolCall));
        let records = registry.list_session_execution_records("ses_1").await;
        let tool = records
            .iter()
            .find(|r| r.id == "tool_call:tc_notoken")
            .unwrap();
        assert!(matches!(tool.status, ExecutionStatus::Cancelling));
    }

    #[tokio::test]
    async fn list_all_executions_spans_multiple_sessions() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .set_session_run_status("ses_2", SessionRunStatus::Busy)
            .await;
        registry
            .register_tool_call("tc_a", "ses_1", "read", None, None)
            .await;
        registry
            .register_tool_call("tc_b", "ses_2", "write", None, None)
            .await;

        let all = registry.list_all_executions().await;
        assert_eq!(all.len(), 4); // 2 prompt runs + 2 tool calls

        let ids = registry.list_active_session_ids().await;
        assert_eq!(ids, vec!["ses_1".to_string(), "ses_2".to_string()]);
    }
}
