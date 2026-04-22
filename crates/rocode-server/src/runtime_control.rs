use chrono::Utc;
#[cfg(test)]
use rocode_command::stage_protocol::{ExecutionNode, ExecutionNodeKind, ExecutionNodeStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

/// Metadata about the execution record that triggered a topology change.
#[derive(Debug, Clone)]
pub(crate) struct TopologyChangeContext {
    pub session_id: String,
    pub execution_id: String,
    pub stage_id: Option<String>,
}

/// Callback fired when the execution topology changes.
/// Receives the session_id, triggering execution_id, and its stage_id.
pub(crate) type TopologyChangedCallback = Arc<dyn Fn(&TopologyChangeContext) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    PromptRun,
    SchedulerRun,
    SchedulerStage,
    ToolCall,
    AgentTask,
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
    /// First-class stage identifier. For `SchedulerStage` nodes this equals
    /// `id`; for child nodes (tool, agent, question, subsession) it points to
    /// the owning stage.
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

impl ExecutionRecord {
    /// Project this internal record into a protocol-level [`ExecutionNode`].
    ///
    /// The protocol shape is defined in `rocode-command::stage_protocol` so
    /// that CLI, TUI, and Web can consume it without depending on server internals.
    #[cfg(test)]
    pub(crate) fn to_node(&self) -> ExecutionNode {
        ExecutionNode {
            execution_id: self.id.clone(),
            parent_execution_id: self.parent_id.clone(),
            // Prefer first-class field; fall back to metadata for backward compat.
            stage_id: self.stage_id.clone().or_else(|| {
                self.metadata
                    .as_ref()
                    .and_then(|m| m.get("scheduler_stage_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            }),
            kind: match self.kind {
                ExecutionKind::SchedulerStage => ExecutionNodeKind::Stage,
                ExecutionKind::AgentTask => ExecutionNodeKind::Agent,
                ExecutionKind::ToolCall => ExecutionNodeKind::Tool,
                ExecutionKind::Question => ExecutionNodeKind::Question,
                // PromptRun / SchedulerRun are top-level orchestration → Stage
                ExecutionKind::PromptRun | ExecutionKind::SchedulerRun => ExecutionNodeKind::Stage,
            },
            label: self.label.clone(),
            status: match self.status {
                ExecutionStatus::Running => ExecutionNodeStatus::Running,
                ExecutionStatus::Waiting => ExecutionNodeStatus::Waiting,
                ExecutionStatus::Cancelling => ExecutionNodeStatus::Cancelling,
                ExecutionStatus::Retry => ExecutionNodeStatus::Retry,
                ExecutionStatus::Done => ExecutionNodeStatus::Done,
            },
            waiting_on: self.waiting_on.clone(),
            started_at: self.started_at,
            updated_at: self.updated_at,
            session_id: self.session_id.clone(),
            child_session_id: self
                .metadata
                .as_ref()
                .and_then(|m| m.get("child_session_id"))
                .and_then(|v| v.as_str())
                .map(String::from),
        }
    }
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
pub(crate) struct ExecutionPatch {
    pub status: Option<ExecutionStatus>,
    pub label: FieldUpdate<String>,
    pub waiting_on: FieldUpdate<String>,
    pub recent_event: FieldUpdate<String>,
    pub metadata: FieldUpdate<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum FieldUpdate<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum SessionRunStatus {
    #[default]
    Idle,
    Busy,
    Retry {
        attempt: u32,
        message: String,
        next: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOptionInfo {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct QuestionInfo {
    pub id: String,
    pub session_id: String,
    /// Legacy: flat question strings (kept for backward compat).
    pub questions: Vec<String>,
    /// Legacy: flat option labels per question (kept for backward compat).
    pub options: Option<Vec<Vec<String>>>,
    /// Full-fidelity question items with descriptions, headers, multi-select.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<QuestionItemInfo>,
}

#[derive(Debug)]
pub(crate) enum QuestionReply {
    Answers(Vec<Vec<String>>),
    Rejected,
    Cancelled,
}

pub(crate) struct RuntimeControlRegistry {
    executions: RwLock<HashMap<String, ExecutionRecord>>,
    scheduler_tokens: Mutex<HashMap<String, CancellationToken>>,
    /// Cancellation tokens for tool calls and agent tasks, keyed by execution ID.
    execution_tokens: Mutex<HashMap<String, CancellationToken>>,
    question_waiters: Mutex<HashMap<String, oneshot::Sender<QuestionReply>>>,
    on_topology_changed: Option<TopologyChangedCallback>,
}

impl RuntimeControlRegistry {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
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
    pub(crate) fn with_topology_callback(callback: TopologyChangedCallback) -> Self {
        Self {
            executions: RwLock::new(HashMap::new()),
            scheduler_tokens: Mutex::new(HashMap::new()),
            execution_tokens: Mutex::new(HashMap::new()),
            question_waiters: Mutex::new(HashMap::new()),
            on_topology_changed: Some(callback),
        }
    }

    pub(crate) async fn set_session_run_status(&self, session_id: &str, status: SessionRunStatus) {
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
        }
    }

    pub(crate) async fn session_run_statuses(&self) -> HashMap<String, SessionRunStatus> {
        let executions = self.executions.read().await;
        executions
            .values()
            .filter(|record| {
                matches!(record.kind, ExecutionKind::PromptRun)
                    && record.status != ExecutionStatus::Done
            })
            .map(|record| {
                let status = match record.status {
                    ExecutionStatus::Running
                    | ExecutionStatus::Waiting
                    | ExecutionStatus::Cancelling => SessionRunStatus::Busy,
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

    pub(crate) async fn has_prompt_run(&self, session_id: &str) -> bool {
        let executions = self.executions.read().await;
        executions
            .get(&prompt_execution_id(session_id))
            .map(|r| r.status != ExecutionStatus::Done)
            .unwrap_or(false)
    }

    pub(crate) async fn register_scheduler_run(
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

    pub(crate) async fn request_scheduler_cancel(&self, session_id: &str) -> bool {
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

    pub(crate) async fn finish_scheduler_run(&self, session_id: &str) {
        self.scheduler_tokens.lock().await.remove(session_id);
        self.finish_execution(&scheduler_execution_id(session_id))
            .await;
        // Clean up all Done records for this session to prevent unbounded growth.
        self.cleanup_done_executions(session_id).await;
    }

    pub(crate) async fn register_scheduler_stage(
        &self,
        session_id: &str,
        execution_id: String,
        label: String,
        metadata: serde_json::Value,
    ) {
        // A stage's own stage_id equals its execution_id.
        let stage_id = execution_id.clone();
        self.upsert_execution(ExecutionRecord {
            id: execution_id.clone(),
            session_id: session_id.to_string(),
            kind: ExecutionKind::SchedulerStage,
            status: ExecutionStatus::Running,
            label: Some(label),
            parent_id: Some(scheduler_execution_id(session_id)),
            stage_id: Some(stage_id),
            waiting_on: Some("model".to_string()),
            recent_event: Some("Stage started".to_string()),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: Some(metadata),
        })
        .await;
        self.update_execution(
            &scheduler_execution_id(session_id),
            ExecutionPatch {
                recent_event: FieldUpdate::Set("Scheduler stage started".to_string()),
                waiting_on: FieldUpdate::Set("model".to_string()),
                ..ExecutionPatch::default()
            },
        )
        .await;
    }

    pub(crate) async fn update_scheduler_stage(&self, execution_id: &str, patch: ExecutionPatch) {
        self.update_execution(execution_id, patch).await;
    }

    pub(crate) async fn mark_scheduler_stage_cancelling(&self, execution_id: &str) {
        self.update_execution(
            execution_id,
            ExecutionPatch {
                status: Some(ExecutionStatus::Cancelling),
                waiting_on: FieldUpdate::Clear,
                recent_event: FieldUpdate::Set("Cancellation requested".to_string()),
                ..ExecutionPatch::default()
            },
        )
        .await;
    }

    pub(crate) async fn finish_scheduler_stage(&self, execution_id: &str) {
        self.finish_execution(execution_id).await;
    }

    // ── ToolCall lifecycle ──

    pub(crate) async fn register_tool_call(
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

    pub(crate) async fn register_tool_call_with_token(
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

    pub(crate) async fn finish_tool_call(&self, tool_call_id: &str) {
        let execution_id = Self::tool_call_execution_id(tool_call_id);
        self.execution_tokens.lock().await.remove(&execution_id);
        self.finish_execution(&execution_id).await;
    }

    // ── AgentTask lifecycle ──

    pub(crate) async fn register_agent_task(
        &self,
        task_id: &str,
        session_id: &str,
        agent_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
    ) {
        self.register_agent_task_with_token(
            task_id, session_id, agent_name, parent_id, stage_id, None,
        )
        .await;
    }

    pub(crate) async fn register_agent_task_with_token(
        &self,
        task_id: &str,
        session_id: &str,
        agent_name: &str,
        parent_id: Option<String>,
        stage_id: Option<String>,
        token: Option<CancellationToken>,
    ) {
        let execution_id = Self::agent_task_execution_id(task_id);
        if let Some(token) = token {
            self.execution_tokens
                .lock()
                .await
                .insert(execution_id.clone(), token);
        }
        self.upsert_execution(ExecutionRecord {
            id: execution_id,
            session_id: session_id.to_string(),
            kind: ExecutionKind::AgentTask,
            status: ExecutionStatus::Running,
            label: Some(format!("Agent: {agent_name}")),
            parent_id,
            stage_id,
            waiting_on: Some("model".to_string()),
            recent_event: Some(format!("{agent_name} started")),
            started_at: now_millis(),
            updated_at: now_millis(),
            metadata: Some(serde_json::json!({
                "task_id": task_id,
                "agent_name": agent_name,
            })),
        })
        .await;
    }

    pub(crate) async fn finish_agent_task(&self, task_id: &str) {
        let execution_id = Self::agent_task_execution_id(task_id);
        self.execution_tokens.lock().await.remove(&execution_id);
        self.finish_execution(&execution_id).await;
    }

    // ── Unified cancel dispatch ──

    /// Cancel any registered execution by ID. Returns the kind that was
    /// cancelled (or `None` if the execution was not found).
    pub(crate) async fn cancel_execution(&self, execution_id: &str) -> Option<ExecutionKind> {
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
            ExecutionKind::SchedulerStage => {
                self.mark_scheduler_stage_cancelling(execution_id).await;
                self.request_scheduler_cancel(&session_id).await;
            }
            ExecutionKind::Question => {
                self.reject_question(execution_id).await;
            }
            ExecutionKind::ToolCall | ExecutionKind::AgentTask => {
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

    pub(crate) async fn register_question(
        &self,
        session_id: String,
        questions: Vec<rocode_tool::QuestionDef>,
    ) -> (QuestionInfo, oneshot::Receiver<QuestionReply>) {
        let request_id = format!("question_{}", uuid::Uuid::new_v4().simple());
        let info = QuestionInfo {
            id: request_id.clone(),
            session_id: session_id.clone(),
            questions: questions.iter().map(|q| q.question.clone()).collect(),
            options: normalize_question_options(&questions),
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
            label: Some(format!("Question ({})", info.questions.len())),
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

    pub(crate) async fn list_questions(&self) -> Vec<QuestionInfo> {
        let executions = self.executions.read().await;
        let mut result = executions
            .values()
            .filter_map(question_record_to_info)
            .collect::<Vec<_>>();
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    pub(crate) async fn list_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
        let executions = self.executions.read().await;
        let mut result = executions
            .values()
            .filter(|record| record.session_id == session_id)
            .filter_map(question_record_to_info)
            .collect::<Vec<_>>();
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }

    pub(crate) async fn list_session_execution_topology(
        &self,
        session_id: &str,
    ) -> SessionExecutionTopology {
        build_session_execution_topology(
            session_id.to_string(),
            self.list_session_execution_records(session_id).await,
        )
    }

    pub(crate) async fn list_session_execution_records(
        &self,
        session_id: &str,
    ) -> Vec<ExecutionRecord> {
        let executions = self.executions.read().await;
        executions
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Return all active execution records across every session.
    pub(crate) async fn list_all_executions(&self) -> Vec<ExecutionRecord> {
        let executions = self.executions.read().await;
        executions.values().cloned().collect()
    }

    /// Return the set of session IDs that currently have at least one active
    /// (non-Done) execution record.
    pub(crate) async fn list_active_session_ids(&self) -> Vec<String> {
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
    pub(crate) async fn has_cancellation_token(&self, execution_id: &str) -> bool {
        self.execution_tokens
            .lock()
            .await
            .contains_key(execution_id)
    }

    pub(crate) async fn answer_question(
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

    pub(crate) async fn reject_question(&self, id: &str) -> Option<QuestionInfo> {
        let info = self.take_question(id).await?;
        if let Some(waiter) = self.question_waiters.lock().await.remove(id) {
            let _ = waiter.send(QuestionReply::Rejected);
        }
        Some(info)
    }

    pub(crate) async fn cancel_questions_for_session(&self, session_id: &str) -> Vec<QuestionInfo> {
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

    pub(crate) async fn drop_question(&self, id: &str) {
        self.finish_execution(id).await;
        self.question_waiters.lock().await.remove(id);
    }

    /// Resolve the `stage_id` for a given execution by looking it up in the registry.
    pub(crate) async fn resolve_stage_id(&self, execution_id: &str) -> Option<String> {
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

pub(crate) fn build_session_execution_topology(
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
        .collect::<Vec<_>>();
    for record in records {
        let has_parent = record
            .parent_id
            .as_ref()
            .map(|parent_id| record_ids.iter().any(|id| id == parent_id))
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
        ExecutionKind::SchedulerStage => 2,
        ExecutionKind::ToolCall => 3,
        ExecutionKind::AgentTask => 4,
        ExecutionKind::Question => 5,
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

impl RuntimeControlRegistry {
    pub(crate) fn tool_call_execution_id(tool_call_id: &str) -> String {
        format!("tool_call:{tool_call_id}")
    }

    pub(crate) fn agent_task_execution_id(task_id: &str) -> String {
        format!("agent_task:{task_id}")
    }

    /// Count agent tasks under a given stage, returning `(done_count, total_count)`.
    pub(crate) async fn count_stage_agents(&self, stage_id: &str) -> (u32, u32) {
        let executions = self.executions.read().await;
        let mut done: u32 = 0;
        let mut total: u32 = 0;
        for record in executions.values() {
            if record.kind == ExecutionKind::AgentTask
                && record.stage_id.as_deref() == Some(stage_id)
            {
                total += 1;
                if record.status == ExecutionStatus::Done {
                    done += 1;
                }
            }
        }
        (done, total)
    }

    pub(crate) async fn count_active_stage_tools(&self, stage_id: &str) -> u32 {
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

fn normalize_question_options(questions: &[rocode_tool::QuestionDef]) -> Option<Vec<Vec<String>>> {
    let options: Vec<Vec<String>> = questions
        .iter()
        .map(|q| {
            q.options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    if options.iter().all(|entry| entry.is_empty()) {
        None
    } else {
        Some(options)
    }
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
        let questions = vec![rocode_tool::QuestionDef {
            question: "Pick one".to_string(),
            header: Some("Need".to_string()),
            options: vec![rocode_tool::QuestionOption {
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
            .register_scheduler_stage(
                "ses_1",
                "msg_stage_1".to_string(),
                "Coordination Gate".to_string(),
                serde_json::json!({
                    "scheduler_profile": "atlas",
                    "stage_name": "coordination-gate",
                    "stage_index": 2
                }),
            )
            .await;

        let (question, _) = registry
            .register_question(
                "ses_1".to_string(),
                vec![rocode_tool::QuestionDef {
                    question: "Approve?".to_string(),
                    header: Some("Decision".to_string()),
                    options: vec![rocode_tool::QuestionOption {
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
        let stage = scheduler
            .children
            .iter()
            .find(|node| matches!(node.kind, ExecutionKind::SchedulerStage))
            .expect("stage child");
        let question_node = stage
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
    async fn agent_task_lifecycle_register_and_finish() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_agent_task(
                "a1",
                "ses_1",
                "planner",
                Some(prompt_execution_id("ses_1")),
                None,
            )
            .await;

        let records = registry.list_session_execution_records("ses_1").await;
        let task_record = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::AgentTask))
            .expect("agent task should be registered");
        assert_eq!(task_record.id, "agent_task:a1");
        assert!(matches!(task_record.status, ExecutionStatus::Running));
        assert_eq!(task_record.label.as_deref(), Some("Agent: planner"));

        registry.finish_agent_task("a1").await;
        let records = registry.list_session_execution_records("ses_1").await;
        let agent_after = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::AgentTask))
            .expect("agent task should still exist with Done status");
        assert!(
            matches!(agent_after.status, ExecutionStatus::Done),
            "agent task should be Done after finish"
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
    async fn tool_call_appears_under_scheduler_stage_in_topology() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", CancellationToken::new(), None)
            .await;
        let stage_id = "msg_stage_plan".to_string();
        registry
            .register_scheduler_stage(
                "ses_1",
                stage_id.clone(),
                "Plan".to_string(),
                serde_json::json!({}),
            )
            .await;
        registry
            .register_tool_call(
                "tc_read",
                "ses_1",
                "read_file",
                Some(stage_id.clone()),
                Some(stage_id.clone()),
            )
            .await;

        let topology = registry.list_session_execution_topology("ses_1").await;
        assert_eq!(topology.active_count, 4); // prompt + scheduler + stage + tool
        let prompt = &topology.roots[0];
        let scheduler = &prompt.children[0];
        let stage = &scheduler.children[0];
        let tool = stage
            .children
            .iter()
            .find(|n| matches!(n.kind, ExecutionKind::ToolCall))
            .expect("tool call under stage");
        assert_eq!(tool.id, "tool_call:tc_read");
    }

    #[tokio::test]
    async fn stage_id_propagates_through_tool_and_agent_registration() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", CancellationToken::new(), None)
            .await;
        let stage_id = "stage_verify".to_string();
        registry
            .register_scheduler_stage(
                "ses_1",
                stage_id.clone(),
                "Verify".to_string(),
                serde_json::json!({}),
            )
            .await;

        // Register tool and agent with stage_id.
        registry
            .register_tool_call(
                "tc_1",
                "ses_1",
                "read_file",
                Some(stage_id.clone()),
                Some(stage_id.clone()),
            )
            .await;
        registry
            .register_agent_task(
                "at_1",
                "ses_1",
                "planner",
                Some(stage_id.clone()),
                Some(stage_id.clone()),
            )
            .await;

        let records = registry.list_session_execution_records("ses_1").await;

        // Stage itself has stage_id = its own id.
        let stage_record = records
            .iter()
            .find(|r| r.id == "stage_verify")
            .expect("stage record");
        assert_eq!(stage_record.stage_id.as_deref(), Some("stage_verify"));

        // Tool call carries stage_id.
        let tool_record = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::ToolCall))
            .expect("tool record");
        assert_eq!(tool_record.stage_id.as_deref(), Some("stage_verify"));

        // Agent task carries stage_id.
        let agent_record = records
            .iter()
            .find(|r| matches!(r.kind, ExecutionKind::AgentTask))
            .expect("agent record");
        assert_eq!(agent_record.stage_id.as_deref(), Some("stage_verify"));

        // Topology nodes carry stage_id.
        let topology = registry.list_session_execution_topology("ses_1").await;
        let prompt = &topology.roots[0];
        let scheduler = &prompt.children[0];
        let stage_node = &scheduler.children[0];
        assert_eq!(stage_node.stage_id.as_deref(), Some("stage_verify"));
        let tool_node = stage_node
            .children
            .iter()
            .find(|n| matches!(n.kind, ExecutionKind::ToolCall))
            .expect("tool node");
        assert_eq!(tool_node.stage_id.as_deref(), Some("stage_verify"));
    }

    #[tokio::test]
    async fn question_inherits_stage_id_from_parent() {
        let registry = RuntimeControlRegistry::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_scheduler_run("ses_1", CancellationToken::new(), None)
            .await;
        let stage_id = "stage_gate".to_string();
        registry
            .register_scheduler_stage(
                "ses_1",
                stage_id.clone(),
                "Gate".to_string(),
                serde_json::json!({}),
            )
            .await;

        let (question, _rx) = registry
            .register_question(
                "ses_1".to_string(),
                vec![rocode_tool::QuestionDef {
                    question: "Approve?".to_string(),
                    header: None,
                    options: vec![],
                    multiple: false,
                }],
            )
            .await;

        let records = registry.list_session_execution_records("ses_1").await;
        let q_record = records
            .iter()
            .find(|r| r.id == question.id)
            .expect("question record");
        assert_eq!(
            q_record.stage_id.as_deref(),
            Some("stage_gate"),
            "question should inherit stage_id from its parent stage"
        );
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
        registry
            .register_scheduler_stage(
                "ses_1",
                "stage_x".to_string(),
                "Test".to_string(),
                serde_json::json!({}),
            )
            .await;
        registry
            .register_tool_call(
                "tc_1",
                "ses_1",
                "bash",
                Some("stage_x".to_string()),
                Some("stage_x".to_string()),
            )
            .await;

        let captured = events.lock().unwrap();
        // Find the tool call event.
        let tool_event = captured
            .iter()
            .find(|(_, eid, _)| eid == "tool_call:tc_1")
            .expect("should have captured tool call event");
        assert_eq!(tool_event.0, "ses_1");
        assert_eq!(tool_event.2, Some("stage_x".to_string()));

        // Find the stage event.
        let stage_event = captured
            .iter()
            .find(|(_, eid, _)| eid == "stage_x")
            .expect("should have captured stage event");
        assert_eq!(stage_event.2, Some("stage_x".to_string()));
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
    async fn cancel_agent_task_triggers_cancellation_token() {
        let registry = RuntimeControlRegistry::new();
        let token = CancellationToken::new();
        registry
            .set_session_run_status("ses_1", SessionRunStatus::Busy)
            .await;
        registry
            .register_agent_task_with_token(
                "at_1",
                "ses_1",
                "planner",
                None,
                None,
                Some(token.clone()),
            )
            .await;
        assert!(!token.is_cancelled());
        assert!(registry.has_cancellation_token("agent_task:at_1").await);

        let kind = registry.cancel_execution("agent_task:at_1").await;
        assert_eq!(kind, Some(ExecutionKind::AgentTask));
        assert!(token.is_cancelled(), "token should be cancelled");

        // finish cleans up token
        registry.finish_agent_task("at_1").await;
        assert!(!registry.has_cancellation_token("agent_task:at_1").await);
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

    #[test]
    fn execution_record_to_node_projects_correctly() {
        let record = ExecutionRecord {
            id: "exec_123".to_string(),
            session_id: "sess_abc".to_string(),
            kind: ExecutionKind::AgentTask,
            status: ExecutionStatus::Running,
            label: Some("planner".to_string()),
            parent_id: Some("exec_000".to_string()),
            stage_id: Some("stage_xyz".to_string()),
            waiting_on: None,
            recent_event: Some("tool_call".to_string()),
            started_at: 1710000000000,
            updated_at: 1710000001000,
            metadata: Some(serde_json::json!({
                "scheduler_stage_id": "stage_xyz",
                "child_session_id": "child_001"
            })),
        };

        let node = record.to_node();
        assert_eq!(node.execution_id, "exec_123");
        assert_eq!(node.parent_execution_id, Some("exec_000".to_string()));
        assert_eq!(node.stage_id, Some("stage_xyz".to_string()));
        assert_eq!(node.kind, ExecutionNodeKind::Agent);
        assert_eq!(node.status, ExecutionNodeStatus::Running);
        assert_eq!(node.label, Some("planner".to_string()));
        assert_eq!(node.session_id, "sess_abc");
        assert_eq!(node.child_session_id, Some("child_001".to_string()));
        assert_eq!(node.started_at, 1710000000000);
        assert_eq!(node.updated_at, 1710000001000);
    }

    #[test]
    fn execution_record_to_node_handles_missing_metadata() {
        let record = ExecutionRecord {
            id: "exec_456".to_string(),
            session_id: "sess_def".to_string(),
            kind: ExecutionKind::ToolCall,
            status: ExecutionStatus::Waiting,
            label: None,
            parent_id: None,
            stage_id: None,
            waiting_on: Some("user_approval".to_string()),
            recent_event: None,
            started_at: 1710000000000,
            updated_at: 1710000000500,
            metadata: None,
        };

        let node = record.to_node();
        assert_eq!(node.execution_id, "exec_456");
        assert_eq!(node.parent_execution_id, None);
        assert_eq!(node.stage_id, None);
        assert_eq!(node.kind, ExecutionNodeKind::Tool);
        assert_eq!(node.status, ExecutionNodeStatus::Waiting);
        assert_eq!(node.waiting_on, Some("user_approval".to_string()));
        assert_eq!(node.child_session_id, None);
    }

    #[test]
    fn execution_record_to_node_maps_all_kinds() {
        let make_record = |kind: ExecutionKind| ExecutionRecord {
            id: "e".to_string(),
            session_id: "s".to_string(),
            kind,
            status: ExecutionStatus::Running,
            label: None,
            parent_id: None,
            stage_id: None,
            waiting_on: None,
            recent_event: None,
            started_at: 0,
            updated_at: 0,
            metadata: None,
        };

        assert_eq!(
            make_record(ExecutionKind::PromptRun).to_node().kind,
            ExecutionNodeKind::Stage
        );
        assert_eq!(
            make_record(ExecutionKind::SchedulerRun).to_node().kind,
            ExecutionNodeKind::Stage
        );
        assert_eq!(
            make_record(ExecutionKind::SchedulerStage).to_node().kind,
            ExecutionNodeKind::Stage
        );
        assert_eq!(
            make_record(ExecutionKind::AgentTask).to_node().kind,
            ExecutionNodeKind::Agent
        );
        assert_eq!(
            make_record(ExecutionKind::ToolCall).to_node().kind,
            ExecutionNodeKind::Tool
        );
        assert_eq!(
            make_record(ExecutionKind::Question).to_node().kind,
            ExecutionNodeKind::Question
        );
    }
}
