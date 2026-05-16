pub(crate) mod events;
pub(crate) mod memory;
pub(crate) mod stage_summary;
pub(crate) mod state;
pub(crate) mod steering;
pub(crate) mod telemetry;

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use self::events::{broadcast_session_updated, emit_output_block_via_hook, DiffEntry};
use crate::runtime_control::{ExecutionPatch, ExecutionStatus, FieldUpdate, SessionRunStatus};
use crate::ServerState;
use rocode_command::output_blocks::{
    MessageBlock, MessageRole as OutputMessageRole, OutputBlock, ReasoningBlock,
    SchedulerDecisionBlock, SchedulerDecisionField, SchedulerDecisionRenderSpec,
    SchedulerDecisionSection, SchedulerStageBlock,
};
use rocode_orchestrator::runtime::StepCheckpointDirective;
use rocode_orchestrator::{
    parse_execution_gate_decision, parse_route_decision, scheduler_stage_observability,
    ExecutionContext as OrchestratorExecutionContext, LifecycleHook, RouteDecision,
    SchedulerExecutionGateDecision, SchedulerStageCapabilities,
    ToolOutput as OrchestratorToolOutput,
};
use rocode_provider::Provider;
use rocode_session::prompt::{OutputBlockEvent, OutputBlockHook};
use rocode_session::snapshot::Snapshot;
use rocode_session::{
    MessageRole, MessageUsage, PartType, Session, SessionContextKind, SessionMessage,
};

const SCHEDULER_CONTEXT_HYDRATE_TOOL: &str = "scheduler_context_hydrate";
const SCHEDULER_MEMORY_HYDRATE_TOOL: &str = "scheduler_memory_hydrate";
const SCHEDULER_STAGE_CONTEXT_HYDRATION_IDS_METADATA_KEY: &str =
    "scheduler_stage_context_hydrated_message_ids";
const SCHEDULER_STAGE_CONTEXT_HYDRATION_EVENTS_METADATA_KEY: &str =
    "scheduler_stage_context_hydration_events";
const SCHEDULER_STAGE_MEMORY_HYDRATION_IDS_METADATA_KEY: &str =
    "scheduler_stage_memory_hydrated_record_ids";
const SCHEDULER_STAGE_MEMORY_HYDRATION_EVENTS_METADATA_KEY: &str =
    "scheduler_stage_memory_hydration_events";

#[derive(Clone)]
struct ActiveStageMessage {
    message_id: String,
    execution_id: String,
    stage_name: String,
    step_count: u32,
    committed_usage: rocode_orchestrator::runtime::events::StepUsage,
    live_usage: rocode_orchestrator::runtime::events::StepUsage,
    /// If this stage creates an isolated attached session, its session ID.
    attached_session_id: Option<String>,
    /// The assistant message ID within the attached session where content flows.
    attached_message_id: Option<String>,
    /// Whether a reasoning stream has started for the attached-session assistant message.
    attached_reasoning_started: bool,
    /// Whether a reasoning stream has started for the main session message.
    reasoning_started: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ModelPricing {
    input_per_million: f64,
    output_per_million: f64,
    /// Per-million cost for cache-read tokens. Falls back to `input_per_million`
    /// when the provider does not publish a separate cache-read price.
    cache_read_per_million: f64,
    /// Per-million cost for cache-write tokens. Falls back to `input_per_million`
    /// when the provider does not publish a separate cache-write price.
    cache_write_per_million: f64,
}

impl ModelPricing {
    pub(crate) fn new(
        input_per_million: f64,
        output_per_million: f64,
        cache_read_per_million: Option<f64>,
        cache_write_per_million: Option<f64>,
    ) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_read_per_million: cache_read_per_million.unwrap_or(input_per_million),
            cache_write_per_million: cache_write_per_million.unwrap_or(input_per_million),
        }
    }

    /// Build from the runtime `ModelInfo`.
    ///
    /// The runtime struct currently only carries input/output prices, so cache
    /// prices fall back to the input price — matching the original
    /// `ModelCost::compute` behaviour (`unwrap_or(self.input)`).
    pub(crate) fn from_model_info(info: &rocode_provider::ModelInfo) -> Self {
        Self::new(
            info.cost_per_million_input,
            info.cost_per_million_output,
            info.cost_per_million_cache_read,
            info.cost_per_million_cache_write,
        )
    }

    pub(crate) fn compute(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_miss_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let uncached_input_tokens = if cache_miss_tokens > 0 {
            cache_miss_tokens
        } else {
            input_tokens
        };
        let input_cost = self.input_per_million * (uncached_input_tokens as f64) / 1_000_000.0;
        let output_cost = self.output_per_million * (output_tokens as f64) / 1_000_000.0;
        let cache_read_cost =
            self.cache_read_per_million * (cache_read_tokens as f64) / 1_000_000.0;
        let cache_write_cost =
            self.cache_write_per_million * (cache_write_tokens as f64) / 1_000_000.0;
        input_cost + output_cost + cache_read_cost + cache_write_cost
    }
}

#[derive(Clone)]
pub(crate) struct SessionSchedulerLifecycleHook {
    state: Arc<ServerState>,
    session_id: String,
    scheduler_profile: String,
    output_hook: Option<OutputBlockHook>,
    /// Tracks the currently streaming stage messages as a stack.
    active_stage_messages: Arc<Mutex<Vec<ActiveStageMessage>>>,
    /// Model pricing info for cost calculation.
    model_pricing: Option<ModelPricing>,
}

impl SessionSchedulerLifecycleHook {
    pub(crate) fn new(
        state: Arc<ServerState>,
        session_id: String,
        scheduler_profile: String,
    ) -> Self {
        Self {
            state,
            session_id,
            scheduler_profile,
            output_hook: None,
            active_stage_messages: Arc::new(Mutex::new(Vec::new())),
            model_pricing: None,
        }
    }

    pub(crate) fn with_model_pricing(mut self, model_pricing: Option<ModelPricing>) -> Self {
        self.model_pricing = model_pricing;
        self
    }

    pub(crate) fn with_output_hook(mut self, output_hook: Option<OutputBlockHook>) -> Self {
        self.output_hook = output_hook;
        self
    }

    async fn emit_stage_message(
        &self,
        stage_name: &str,
        stage_index: u32,
        stage_total: u32,
        content: &str,
        exec_ctx: &OrchestratorExecutionContext,
    ) {
        emit_scheduler_stage_message(SchedulerStageMessageInput {
            state: &self.state,
            session_id: &self.session_id,
            scheduler_profile: &self.scheduler_profile,
            stage_name,
            stage_index,
            stage_total,
            content,
            exec_ctx,
            output_hook: self.output_hook.as_ref(),
        })
        .await;
    }

    async fn update_active_stage_message<F>(&self, mut update: F, _source: &'static str)
    where
        F: FnMut(&mut SessionMessage, &mut ActiveStageMessage),
    {
        // Snapshot the active entry under a brief lock.
        let active = {
            let guard = self.active_stage_messages.lock().await;
            guard.last().cloned()
        };
        let Some(mut active) = active else {
            return;
        };

        let mut sessions = self.state.sessions.lock().await;
        let Some(mut session) = sessions.get(&self.session_id).cloned() else {
            return;
        };

        let mut runtime_patch = None;
        let mut execution_id = None;
        let mut message_snapshot = None;
        if let Some(message) = session.get_message_mut(&active.message_id) {
            update(message, &mut active);
            runtime_patch = Some(stage_execution_patch_from_message(message));
            execution_id = Some(active.execution_id.clone());
            message_snapshot = Some(message.clone());
            session.touch();
            sessions.update(session);
            drop(sessions);

            // Write the updated snapshot back to the canonical entry.
            let mut guard = self.active_stage_messages.lock().await;
            if let Some(last) = guard.last_mut() {
                if last.message_id == active.message_id {
                    *last = active;
                }
            }
        }

        if let Some(message) = message_snapshot.as_ref() {
            self.emit_stage_block(message).await;
        }

        if message_snapshot.is_some() {
            broadcast_session_updated(
                self.state.as_ref(),
                self.session_id.clone(),
                _source.to_string(),
            );
        }

        if let (Some(execution_id), Some(patch)) = (execution_id, runtime_patch) {
            self.state
                .runtime_control
                .update_scheduler_stage(&execution_id, patch)
                .await;
        }
    }

    async fn emit_stage_block(&self, message: &SessionMessage) {
        let _ = self
            .state
            .runtime_telemetry
            .refresh_stage_summary_from_message(&self.session_id, message)
            .await;
        if let Some(block) = scheduler_stage_block_from_message(message) {
            self.emit_realtime_block(OutputBlockEvent {
                session_id: self.session_id.clone(),
                block: OutputBlock::SchedulerStage(Box::new(block)),
                id: Some(message.id.clone()),
            })
            .await;
        }
    }

    async fn emit_realtime_block(&self, event: OutputBlockEvent) {
        emit_output_block_via_hook(self.output_hook.as_ref(), event).await;
    }

    async fn emit_output_block(&self, session_id: String, block: OutputBlock, id: Option<String>) {
        self.emit_realtime_block(OutputBlockEvent {
            session_id,
            block,
            id,
        })
        .await;
    }

    async fn resolve_step_checkpoint_provider(
        &self,
    ) -> Result<Option<(Arc<dyn Provider>, String)>, rocode_orchestrator::OrchestratorError> {
        let (provider_id, model_id) = {
            let sessions = self.state.sessions.lock().await;
            let Some(session) = sessions.get(&self.session_id) else {
                return Ok(None);
            };
            let provider_id = session
                .record()
                .metadata
                .get("model_provider")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let model_id = session
                .record()
                .metadata
                .get("model_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            match provider_id.zip(model_id) {
                Some(value) => value,
                None => return Ok(None),
            }
        };

        let providers = self.state.providers.read().await;
        let provider = providers.get_provider(&provider_id).map_err(|error| {
            rocode_orchestrator::OrchestratorError::from_provider_error(
                &provider_id,
                Some(&model_id),
                &error,
            )
        })?;
        Ok(Some((provider, model_id)))
    }

    async fn govern_step_checkpoint(
        &self,
        request_context_tokens: Option<u64>,
        request_body_chars: Option<usize>,
        compaction_attempted: bool,
        compaction_succeeded: bool,
    ) -> Result<
        Option<rocode_types::ContextPressureGovernanceSummary>,
        rocode_orchestrator::OrchestratorError,
    > {
        let Some((provider, model_id)) = self.resolve_step_checkpoint_provider().await? else {
            return Ok(None);
        };

        let summary = {
            let mut sessions = self.state.sessions.lock().await;
            let Some(mut session) = sessions.get(&self.session_id).cloned() else {
                return Ok(None);
            };
            let summary = rocode_session::prompt::assess_request_view_context_governance(
                provider.as_ref(),
                &model_id,
                None,
                Some(self.state.config_store.as_ref()),
                None,
                request_context_tokens,
                request_body_chars,
                "step_checkpoint_gate",
                "scheduler.step_checkpoint",
                compaction_attempted,
                compaction_succeeded,
            );
            rocode_session::prompt::record_context_pressure_governance_summary(
                &mut session,
                &summary,
            );
            sessions.update(session);
            summary
        };

        Ok(Some(summary))
    }

    /// Capture a git worktree snapshot and store its hash in the active stage
    /// message metadata under the given key.
    ///
    /// Runs `Snapshot::track()` on the session's worktree directory in a
    /// blocking task to avoid stalling the async runtime.
    async fn track_snapshot(&self, metadata_key: &str) {
        let worktree = {
            let sessions = self.state.sessions.lock().await;
            sessions.get(&self.session_id).map(|s| s.directory.clone())
        };
        let Some(worktree) = worktree else {
            return;
        };

        let worktree_path = std::path::PathBuf::from(&worktree);
        let snapshot_hash =
            tokio::task::spawn_blocking(move || Snapshot::track(&worktree_path)).await;

        let hash = match snapshot_hash {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                tracing::debug!("Snapshot::track() failed: {e}");
                return;
            }
            Err(e) => {
                tracing::debug!("Snapshot::track() task panicked: {e}");
                return;
            }
        };

        let key = metadata_key.to_string();
        self.update_active_stage_message(
            move |message, _active| {
                message
                    .metadata
                    .insert(key.clone(), serde_json::json!(hash));
            },
            "prompt.scheduler.snapshot",
        )
        .await;
    }
}

fn format_step_checkpoint_context_pressure_error(
    summary: &rocode_types::ContextPressureGovernanceSummary,
) -> String {
    let reason = summary
        .reason
        .as_deref()
        .unwrap_or("context pressure remained above the safe step checkpoint limit");
    let mut details = Vec::new();
    if let Some(request_context_tokens) = summary.request_context_tokens {
        details.push(format!("request={request_context_tokens}"));
    }
    if let Some(live_context_tokens) = summary.live_context_tokens {
        details.push(format!("live={live_context_tokens}"));
    }
    if let Some(limit_tokens) = summary.limit_tokens {
        details.push(format!("limit={limit_tokens}"));
    }
    if let Some(body_chars) = summary.body_chars {
        details.push(format!("body_chars={body_chars}"));
    }

    if details.is_empty() {
        format!("Context pressure gate blocked the next scheduler step ({reason}).")
    } else {
        format!(
            "Context pressure gate blocked the next scheduler step ({reason}; {}).",
            details.join(", ")
        )
    }
}

fn directive_from_context_pressure_summary(
    summary: &rocode_types::ContextPressureGovernanceSummary,
    stage_name: Option<&str>,
    checkpoint: &rocode_orchestrator::runtime::events::StepCheckpointSnapshot,
) -> StepCheckpointDirective {
    if !checkpoint.compaction_attempted()
        && matches!(
            summary.status,
            rocode_types::ContextPressureGovernanceStatus::Deferred
        )
    {
        return StepCheckpointDirective::CompactRequestView {
            focus: stage_name.map(str::to_string),
            reason: summary.reason.clone(),
        };
    }

    if summary.blocking {
        return StepCheckpointDirective::Block {
            reason: format_step_checkpoint_context_pressure_error(summary),
        };
    }

    StepCheckpointDirective::Continue
}

fn same_step_checkpoint_directive(
    left: &StepCheckpointDirective,
    right: &StepCheckpointDirective,
) -> bool {
    match (left, right) {
        (StepCheckpointDirective::Continue, StepCheckpointDirective::Continue) => true,
        (
            StepCheckpointDirective::Block {
                reason: left_reason,
            },
            StepCheckpointDirective::Block {
                reason: right_reason,
            },
        ) => left_reason == right_reason,
        (
            StepCheckpointDirective::CompactRequestView {
                focus: left_focus,
                reason: left_reason,
            },
            StepCheckpointDirective::CompactRequestView {
                focus: right_focus,
                reason: right_reason,
            },
        ) => left_focus == right_focus && left_reason == right_reason,
        (
            StepCheckpointDirective::ReplaceRequestView {
                messages: left_messages,
                reason: left_reason,
            },
            StepCheckpointDirective::ReplaceRequestView {
                messages: right_messages,
                reason: right_reason,
            },
        ) => {
            serde_json::to_value(left_messages).ok() == serde_json::to_value(right_messages).ok()
                && left_reason == right_reason
        }
        _ => false,
    }
}

fn preferred_checkpoint_request_context_tokens(
    checkpoint: &rocode_orchestrator::runtime::events::StepCheckpointSnapshot,
    usage: &rocode_orchestrator::runtime::events::StepUsage,
) -> Option<u64> {
    let estimated = checkpoint.current_view.estimated_context_tokens;
    let observed = Some(usage.context_tokens.max(usage.prompt_tokens)).filter(|tokens| *tokens > 0);
    match (estimated, observed) {
        (Some(estimated), Some(observed)) => Some(estimated.max(observed)),
        (Some(estimated), None) => Some(estimated),
        (None, Some(observed)) => Some(observed),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SchedulerAbortInfo {
    pub execution_id: Option<String>,
    pub scheduler_profile: Option<String>,
    pub stage_name: Option<String>,
    pub stage_index: Option<u32>,
}

pub(crate) async fn request_active_scheduler_stage_abort(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Option<SchedulerAbortInfo> {
    let info = update_active_scheduler_stage_message(
        state,
        session_id,
        |message| {
            let info = scheduler_abort_info(message);
            message.metadata.insert(
                "scheduler_stage_status".to_string(),
                serde_json::json!("cancelling"),
            );
            message.metadata.insert(
                "scheduler_stage_waiting_on".to_string(),
                serde_json::json!("none"),
            );
            message.metadata.insert(
                "scheduler_stage_last_event".to_string(),
                serde_json::json!("Cancellation requested by user"),
            );
            Some(info)
        },
        "prompt.scheduler.stage.abort.requested",
    )
    .await;
    if let Some(execution_id) = info.as_ref().and_then(|info| info.execution_id.as_deref()) {
        state
            .runtime_telemetry
            .mark_scheduler_stage_cancelling(execution_id)
            .await;
    }
    info
}

pub(crate) async fn finalize_active_scheduler_stage_cancelled(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Option<SchedulerAbortInfo> {
    let info = update_active_scheduler_stage_message(
        state,
        session_id,
        |message| {
            let info = scheduler_abort_info(message);
            message.metadata.remove("scheduler_stage_streaming");
            message.metadata.insert(
                "scheduler_stage_status".to_string(),
                serde_json::json!("cancelled"),
            );
            message.metadata.insert(
                "scheduler_stage_waiting_on".to_string(),
                serde_json::json!("none"),
            );
            message.metadata.insert(
                "scheduler_stage_last_event".to_string(),
                serde_json::json!("Stage cancelled by user"),
            );
            Some(info)
        },
        "prompt.scheduler.stage.abort.finalized",
    )
    .await;
    if let Some(execution_id) = info.as_ref().and_then(|info| info.execution_id.as_deref()) {
        state
            .runtime_telemetry
            .finish_scheduler_stage(execution_id)
            .await;
    }
    info
}

async fn update_active_scheduler_stage_message<T, F>(
    state: &Arc<ServerState>,
    session_id: &str,
    mut update: F,
    source: &'static str,
) -> Option<T>
where
    F: FnMut(&mut SessionMessage) -> Option<T>,
{
    let mut sessions = state.sessions.lock().await;
    let mut session = sessions.get(session_id).cloned()?;
    let message = find_active_scheduler_stage_message_mut(&mut session)?;
    let result = update(message)?;
    let message_snapshot = message.clone();
    session.touch();
    sessions.update(session);
    drop(sessions);

    let _ = state
        .runtime_telemetry
        .refresh_stage_summary_from_message(session_id, &message_snapshot)
        .await;
    broadcast_session_updated(state, session_id.to_string(), source.to_string());
    Some(result)
}

fn find_active_scheduler_stage_message_mut(session: &mut Session) -> Option<&mut SessionMessage> {
    session.messages_mut().iter_mut().rev().find(|message| {
        message.role == MessageRole::Assistant
            && message
                .metadata
                .get("scheduler_stage_emitted")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            && (message
                .metadata
                .get("scheduler_stage_streaming")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
                || matches!(
                    message
                        .metadata
                        .get("scheduler_stage_status")
                        .and_then(|value| value.as_str()),
                    Some("running" | "waiting" | "cancelling")
                ))
    })
}

fn scheduler_abort_info(message: &SessionMessage) -> SchedulerAbortInfo {
    SchedulerAbortInfo {
        execution_id: message
            .metadata
            .get("scheduler_stage_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        scheduler_profile: message
            .metadata
            .get("scheduler_profile")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        stage_name: message
            .metadata
            .get("scheduler_stage")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        stage_index: message
            .metadata
            .get("scheduler_stage_index")
            .and_then(|value| value.as_u64())
            .map(|value| value as u32),
    }
}

fn write_stage_usage_totals(
    message: &mut SessionMessage,
    committed_usage: &rocode_orchestrator::runtime::events::StepUsage,
    live_usage: &rocode_orchestrator::runtime::events::StepUsage,
    allow_zero_fields: bool,
    model_pricing: Option<ModelPricing>,
) {
    let prompt_tokens = committed_usage.prompt_tokens + live_usage.prompt_tokens;
    let completion_tokens = committed_usage.completion_tokens + live_usage.completion_tokens;
    let context_tokens = committed_usage
        .context_tokens
        .max(live_usage.context_tokens)
        .max(prompt_tokens);
    let reasoning_tokens = committed_usage.reasoning_tokens + live_usage.reasoning_tokens;
    let cache_read_tokens = committed_usage.cache_read_tokens + live_usage.cache_read_tokens;
    let cache_miss_tokens = committed_usage.cache_miss_tokens + live_usage.cache_miss_tokens;
    let cache_write_tokens = committed_usage.cache_write_tokens + live_usage.cache_write_tokens;

    let mut has_any_visible_usage = false;
    for (key, value) in [
        ("scheduler_stage_prompt_tokens", prompt_tokens),
        ("scheduler_stage_context_tokens", context_tokens),
        ("scheduler_stage_completion_tokens", completion_tokens),
        ("scheduler_stage_reasoning_tokens", reasoning_tokens),
        ("scheduler_stage_cache_read_tokens", cache_read_tokens),
        ("scheduler_stage_cache_miss_tokens", cache_miss_tokens),
        ("scheduler_stage_cache_write_tokens", cache_write_tokens),
    ] {
        if value > 0 || allow_zero_fields {
            has_any_visible_usage = true;
            message
                .metadata
                .insert(key.to_string(), serde_json::json!(value));
        } else {
            message.metadata.remove(key);
        }
    }

    if has_any_visible_usage {
        let total_cost = model_pricing
            .map(|pricing| {
                pricing.compute(
                    prompt_tokens,
                    completion_tokens,
                    cache_read_tokens,
                    cache_miss_tokens,
                    cache_write_tokens,
                )
            })
            .or_else(|| message.usage.as_ref().map(|u| u.total_cost))
            .unwrap_or(0.0);
        message.usage = Some(MessageUsage {
            input_tokens: prompt_tokens,
            output_tokens: completion_tokens,
            reasoning_tokens,
            cache_write_tokens,
            cache_read_tokens,
            cache_miss_tokens,
            context_tokens,
            total_cost,
        });
    } else {
        message.usage = None;
    }
}

/// Returns `true` for tools that modify files on disk (edit, write, apply_patch).
/// These are the tools that warrant capturing a snapshot after completion.
fn is_file_modifying_tool(tool_name: &str) -> bool {
    let lower = tool_name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "write"
            | "writefile"
            | "write_file"
            | "edit"
            | "editfile"
            | "edit_file"
            | "apply_patch"
            | "applypatch"
    )
}

#[async_trait]
impl LifecycleHook for SessionSchedulerLifecycleHook {
    async fn on_orchestration_start(
        &self,
        _: &str,
        _: Option<u32>,
        _: &OrchestratorExecutionContext,
    ) {
        // Intentionally empty for now. Session-level snapshotting is anchored
        // at the first step start and at successful file-modifying tool
        // completion so we do not mutate session state before the scheduler
        // has actually begun work. Future orchestration-level metadata can be
        // recorded here without touching prompt-visible context.
    }

    async fn on_step_start(
        &self,
        _: &str,
        _: &str,
        step_index: u32,
        _: &OrchestratorExecutionContext,
    ) {
        // Capture a "before" snapshot at the first step so compute_diff()
        // can later compare against the final snapshot.
        if step_index == 0 {
            self.track_snapshot("step_start_snapshot").await;
        }

        self.update_active_stage_message(
            |message, active| {
                active.step_count += 1;
                active.live_usage = rocode_orchestrator::runtime::events::StepUsage::default();
                write_stage_usage_totals(
                    message,
                    &active.committed_usage,
                    &active.live_usage,
                    false,
                    None,
                );
                message.metadata.insert(
                    "scheduler_stage_step".to_string(),
                    serde_json::json!(active.step_count),
                );
                message.metadata.insert(
                    "scheduler_stage_status".to_string(),
                    serde_json::json!("running"),
                );
                message.metadata.insert(
                    "scheduler_stage_last_event".to_string(),
                    serde_json::json!(format!("Step {} started", active.step_count)),
                );
                message.metadata.insert(
                    "scheduler_stage_waiting_on".to_string(),
                    serde_json::json!("model"),
                );
            },
            "prompt.scheduler.stage.step",
        )
        .await;
    }

    async fn on_tool_start(
        &self,
        _: &str,
        tool_call_id: &str,
        tool_name: &str,
        tool_args: &serde_json::Value,
        _: &OrchestratorExecutionContext,
    ) {
        // Register tool call into RuntimeControlRegistry for topology visibility.
        let (parent_id, stage_id) = {
            let guard = self.active_stage_messages.lock().await;
            let pid = guard.last().map(|s| s.execution_id.clone());
            let sid = guard.last().map(|s| s.execution_id.clone());
            (pid, sid)
        };
        self.state
            .runtime_telemetry
            .register_tool_call(
                &self.session_id,
                tool_call_id,
                tool_name,
                parent_id,
                stage_id,
            )
            .await;

        self.update_active_stage_message(
            |message, _active| {
                apply_stage_capability_activity_evidence(
                    message,
                    extract_stage_capability_activity(
                        tool_name,
                        StageCapabilityActivitySource::ToolArgs(tool_args),
                    ),
                );
                if let Some(activity) = summarize_tool_activity(tool_name, tool_args) {
                    message.metadata.insert(
                        "scheduler_stage_activity".to_string(),
                        serde_json::json!(activity),
                    );
                }
                if tool_name.eq_ignore_ascii_case("question") {
                    message.metadata.insert(
                        "scheduler_stage_status".to_string(),
                        serde_json::json!("waiting"),
                    );
                    message.metadata.insert(
                        "scheduler_stage_waiting_on".to_string(),
                        serde_json::json!("user"),
                    );
                    message.metadata.insert(
                        "scheduler_stage_last_event".to_string(),
                        serde_json::json!("Waiting for user answer"),
                    );
                } else {
                    message.metadata.insert(
                        "scheduler_stage_status".to_string(),
                        serde_json::json!("running"),
                    );
                    message.metadata.insert(
                        "scheduler_stage_waiting_on".to_string(),
                        serde_json::json!("tool"),
                    );
                    message.metadata.insert(
                        "scheduler_stage_last_event".to_string(),
                        serde_json::json!(format!(
                            "Tool started: {}",
                            pretty_scheduler_stage_name(tool_name)
                        )),
                    );
                }
            },
            "prompt.scheduler.stage.tool.start",
        )
        .await;

        // Populate the TodoManager when a todowrite tool is invoked so the
        // /session/{id}/todo endpoint returns live data.
        if tool_name.eq_ignore_ascii_case("todowrite")
            || tool_name.eq_ignore_ascii_case("todo_write")
        {
            if let Some(todos) = extract_todo_items_from_args(tool_args) {
                self.state
                    .todo_manager
                    .update(&self.session_id, todos)
                    .await;
            }
        }
    }

    async fn on_tool_end(
        &self,
        _: &str,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &OrchestratorToolOutput,
        _: &OrchestratorExecutionContext,
    ) {
        // Remove tool call from RuntimeControlRegistry.
        self.state
            .runtime_telemetry
            .finish_tool_call(&self.session_id, tool_call_id)
            .await;

        let stage_id = {
            let guard = self.active_stage_messages.lock().await;
            guard.last().map(|stage| stage.execution_id.clone())
        };
        if let Err(error) = self
            .state
            .runtime_memory
            .ingest_tool_result(
                &self.session_id,
                tool_call_id,
                tool_name,
                stage_id.as_deref(),
                &tool_output.output,
                tool_output.is_error,
            )
            .await
        {
            tracing::warn!(
                session_id = %self.session_id,
                tool_call_id,
                tool_name,
                %error,
                "failed to persist runtime memory candidate from tool result"
            );
        }
        if tool_name.eq_ignore_ascii_case("skill_manage") {
            if let Err(error) = self
                .state
                .runtime_memory
                .ingest_skill_manage_result(
                    &self.session_id,
                    tool_call_id,
                    tool_output.metadata.as_ref(),
                )
                .await
            {
                tracing::warn!(
                    session_id = %self.session_id,
                    tool_call_id,
                    tool_name,
                    %error,
                    "failed to persist runtime skill memory linkage"
                );
            }
        }
        if let Err(error) = self
            .state
            .runtime_memory
            .ingest_runtime_loaded_skills(
                &self.session_id,
                tool_call_id,
                tool_name,
                stage_id.as_deref(),
                tool_output.metadata.as_ref(),
                &tool_output.output,
                tool_output.is_error,
            )
            .await
        {
            tracing::warn!(
                session_id = %self.session_id,
                tool_call_id,
                tool_name,
                %error,
                "failed to persist runtime loaded skill memory usage"
            );
        }

        self.update_active_stage_message(
            |message, _active| {
                apply_stage_capability_activity_evidence(
                    message,
                    extract_stage_capability_activity(
                        tool_name,
                        StageCapabilityActivitySource::ToolOutput(tool_output),
                    ),
                );
                if let Some(activity) = summarize_tool_result_activity(tool_name, tool_output) {
                    message.metadata.insert(
                        "scheduler_stage_activity".to_string(),
                        serde_json::json!(activity),
                    );
                }
                if tool_name.eq_ignore_ascii_case(SCHEDULER_CONTEXT_HYDRATE_TOOL) {
                    persist_scheduler_context_hydration_event(
                        message,
                        tool_call_id,
                        tool_output.metadata.as_ref(),
                    );
                }
                if tool_name.eq_ignore_ascii_case(SCHEDULER_MEMORY_HYDRATE_TOOL) {
                    persist_scheduler_memory_hydration_event(
                        message,
                        tool_call_id,
                        tool_output.metadata.as_ref(),
                    );
                }
                message.metadata.insert(
                    "scheduler_stage_status".to_string(),
                    serde_json::json!("running"),
                );
                message.metadata.insert(
                    "scheduler_stage_waiting_on".to_string(),
                    serde_json::json!("model"),
                );
                let event = if tool_name.eq_ignore_ascii_case("question") {
                    if tool_output.is_error {
                        "Question tool failed".to_string()
                    } else {
                        "User answer received".to_string()
                    }
                } else if tool_output.is_error {
                    format!("Tool failed: {}", pretty_scheduler_stage_name(tool_name))
                } else {
                    format!("Tool finished: {}", pretty_scheduler_stage_name(tool_name))
                };
                message.metadata.insert(
                    "scheduler_stage_last_event".to_string(),
                    serde_json::json!(event),
                );
            },
            "prompt.scheduler.stage.tool.end",
        )
        .await;

        // Capture an "after" snapshot when a file-modifying tool completes
        // successfully so that compute_diff() can measure the delta.
        if !tool_output.is_error && is_file_modifying_tool(tool_name) {
            self.track_snapshot("step_finish_snapshot").await;
        }
    }

    async fn on_orchestration_end(&self, _: &str, _: u32, _: &OrchestratorExecutionContext) {
        // Compute cumulative session diffs and persist them.
        // We scan message metadata directly for snapshot hashes that were
        // recorded during step lifecycle events, then call Snapshot::diff_full().
        let session_id = self.session_id.clone();
        let state = self.state.clone();

        let result: Option<()> = async {
            let sessions_guard = state.sessions.lock().await;
            let session = sessions_guard.get(&session_id)?;
            let session_record = session.record();
            let worktree = session_record.directory.clone();

            // Find the earliest step_start_snapshot and latest step_finish_snapshot
            // across all messages in the session.
            let mut from_snapshot: Option<String> = None;
            let mut to_snapshot: Option<String> = None;

            for msg in &session_record.messages {
                if from_snapshot.is_none() {
                    if let Some(s) = msg
                        .metadata
                        .get("step_start_snapshot")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        from_snapshot = Some(s.to_string());
                    }
                }
                if let Some(s) = msg
                    .metadata
                    .get("step_finish_snapshot")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    to_snapshot = Some(s.to_string());
                }
            }

            drop(sessions_guard);

            let (from_hash, to_hash) = match (from_snapshot, to_snapshot) {
                (Some(f), Some(t)) if f != t => (f, t),
                _ => return Some(()), // no snapshots or identical — nothing to diff
            };

            let worktree_path = std::path::PathBuf::from(&worktree);
            let diff_result = tokio::task::spawn_blocking(move || {
                Snapshot::diff_full(&worktree_path, &from_hash, &to_hash)
            })
            .await;

            let file_diffs = match diff_result {
                Ok(Ok(diffs)) => diffs,
                Ok(Err(e)) => {
                    tracing::debug!("Snapshot::diff_full() failed: {e}");
                    return Some(());
                }
                Err(e) => {
                    tracing::debug!("Snapshot::diff_full() task panicked: {e}");
                    return Some(());
                }
            };

            if file_diffs.is_empty() {
                return Some(());
            }

            let summary_diffs: Vec<rocode_session::summary::SummaryFileDiff> = file_diffs
                .iter()
                .map(|d| rocode_session::summary::SummaryFileDiff {
                    file: rocode_session::summary::unquote_git_path(&d.path),
                    additions: d.additions,
                    deletions: d.deletions,
                })
                .collect();

            let summary_data = rocode_session::summary::SessionSummaryData {
                additions: summary_diffs.iter().map(|d| d.additions).sum(),
                deletions: summary_diffs.iter().map(|d| d.deletions).sum(),
                files: summary_diffs.len() as u64,
                diffs: summary_diffs.clone(),
            };

            // Persist summary into session record.
            let mut sessions_guard = state.sessions.lock().await;
            let mut session = sessions_guard.get(&session_id)?.clone();
            rocode_session::summary::set_session_summary(&mut session, &summary_data);
            let _ = rocode_session::summary::persist_session_diffs(
                &mut session,
                &session_id,
                &summary_data.diffs,
            );
            session.touch();
            sessions_guard.update(session);
            drop(sessions_guard);

            state
                .runtime_telemetry
                .diff_updated(
                    &session_id,
                    summary_diffs
                        .iter()
                        .map(|d| DiffEntry {
                            path: d.file.clone(),
                            additions: d.additions,
                            deletions: d.deletions,
                        })
                        .collect(),
                )
                .await;

            Some(())
        }
        .await;

        if result.is_none() {
            tracing::debug!(
                session_id = %self.session_id,
                "Skipped orchestration-end diff summarization (session not found)"
            );
        }
    }

    async fn on_scheduler_stage_start(
        &self,
        _agent_name: &str,
        stage_name: &str,
        stage_index: u32,
        capabilities: Option<&SchedulerStageCapabilities>,
        exec_ctx: &OrchestratorExecutionContext,
    ) {
        let wants_attached_session = capabilities
            .map(|caps| caps.attached_session)
            .unwrap_or(false);

        let mut sessions = self.state.sessions.lock().await;
        let Some(mut session) = sessions.get(&self.session_id).cloned() else {
            return;
        };

        // ── Create attached session if requested ──
        let (attached_session_id, attached_message_id) = if wants_attached_session {
            let mut child = Session::attached_with_context_kind(
                &session,
                SessionContextKind::SchedulerStageOutputSession,
            );
            child.set_title(format!(
                "Stage: {} — {}",
                pretty_scheduler_stage_name(stage_name),
                &self.scheduler_profile
            ));
            let attached_id = child.id.clone();
            let child_msg = child.add_assistant_message();
            let child_msg_id = child_msg.id.clone();
            child_msg.add_text(String::new());
            child.touch();
            sessions.update(child);
            (Some(attached_id), Some(child_msg_id))
        } else {
            (None, None)
        };
        if let (Some(child_sid), Some(child_mid)) =
            (attached_session_id.as_ref(), attached_message_id.as_ref())
        {
            self.state
                .runtime_telemetry
                .attached_session_registered(
                    &self.session_id,
                    child_sid,
                    SessionContextKind::SchedulerStageOutputSession,
                )
                .await;
            self.emit_output_block(
                child_sid.clone(),
                OutputBlock::Message(MessageBlock::start(OutputMessageRole::Assistant)),
                Some(child_mid.clone()),
            )
            .await;
        }
        let message = session.add_assistant_message();
        let message_id = message.id.clone();
        let execution_id = format!("stage_{}", uuid::Uuid::new_v4().simple());
        message.metadata.insert(
            "scheduler_stage_id".to_string(),
            serde_json::json!(&execution_id),
        );
        message.metadata.insert(
            "scheduler_profile".to_string(),
            serde_json::json!(&self.scheduler_profile),
        );
        message
            .metadata
            .insert("scheduler_stage".to_string(), serde_json::json!(stage_name));
        message.metadata.insert(
            "scheduler_stage_index".to_string(),
            serde_json::json!(stage_index),
        );
        message.metadata.insert(
            "scheduler_stage_emitted".to_string(),
            serde_json::json!(true),
        );
        message.metadata.insert(
            "scheduler_stage_agent".to_string(),
            serde_json::json!(&exec_ctx.agent_name),
        );
        message.metadata.insert(
            "scheduler_stage_streaming".to_string(),
            serde_json::json!(true),
        );
        message.metadata.insert(
            "scheduler_stage_status".to_string(),
            serde_json::json!("running"),
        );
        message.metadata.insert(
            "scheduler_stage_focus".to_string(),
            serde_json::json!(scheduler_stage_focus(stage_name)),
        );
        message.metadata.insert(
            "scheduler_stage_last_event".to_string(),
            serde_json::json!("Stage started"),
        );
        message.metadata.insert(
            "scheduler_stage_waiting_on".to_string(),
            serde_json::json!("model"),
        );
        apply_exec_ctx_stage_telemetry_metadata(message, exec_ctx);
        if let Some(observability) =
            scheduler_stage_observability(&self.scheduler_profile, stage_name)
        {
            message.metadata.insert(
                "scheduler_stage_projection".to_string(),
                serde_json::json!(observability.projection),
            );
            message.metadata.insert(
                "scheduler_stage_tool_policy".to_string(),
                serde_json::json!(observability.tool_policy),
            );
            message.metadata.insert(
                "scheduler_stage_loop_budget".to_string(),
                serde_json::json!(observability.loop_budget),
            );
        }
        // Write per-stage capability pool counts into metadata. Concrete
        // runtime usage is tracked separately from tool invocations.
        if let Some(caps) = capabilities {
            message.metadata.insert(
                "scheduler_stage_available_skill_count".to_string(),
                serde_json::json!(caps.skill_list.len()),
            );
            message.metadata.insert(
                "scheduler_stage_available_agent_count".to_string(),
                serde_json::json!(caps.agents.len()),
            );
            message.metadata.insert(
                "scheduler_stage_available_category_count".to_string(),
                serde_json::json!(caps.categories.len()),
            );
        }
        message.metadata.insert(
            "scheduler_stage_active_skills".to_string(),
            serde_json::json!(Vec::<String>::new()),
        );
        message.metadata.insert(
            "scheduler_stage_active_agents".to_string(),
            serde_json::json!(Vec::<String>::new()),
        );
        message.metadata.insert(
            "scheduler_stage_active_categories".to_string(),
            serde_json::json!(Vec::<String>::new()),
        );

        // Store attached-session reference in metadata for persistence/reconstruction.
        if let Some(ref attached_id) = attached_session_id {
            message.metadata.insert(
                "scheduler_stage_attached_session_id".to_string(),
                serde_json::json!(attached_id),
            );
            message.metadata.insert(
                "scheduler_stage_attached_session_kind".to_string(),
                serde_json::to_value(SessionContextKind::SchedulerStageOutputSession)
                    .unwrap_or_else(|_| serde_json::json!("scheduler_stage_output_session")),
            );
        }

        // Start with an empty body; title is rendered from metadata, not persisted text.
        message.add_text(String::new());

        session.touch();
        sessions.update(session);
        drop(sessions);

        if let Some(snapshot) = {
            let sessions = self.state.sessions.lock().await;
            sessions
                .get(&self.session_id)
                .and_then(|session| session.get_message(&message_id).cloned())
        } {
            self.emit_stage_block(&snapshot).await;
        }

        self.state
            .runtime_telemetry
            .register_scheduler_stage(
                &self.session_id,
                execution_id.clone(),
                pretty_scheduler_stage_name(stage_name),
                scheduler_stage_execution_metadata(
                    &self.scheduler_profile,
                    stage_name,
                    stage_index,
                    None,
                    &exec_ctx.agent_name,
                ),
            )
            .await;

        self.active_stage_messages
            .lock()
            .await
            .push(ActiveStageMessage {
                message_id,
                execution_id,
                stage_name: stage_name.to_string(),
                step_count: 0,
                committed_usage: rocode_orchestrator::runtime::events::StepUsage::default(),
                live_usage: rocode_orchestrator::runtime::events::StepUsage::default(),
                attached_session_id,
                attached_message_id,
                attached_reasoning_started: false,
                reasoning_started: false,
            });
    }

    async fn on_scheduler_stage_content(
        &self,
        stage_name: &str,
        _stage_index: u32,
        content_delta: &str,
        _exec_ctx: &OrchestratorExecutionContext,
    ) {
        let (message_id, attached_session_id, attached_message_id) = {
            let guard = self.active_stage_messages.lock().await;
            match guard.last() {
                Some(active) => (
                    active.message_id.clone(),
                    active.attached_session_id.clone(),
                    active.attached_message_id.clone(),
                ),
                None => return,
            }
        };

        // If an attached session exists, route content there instead of the parent stage message.
        if let (Some(child_sid), Some(child_mid)) = (attached_session_id, attached_message_id) {
            let mut sessions = self.state.sessions.lock().await;
            if let Some(mut child) = sessions.get(&child_sid).cloned() {
                if let Some(msg) = child.get_message_mut(&child_mid) {
                    msg.append_text(content_delta);
                }
                child.touch();
                sessions.update(child);
            }
            drop(sessions);
            self.emit_output_block(
                child_sid,
                OutputBlock::Message(MessageBlock::delta(
                    OutputMessageRole::Assistant,
                    content_delta.to_string(),
                )),
                Some(child_mid),
            )
            .await;
            return;
        }

        let mut sessions = self.state.sessions.lock().await;
        let Some(mut session) = sessions.get(&self.session_id).cloned() else {
            return;
        };

        let mut message_snapshot = None;
        if let Some(message) = session.get_message_mut(&message_id) {
            message.append_text(content_delta);
            apply_scheduler_decision_metadata(stage_name, message);
            message_snapshot = Some(message.clone());
        }
        session.touch();
        sessions.update(session);
        drop(sessions);

        if let Some(message) = message_snapshot.as_ref() {
            // The stage message already carries scheduler metadata locally.
            // Stream only the text delta here so we do not re-broadcast the
            // entire accumulated stage card on every content token.
            self.emit_output_block(
                self.session_id.clone(),
                OutputBlock::Message(MessageBlock::delta(
                    OutputMessageRole::Assistant,
                    content_delta.to_string(),
                )),
                Some(message.id.clone()),
            )
            .await;
        }
    }

    async fn on_scheduler_stage_reasoning(
        &self,
        stage_name: &str,
        _stage_index: u32,
        reasoning_delta: &str,
        _exec_ctx: &OrchestratorExecutionContext,
    ) {
        tracing::debug!(
            session_id = %self.session_id,
            stage_name = %stage_name,
            reasoning_len = reasoning_delta.len(),
            "on_scheduler_stage_reasoning called"
        );

        let (
            message_id,
            attached_session_id,
            attached_message_id,
            start_attached_reasoning,
            start_reasoning,
        ) = {
            let mut guard = self.active_stage_messages.lock().await;
            match guard.last_mut() {
                Some(active) => {
                    let start_attached_reasoning = active.attached_session_id.is_some()
                        && active.attached_message_id.is_some()
                        && !active.attached_reasoning_started;
                    if start_attached_reasoning {
                        active.attached_reasoning_started = true;
                    }
                    // For main session (non-attached), track reasoning started
                    let start_reasoning =
                        active.attached_session_id.is_none() && !active.reasoning_started;
                    if start_reasoning {
                        active.reasoning_started = true;
                    }
                    (
                        Some(active.message_id.clone()),
                        active.attached_session_id.clone(),
                        active.attached_message_id.clone(),
                        start_attached_reasoning,
                        start_reasoning,
                    )
                }
                None => {
                    // Non-scheduler-stage mode: find current assistant message
                    drop(guard);
                    let sessions = self.state.sessions.lock().await;
                    if let Some(session) = sessions.get(&self.session_id) {
                        if let Some(last_assistant) = session
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.role == MessageRole::Assistant)
                        {
                            (Some(last_assistant.id.clone()), None, None, false, false)
                        } else {
                            (None, None, None, false, false)
                        }
                    } else {
                        (None, None, None, false, false)
                    }
                }
            }
        };

        let Some(message_id) = message_id else {
            return;
        };

        // If an attached session exists, route reasoning there.
        if let (Some(child_sid), Some(child_mid)) = (attached_session_id, attached_message_id) {
            let mut sessions = self.state.sessions.lock().await;
            if let Some(mut child) = sessions.get(&child_sid).cloned() {
                if let Some(msg) = child.get_message_mut(&child_mid) {
                    msg.add_reasoning(reasoning_delta);
                }
                child.touch();
                sessions.update(child);
            }
            drop(sessions);
            if start_attached_reasoning {
                self.emit_output_block(
                    child_sid.clone(),
                    OutputBlock::Reasoning(ReasoningBlock::start()),
                    Some(child_mid.clone()),
                )
                .await;
            }
            self.emit_output_block(
                child_sid.clone(),
                OutputBlock::Reasoning(ReasoningBlock::delta(reasoning_delta.to_string())),
                Some(child_mid),
            )
            .await;
            return;
        }

        // Non-attached session: emit reasoning events for TUI/CLI to display
        if start_reasoning {
            self.emit_output_block(
                self.session_id.clone(),
                OutputBlock::Reasoning(ReasoningBlock::start()),
                Some(message_id.clone()),
            )
            .await;
        }
        self.emit_output_block(
            self.session_id.clone(),
            OutputBlock::Reasoning(ReasoningBlock::delta(reasoning_delta.to_string())),
            Some(message_id.clone()),
        )
        .await;

        let mut sessions = self.state.sessions.lock().await;
        let Some(mut session) = sessions.get(&self.session_id).cloned() else {
            return;
        };

        if let Some(message) = session.get_message_mut(&message_id) {
            message.add_reasoning(reasoning_delta);
            apply_scheduler_decision_metadata(stage_name, message);
        }
        session.touch();
        sessions.update(session);
        drop(sessions);
    }

    async fn on_scheduler_stage_usage(
        &self,
        _stage_name: &str,
        _stage_index: u32,
        usage: &rocode_orchestrator::runtime::events::StepUsage,
        finalized: bool,
        _exec_ctx: &OrchestratorExecutionContext,
    ) {
        let model_pricing = self.model_pricing;
        self.update_active_stage_message(
            |message, active| {
                active.live_usage.merge_snapshot(usage);
                if finalized {
                    let live_usage = active.live_usage.clone();
                    active.committed_usage.accumulate(&live_usage);
                    active.live_usage = rocode_orchestrator::runtime::events::StepUsage::default();
                }
                write_stage_usage_totals(
                    message,
                    &active.committed_usage,
                    &active.live_usage,
                    finalized,
                    model_pricing,
                );
            },
            "prompt.scheduler.stage.usage",
        )
        .await;
    }

    async fn on_step_checkpoint(
        &self,
        _agent_name: &str,
        _model_id: &str,
        _step: u32,
        stage_name: Option<&str>,
        _stage_index: Option<u32>,
        usage: &rocode_orchestrator::runtime::events::StepUsage,
        _request_view: &[rocode_provider::Message],
        checkpoint: &rocode_orchestrator::runtime::events::StepCheckpointSnapshot,
        default_directive: &StepCheckpointDirective,
        _exec_ctx: &OrchestratorExecutionContext,
    ) -> Result<Option<StepCheckpointDirective>, rocode_orchestrator::OrchestratorError> {
        let request_context_tokens = preferred_checkpoint_request_context_tokens(checkpoint, usage);
        let request_body_chars = checkpoint.current_view.estimated_body_chars;
        let compaction_attempted = checkpoint.compaction_attempted();
        let compaction_succeeded = checkpoint.compaction_succeeded();
        let Some(summary) = self
            .govern_step_checkpoint(
                request_context_tokens,
                request_body_chars,
                compaction_attempted,
                compaction_succeeded,
            )
            .await?
        else {
            return Ok(None);
        };

        let preferred_directive =
            directive_from_context_pressure_summary(&summary, stage_name, checkpoint);

        if matches!(
            preferred_directive,
            StepCheckpointDirective::CompactRequestView { .. }
        ) {
            self.state
                .runtime_telemetry
                .set_session_run_status(&self.session_id, SessionRunStatus::Compacting)
                .await;
            self.update_active_stage_message(
                |message, _active| {
                    message.metadata.insert(
                        "scheduler_stage_last_event".to_string(),
                        serde_json::json!(
                            "Context pressure checkpoint requested request-view compaction"
                        ),
                    );
                },
                "prompt.scheduler.stage.step_checkpoint.compact",
            )
            .await;
            return if same_step_checkpoint_directive(&preferred_directive, default_directive) {
                Ok(None)
            } else {
                Ok(Some(preferred_directive))
            };
        }

        if matches!(preferred_directive, StepCheckpointDirective::Continue) {
            if compaction_attempted && compaction_succeeded {
                self.state
                    .runtime_telemetry
                    .set_session_run_status(&self.session_id, SessionRunStatus::Busy)
                    .await;
                self.update_active_stage_message(
                    |message, _active| {
                        message.metadata.insert(
                            "scheduler_stage_last_event".to_string(),
                            serde_json::json!("Context pressure checkpoint compacted request view"),
                        );
                    },
                    "prompt.scheduler.stage.step_checkpoint.compacted",
                )
                .await;
            }
            return if same_step_checkpoint_directive(&preferred_directive, default_directive) {
                Ok(None)
            } else {
                Ok(Some(preferred_directive))
            };
        }

        if compaction_attempted {
            self.state
                .runtime_telemetry
                .set_session_run_status(&self.session_id, SessionRunStatus::Busy)
                .await;
        }
        self.update_active_stage_message(
            |message, _active| {
                message.metadata.insert(
                    "scheduler_stage_status".to_string(),
                    serde_json::json!("blocked"),
                );
                message.metadata.insert(
                    "scheduler_stage_waiting_on".to_string(),
                    serde_json::json!("none"),
                );
                message.metadata.insert(
                    "scheduler_stage_last_event".to_string(),
                    serde_json::json!("Context pressure gate blocked next step"),
                );
            },
            "prompt.scheduler.stage.step_checkpoint.blocked",
        )
        .await;

        if same_step_checkpoint_directive(&preferred_directive, default_directive) {
            Ok(None)
        } else {
            Ok(Some(preferred_directive))
        }
    }

    async fn on_scheduler_stage_end(
        &self,
        _: &str,
        stage_name: &str,
        stage_index: u32,
        stage_total: u32,
        content: &str,
        exec_ctx: &OrchestratorExecutionContext,
    ) {
        let active = {
            let mut guard = self.active_stage_messages.lock().await;
            guard.pop()
        };

        match active {
            Some(active) if active.stage_name == stage_name => {
                // Finalize the streaming message: replace content with final version.
                let body = content.trim();
                let mut sessions = self.state.sessions.lock().await;
                let Some(mut session) = sessions.get(&self.session_id).cloned() else {
                    return;
                };
                let mut message_snapshot = None;
                if let Some(message) = session.get_message_mut(&active.message_id) {
                    message.set_text(body.to_string());
                    message.metadata.insert(
                        "scheduler_stage_total".to_string(),
                        serde_json::json!(stage_total),
                    );
                    message.metadata.remove("scheduler_stage_streaming");
                    message.metadata.insert(
                        "scheduler_stage_status".to_string(),
                        serde_json::json!(if body.starts_with("Stage error:") {
                            "blocked"
                        } else {
                            "done"
                        }),
                    );
                    message.metadata.insert(
                        "scheduler_stage_focus".to_string(),
                        serde_json::json!(scheduler_stage_focus(stage_name)),
                    );
                    message.metadata.insert(
                        "scheduler_stage_last_event".to_string(),
                        serde_json::json!(if body.starts_with("Stage error:") {
                            "Stage failed"
                        } else {
                            "Stage completed"
                        }),
                    );
                    message.metadata.insert(
                        "scheduler_stage_waiting_on".to_string(),
                        serde_json::json!("none"),
                    );
                    if active.step_count > 0 {
                        message.metadata.insert(
                            "scheduler_stage_step".to_string(),
                            serde_json::json!(active.step_count),
                        );
                    }
                    apply_scheduler_decision_metadata(stage_name, message);
                    message_snapshot = Some(message.clone());
                }

                let attached_session_id = active.attached_session_id.clone();
                let attached_message_id = active.attached_message_id.clone();

                // Finalize attached-session assistant message if present.
                if let (Some(ref child_sid), Some(ref child_mid)) =
                    (attached_session_id.as_ref(), attached_message_id.as_ref())
                {
                    if let Some(mut child) = sessions.get(child_sid).cloned() {
                        if let Some(msg) = child.get_message_mut(child_mid) {
                            msg.finish = Some("end_turn".to_string());
                        }
                        child.touch();
                        sessions.update(child);
                    }
                }

                session.touch();
                sessions.update(session);
                drop(sessions);

                if let Some(message) = message_snapshot.as_ref() {
                    self.emit_stage_block(message).await;
                }
                if let (Some(child_sid), Some(child_mid)) =
                    (attached_session_id, attached_message_id)
                {
                    if active.attached_reasoning_started {
                        self.emit_output_block(
                            child_sid.clone(),
                            OutputBlock::Reasoning(ReasoningBlock::end()),
                            Some(child_mid.clone()),
                        )
                        .await;
                    }
                    self.emit_output_block(
                        child_sid.clone(),
                        OutputBlock::Message(MessageBlock::end(OutputMessageRole::Assistant)),
                        Some(child_mid.clone()),
                    )
                    .await;
                    self.state
                        .runtime_telemetry
                        .attached_session_unregistered(&self.session_id, &child_sid)
                        .await;
                } else {
                    // Non-attached session: emit reasoning end if reasoning was started
                    if active.reasoning_started {
                        self.emit_output_block(
                            self.session_id.clone(),
                            OutputBlock::Reasoning(ReasoningBlock::end()),
                            Some(active.message_id.clone()),
                        )
                        .await;
                    }
                }
                self.state
                    .runtime_telemetry
                    .finish_scheduler_stage(&active.execution_id)
                    .await;
            }
            Some(_) => {
                self.emit_stage_message(stage_name, stage_index, stage_total, content, exec_ctx)
                    .await;
            }
            None => {
                // Fallback: no streaming message was created, emit full message.
                self.emit_stage_message(stage_name, stage_index, stage_total, content, exec_ctx)
                    .await;
            }
        }
    }
}

fn stage_execution_patch_from_message(message: &SessionMessage) -> ExecutionPatch {
    ExecutionPatch {
        status: message
            .metadata
            .get("scheduler_stage_status")
            .and_then(|value| value.as_str())
            .and_then(runtime_execution_status_from_stage_status),
        waiting_on: message
            .metadata
            .get("scheduler_stage_waiting_on")
            .and_then(|value| value.as_str())
            .filter(|value| *value != "none" && !value.is_empty())
            .map(|value| FieldUpdate::Set(value.to_string()))
            .unwrap_or(FieldUpdate::Clear),
        recent_event: message
            .metadata
            .get("scheduler_stage_last_event")
            .and_then(|value| value.as_str())
            .map(|value| FieldUpdate::Set(value.to_string()))
            .unwrap_or(FieldUpdate::Keep),
        metadata: FieldUpdate::Set(scheduler_stage_runtime_metadata(message)),
        ..ExecutionPatch::default()
    }
}

fn runtime_execution_status_from_stage_status(value: &str) -> Option<ExecutionStatus> {
    match value {
        "running" => Some(ExecutionStatus::Running),
        "waiting" => Some(ExecutionStatus::Waiting),
        "cancelling" => Some(ExecutionStatus::Cancelling),
        "retry" => Some(ExecutionStatus::Retry),
        _ => None,
    }
}

fn persist_scheduler_context_hydration_event(
    message: &mut SessionMessage,
    tool_call_id: &str,
    metadata: Option<&serde_json::Value>,
) {
    let Some(metadata) = metadata else {
        return;
    };
    let hydrated_ids = metadata_array_strings(metadata, "hydrated_message_ids");
    let rejected_ids = metadata_array_strings(metadata, "rejected_message_ids");
    let missing_ids = metadata_array_strings(metadata, "missing_message_ids");
    if hydrated_ids.is_empty() && rejected_ids.is_empty() && missing_ids.is_empty() {
        return;
    }

    merge_unique_metadata_strings(
        &mut message.metadata,
        SCHEDULER_STAGE_CONTEXT_HYDRATION_IDS_METADATA_KEY,
        &hydrated_ids,
    );

    let mut events = message
        .metadata
        .get(SCHEDULER_STAGE_CONTEXT_HYDRATION_EVENTS_METADATA_KEY)
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    events.push(serde_json::json!({
        "tool_call_id": tool_call_id,
        "hydrated_message_ids": hydrated_ids,
        "rejected_message_ids": rejected_ids,
        "missing_message_ids": missing_ids,
        "hydrated_count": metadata.get("hydrated_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "rejected_count": metadata.get("rejected_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "missing_count": metadata.get("missing_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "max_chars_per_message": metadata.get("max_chars_per_message").and_then(|value| value.as_u64()),
    }));
    message.metadata.insert(
        SCHEDULER_STAGE_CONTEXT_HYDRATION_EVENTS_METADATA_KEY.to_string(),
        serde_json::Value::Array(events),
    );
}

fn persist_scheduler_memory_hydration_event(
    message: &mut SessionMessage,
    tool_call_id: &str,
    metadata: Option<&serde_json::Value>,
) {
    let Some(metadata) = metadata else {
        return;
    };
    let hydrated_ids = metadata_array_strings(metadata, "hydrated_memory_record_ids");
    let rejected_ids = metadata_array_strings(metadata, "rejected_memory_record_ids");
    let missing_ids = metadata_array_strings(metadata, "missing_memory_record_ids");
    if hydrated_ids.is_empty() && rejected_ids.is_empty() && missing_ids.is_empty() {
        return;
    }

    merge_unique_metadata_strings(
        &mut message.metadata,
        SCHEDULER_STAGE_MEMORY_HYDRATION_IDS_METADATA_KEY,
        &hydrated_ids,
    );

    let mut events = message
        .metadata
        .get(SCHEDULER_STAGE_MEMORY_HYDRATION_EVENTS_METADATA_KEY)
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    events.push(serde_json::json!({
        "tool_call_id": tool_call_id,
        "hydrated_memory_record_ids": hydrated_ids,
        "rejected_memory_record_ids": rejected_ids,
        "missing_memory_record_ids": missing_ids,
        "hydrated_count": metadata.get("hydrated_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "rejected_count": metadata.get("rejected_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "missing_count": metadata.get("missing_count").and_then(|value| value.as_u64()).unwrap_or(0),
        "max_chars_per_record": metadata.get("max_chars_per_record").and_then(|value| value.as_u64()),
        "include_evidence": metadata.get("include_evidence").and_then(|value| value.as_bool()).unwrap_or(false),
    }));
    message.metadata.insert(
        SCHEDULER_STAGE_MEMORY_HYDRATION_EVENTS_METADATA_KEY.to_string(),
        serde_json::Value::Array(events),
    );
}

fn metadata_array_strings(metadata: &serde_json::Value, key: &str) -> Vec<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn merge_unique_metadata_strings(
    metadata: &mut std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    values: &[String],
) {
    let mut merged = metadata
        .get(key)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for value in values {
        if !merged.iter().any(|existing| existing == value) {
            merged.push(value.clone());
        }
    }
    metadata.insert(key.to_string(), serde_json::json!(merged));
}

fn apply_exec_ctx_stage_telemetry_metadata(
    message: &mut SessionMessage,
    exec_ctx: &OrchestratorExecutionContext,
) {
    for key in [
        "scheduler_stage_estimated_context_tokens",
        "scheduler_stage_skill_tree_budget",
        "scheduler_stage_skill_tree_truncation_strategy",
        "scheduler_stage_skill_tree_truncated",
        "scheduler_stage_retry_attempt",
    ] {
        if let Some(value) = exec_ctx.metadata.get(key).cloned() {
            message.metadata.insert(key.to_string(), value);
        }
    }
}

fn scheduler_stage_runtime_metadata(message: &SessionMessage) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    for key in [
        "scheduler_profile",
        "scheduler_stage",
        "scheduler_stage_index",
        "scheduler_stage_total",
        "scheduler_stage_agent",
        "scheduler_stage_step",
        "scheduler_stage_focus",
        "scheduler_stage_projection",
        "scheduler_stage_tool_policy",
        "scheduler_stage_loop_budget",
        "scheduler_stage_estimated_context_tokens",
        "scheduler_stage_skill_tree_budget",
        "scheduler_stage_skill_tree_truncation_strategy",
        "scheduler_stage_skill_tree_truncated",
        "scheduler_stage_retry_attempt",
        "scheduler_stage_activity",
        "scheduler_stage_available_skill_count",
        "scheduler_stage_available_agent_count",
        "scheduler_stage_available_category_count",
        "scheduler_stage_active_skills",
        "scheduler_stage_active_agents",
        "scheduler_stage_active_categories",
        SCHEDULER_STAGE_CONTEXT_HYDRATION_IDS_METADATA_KEY,
        SCHEDULER_STAGE_CONTEXT_HYDRATION_EVENTS_METADATA_KEY,
        SCHEDULER_STAGE_MEMORY_HYDRATION_IDS_METADATA_KEY,
        SCHEDULER_STAGE_MEMORY_HYDRATION_EVENTS_METADATA_KEY,
        "scheduler_stage_prompt_tokens",
        "scheduler_stage_completion_tokens",
        "scheduler_stage_reasoning_tokens",
        "scheduler_stage_cache_read_tokens",
        "scheduler_stage_cache_miss_tokens",
        "scheduler_stage_cache_write_tokens",
    ] {
        if let Some(value) = message.metadata.get(key).cloned() {
            metadata.insert(key.to_string(), value);
        }
    }
    serde_json::Value::Object(metadata)
}

fn scheduler_stage_execution_metadata(
    scheduler_profile: &str,
    stage_name: &str,
    stage_index: u32,
    stage_total: Option<u32>,
    agent_name: &str,
) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "scheduler_profile".to_string(),
        serde_json::json!(scheduler_profile),
    );
    metadata.insert("scheduler_stage".to_string(), serde_json::json!(stage_name));
    metadata.insert(
        "scheduler_stage_index".to_string(),
        serde_json::json!(stage_index),
    );
    if let Some(stage_total) = stage_total {
        metadata.insert(
            "scheduler_stage_total".to_string(),
            serde_json::json!(stage_total),
        );
    }
    metadata.insert(
        "scheduler_stage_agent".to_string(),
        serde_json::json!(agent_name),
    );
    metadata.insert(
        "scheduler_stage_focus".to_string(),
        serde_json::json!(scheduler_stage_focus(stage_name)),
    );
    serde_json::Value::Object(metadata)
}

fn summarize_tool_activity(tool_name: &str, tool_args: &serde_json::Value) -> Option<String> {
    match tool_name.to_ascii_lowercase().as_str() {
        "question" => summarize_question_args(tool_args),
        "todowrite" | "todo_write" => summarize_todo_args(tool_args),
        "todoread" | "todo_read" => Some("Todo list read".to_string()),
        "task" => summarize_task_args(tool_args),
        "task_flow" => summarize_task_flow_args(tool_args),
        "bash" | "shell" => summarize_bash_args(tool_args),
        "read" | "readfile" | "read_file" => summarize_read_args(tool_args),
        "write" | "writefile" | "write_file" => summarize_write_args(tool_args),
        "edit" | "editfile" | "edit_file" => summarize_edit_args(tool_args),
        "glob" => summarize_glob_args(tool_args),
        "grep" => summarize_grep_args(tool_args),
        "webfetch" | "web_fetch" => summarize_webfetch_args(tool_args),
        "websearch" | "web_search" | "codesearch" | "code_search" => {
            summarize_search_args(tool_name, tool_args)
        }
        "lsp" => summarize_lsp_args(tool_args),
        "batch" => summarize_batch_args(tool_args),
        "skill" | "skill_view" => summarize_skill_args(tool_args),
        "skills_list" => summarize_skills_list_args(tool_args),
        "apply_patch" | "applypatch" => Some("Apply Patch".to_string()),
        "list" | "ls" | "listdir" | "list_dir" | "list_directory" => summarize_list_args(tool_args),
        "notebook_edit" | "notebookedit" => summarize_notebook_edit_args(tool_args),
        _ => summarize_generic_tool_args(tool_name, tool_args),
    }
}

fn summarize_tool_result_activity(
    tool_name: &str,
    tool_output: &OrchestratorToolOutput,
) -> Option<String> {
    match tool_name.to_ascii_lowercase().as_str() {
        "question" => summarize_question_result(tool_output.metadata.as_ref()),
        "todowrite" | "todo_write" | "todoread" | "todo_read" => {
            summarize_todo_result(tool_output.metadata.as_ref())
        }
        _ => None,
    }
}

fn summarize_question_args(tool_args: &serde_json::Value) -> Option<String> {
    let questions = tool_args.get("questions")?.as_array()?;
    if questions.is_empty() {
        return None;
    }
    let mut lines = vec![format!("Question ({})", questions.len())];
    for question in questions.iter().take(3) {
        let header = question
            .get("header")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Prompt");
        let text = question
            .get("question")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !text.is_empty() {
            lines.push(format!("- {header}: {}", collapse_text(text, 96)));
        }
    }
    Some(lines.join("\n"))
}

fn summarize_todo_args(tool_args: &serde_json::Value) -> Option<String> {
    let todos = tool_args.get("todos")?.as_array()?;
    if todos.is_empty() {
        return None;
    }
    let mut lines = vec![format!("Todo list ({})", todos.len())];
    for todo in todos.iter().take(5) {
        let content = todo.get("content").and_then(|value| value.as_str())?;
        let status = todo
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("pending");
        lines.push(format!(
            "- [{}] {}",
            prettify_token(status),
            collapse_text(content, 88)
        ));
    }
    Some(lines.join("\n"))
}

fn summarize_task_args(tool_args: &serde_json::Value) -> Option<String> {
    let agent = tool_args
        .get("subagent_type")
        .or_else(|| tool_args.get("subagentType"))
        .or_else(|| tool_args.get("category"))
        .and_then(|value| value.as_str())
        .unwrap_or("subagent");
    let description = tool_args
        .get("description")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let prompt = tool_args
        .get("prompt")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mut lines = vec![format!("Task → {}", prettify_token(agent))];
    if !description.is_empty() {
        lines.push(format!("- label: {}", collapse_text(description, 88)));
    }
    if !prompt.is_empty() {
        lines.push(format!("- prompt: {}", collapse_text(prompt, 88)));
    }
    Some(lines.join("\n"))
}

fn summarize_task_flow_args(tool_args: &serde_json::Value) -> Option<String> {
    let operation = tool_args
        .get("operation")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let mut lines = vec![format!("TaskFlow → {}", prettify_token(operation))];
    if let Some(agent) = tool_args.get("agent").and_then(|value| value.as_str()) {
        lines.push(format!("- agent: {}", prettify_token(agent)));
    }
    if let Some(description) = tool_args
        .get("description")
        .and_then(|value| value.as_str())
    {
        lines.push(format!("- label: {}", collapse_text(description, 88)));
    }
    if let Some(prompt) = tool_args.get("prompt").and_then(|value| value.as_str()) {
        lines.push(format!("- prompt: {}", collapse_text(prompt, 88)));
    }
    let todo_items = task_flow_todo_labels(tool_args);
    if !todo_items.is_empty() {
        for (index, content) in todo_items.iter().take(3).enumerate() {
            let label = if todo_items.len() == 1 {
                "todo".to_string()
            } else {
                format!("todo {}", index + 1)
            };
            lines.push(format!("- {}: {}", label, collapse_text(content, 88)));
        }
        if todo_items.len() > 3 {
            lines.push(format!("- more todos: {}", todo_items.len() - 3));
        }
    }
    Some(lines.join("\n"))
}

fn task_flow_todo_labels(tool_args: &serde_json::Value) -> Vec<String> {
    let Some(todo_value) = tool_args
        .get("todo_items")
        .or_else(|| tool_args.get("todos"))
        .or_else(|| tool_args.get("todo_item"))
    else {
        return Vec::new();
    };

    match todo_value {
        serde_json::Value::Object(todo) => todo
            .get("content")
            .and_then(|value| value.as_str())
            .map(|value| vec![value.to_string()])
            .unwrap_or_default(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("content")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string)
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Runtime evidence for which scheduler capabilities were actually activated
/// inside the current stage.
///
/// Governance rule:
/// - `SchedulerStageCapabilities` is the stage's available resource pool.
/// - `scheduler_stage_active_*` is runtime evidence only.
/// - Adapters render these fields but never infer them.
/// - Evidence may arrive from request-time tool arguments or result-time tool
///   metadata, so both sides feed the same authority here.
#[derive(Default)]
struct StageCapabilityActivityEvidence {
    agents: Vec<String>,
    categories: Vec<String>,
    skills: Vec<String>,
}

impl StageCapabilityActivityEvidence {
    fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.categories.is_empty() && self.skills.is_empty()
    }

    fn push_agent(&mut self, value: Option<&str>) {
        push_unique_trimmed(&mut self.agents, value);
    }

    fn push_category(&mut self, value: Option<&str>) {
        push_unique_trimmed(&mut self.categories, value);
    }

    fn push_skills_from_array(&mut self, value: Option<&serde_json::Value>) {
        let Some(values) = value.and_then(|value| value.as_array()) else {
            return;
        };
        for skill in values.iter().filter_map(|value| value.as_str()) {
            push_unique_trimmed(&mut self.skills, Some(skill));
        }
    }
}

enum StageCapabilityActivitySource<'a> {
    ToolArgs(&'a serde_json::Value),
    ToolOutput(&'a OrchestratorToolOutput),
}

/// Extract the single authority view of runtime capability activation for a
/// scheduler stage.
///
/// This intentionally tracks only concrete scheduling choices:
/// - selected agent
/// - selected category
/// - explicitly loaded skills
///
/// It does not treat generic tool usage, questions, summaries, or stage
/// capability pools as "active" capability evidence.
fn extract_stage_capability_activity(
    tool_name: &str,
    source: StageCapabilityActivitySource<'_>,
) -> StageCapabilityActivityEvidence {
    let mut evidence = StageCapabilityActivityEvidence::default();

    match source {
        StageCapabilityActivitySource::ToolArgs(args) => {
            if !tool_supports_stage_capability_activity_args(tool_name) {
                return evidence;
            }

            evidence.push_agent(
                args.get("subagent_type")
                    .or_else(|| args.get("subagentType"))
                    .or_else(|| args.get("agent"))
                    .and_then(|value| value.as_str()),
            );
            evidence.push_category(args.get("category").and_then(|value| value.as_str()));
            evidence.push_skills_from_array(
                args.get("load_skills").or_else(|| args.get("loadedSkills")),
            );
        }
        StageCapabilityActivitySource::ToolOutput(tool_output) => {
            let Some(metadata) = tool_output.metadata.as_ref() else {
                return evidence;
            };
            if !tool_supports_stage_capability_activity_output(tool_name, metadata) {
                return evidence;
            }

            evidence.push_agent(
                metadata
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        metadata
                            .get("task")
                            .and_then(|value| value.get("agent"))
                            .and_then(|value| value.as_str())
                    }),
            );
            evidence.push_category(metadata.get("category").and_then(|value| value.as_str()));
            evidence.push_skills_from_array(
                metadata
                    .get("loadedSkills")
                    .or_else(|| metadata.get("load_skills"))
                    .or_else(|| {
                        metadata
                            .get("task")
                            .and_then(|value| value.get("loadedSkills"))
                    }),
            );
        }
    }

    evidence
}

fn tool_supports_stage_capability_activity_args(tool_name: &str) -> bool {
    matches!(tool_name, "task" | "task_flow")
}

fn tool_supports_stage_capability_activity_output(
    tool_name: &str,
    metadata: &serde_json::Value,
) -> bool {
    matches!(tool_name, "task" | "task_flow")
        || metadata
            .get("delegated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || metadata.get("agentTaskId").is_some()
        || metadata.get("task").is_some()
}

fn apply_stage_capability_activity_evidence(
    message: &mut SessionMessage,
    evidence: StageCapabilityActivityEvidence,
) {
    if evidence.is_empty() {
        return;
    }

    for agent in evidence.agents {
        push_stage_active_value(message, "scheduler_stage_active_agents", &agent);
    }
    for category in evidence.categories {
        push_stage_active_value(message, "scheduler_stage_active_categories", &category);
    }
    for skill in evidence.skills {
        push_stage_active_value(message, "scheduler_stage_active_skills", &skill);
    }
}

fn push_stage_active_value(message: &mut SessionMessage, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    let entry = message
        .metadata
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!([]));

    if !entry.is_array() {
        *entry = serde_json::json!([]);
    }

    let Some(values) = entry.as_array_mut() else {
        return;
    };

    if values
        .iter()
        .any(|existing| existing.as_str() == Some(value))
    {
        return;
    }

    values.push(serde_json::json!(value));
}

fn push_unique_trimmed(target: &mut Vec<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if target.iter().any(|existing| existing == value) {
        return;
    }
    target.push(value.to_string());
}

fn summarize_generic_tool_args(tool_name: &str, tool_args: &serde_json::Value) -> Option<String> {
    if tool_args.is_null() {
        return None;
    }
    let name = pretty_scheduler_stage_name(tool_name);
    // Extract key=value pairs from primitive fields, omitting large text blobs.
    if let Some(object) = tool_args.as_object() {
        let preview = format_activity_primitive_args(object);
        if let Some(preview) = preview {
            return Some(format!("{name} → {preview}"));
        }
    }
    // Fallback: collapse raw JSON
    let raw = collapse_text(&tool_args.to_string(), 120);
    Some(format!("{name} → {raw}"))
}

// ── Tool-specific activity summarizers ──────────────────────────────────

fn summarize_bash_args(tool_args: &serde_json::Value) -> Option<String> {
    let command = activity_extract_string(tool_args, &["command", "cmd", "script", "input"])?;
    Some(format!("Bash → $ {}", collapse_text(&command, 120)))
}

fn summarize_read_args(tool_args: &serde_json::Value) -> Option<String> {
    let path = activity_extract_string(tool_args, &["file_path", "filePath", "path", "file"])?;
    Some(format!("Read → {path}"))
}

fn summarize_write_args(tool_args: &serde_json::Value) -> Option<String> {
    let path = activity_extract_string(tool_args, &["file_path", "filePath", "path", "file"])?;
    Some(format!("Write ← {path}"))
}

fn summarize_edit_args(tool_args: &serde_json::Value) -> Option<String> {
    let path = activity_extract_string(tool_args, &["file_path", "filePath", "path", "file"])?;
    Some(format!("Edit ← {path}"))
}

fn summarize_glob_args(tool_args: &serde_json::Value) -> Option<String> {
    let pattern = activity_extract_string(tool_args, &["pattern"])?;
    let target = activity_extract_string(tool_args, &["path", "file_path", "filePath"]);
    let summary = match target {
        Some(path) => format!("Glob → \"{}\" in {}", pattern, path),
        None => format!("Glob → \"{}\"", pattern),
    };
    Some(summary)
}

fn summarize_grep_args(tool_args: &serde_json::Value) -> Option<String> {
    let pattern = activity_extract_string(tool_args, &["pattern", "query"])?;
    let target = activity_extract_string(tool_args, &["path", "file_path", "filePath"]);
    let summary = match target {
        Some(path) => format!("Grep → \"{}\" in {}", pattern, path),
        None => format!("Grep → \"{}\"", pattern),
    };
    Some(summary)
}

fn summarize_webfetch_args(tool_args: &serde_json::Value) -> Option<String> {
    let url = activity_extract_string(tool_args, &["url"])?;
    Some(format!("Web Fetch → {url}"))
}

fn summarize_search_args(tool_name: &str, tool_args: &serde_json::Value) -> Option<String> {
    let query = activity_extract_string(tool_args, &["query"])?;
    let name = pretty_scheduler_stage_name(tool_name);
    Some(format!("{name} → \"{query}\""))
}

fn summarize_lsp_args(tool_args: &serde_json::Value) -> Option<String> {
    let operation = activity_extract_string(tool_args, &["operation"])?;
    let target = activity_extract_string(tool_args, &["filePath", "file_path", "path"]);
    let summary = match target {
        Some(path) => format!("LSP → {} {}", operation, path),
        None => format!("LSP → {}", operation),
    };
    Some(summary)
}

fn summarize_batch_args(tool_args: &serde_json::Value) -> Option<String> {
    let calls = tool_args
        .get("toolCalls")
        .or_else(|| tool_args.get("tool_calls"))
        .and_then(|v| v.as_array())?;
    let count = calls.len();
    let mut names: Vec<String> = calls
        .iter()
        .filter_map(|call| {
            call.get("tool")
                .or_else(|| call.get("name"))
                .or_else(|| call.get("tool_name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    names.dedup();
    if names.is_empty() {
        Some(format!("Batch → {} tools", count))
    } else {
        Some(format!("Batch → {} tools ({})", count, names.join(", ")))
    }
}

fn summarize_skill_args(tool_args: &serde_json::Value) -> Option<String> {
    let name = activity_extract_string(tool_args, &["name", "skill"])?;
    Some(format!("Skill → \"{}\"", name))
}

fn summarize_skills_list_args(tool_args: &serde_json::Value) -> Option<String> {
    let category = activity_extract_string(tool_args, &["category"]);
    Some(match category {
        Some(category) => format!("Skills List → [{}]", category),
        None => "Skills List".to_string(),
    })
}

fn summarize_list_args(tool_args: &serde_json::Value) -> Option<String> {
    let path = activity_extract_string(tool_args, &["path", "file_path", "filePath"]);
    match path {
        Some(path) => Some(format!("List → {path}")),
        None => Some("List → .".to_string()),
    }
}

fn summarize_notebook_edit_args(tool_args: &serde_json::Value) -> Option<String> {
    let path = activity_extract_string(
        tool_args,
        &["notebook_path", "notebookPath", "path", "file_path"],
    );
    let mode = activity_extract_string(tool_args, &["edit_mode", "editMode"]);
    let summary = match (path, mode) {
        (Some(path), Some(mode)) => format!("Notebook Edit → {} {}", mode, path),
        (Some(path), None) => format!("Notebook Edit → {}", path),
        (None, Some(mode)) => format!("Notebook Edit → {}", mode),
        (None, None) => "Notebook Edit".to_string(),
    };
    Some(summary)
}

// ── Shared helpers ─────────────────────────────────────────────────────

/// Extract the first non-empty string value for any of the given keys.
fn activity_extract_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(content) = object.get(*key).and_then(|v| v.as_str()) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Format an object's primitive fields as `key=value` pairs, omitting large
/// text blobs to keep the summary readable.
fn format_activity_primitive_args(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    const OMIT: &[&str] = &[
        "content",
        "new_string",
        "old_string",
        "new_source",
        "patch",
        "prompt",
        "questions",
        "todos",
        "body",
        "text",
    ];
    let mut parts = Vec::new();
    for (key, value) in object {
        if OMIT.contains(&key.as_str()) {
            continue;
        }
        let rendered = match value {
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    continue;
                }
                collapse_text(trimmed, 40)
            }
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => continue,
        };
        parts.push(format!("{key}={rendered}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("[{}]", parts.join(", ")))
    }
}

fn summarize_question_result(metadata: Option<&serde_json::Value>) -> Option<String> {
    let fields = metadata?
        .get("display.fields")
        .and_then(|value| value.as_array())?;
    if fields.is_empty() {
        return None;
    }
    let mut lines = vec![format!("Answered ({})", fields.len())];
    for field in fields.iter().take(3) {
        let key = field
            .get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("Question");
        let value = field
            .get("value")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        lines.push(format!(
            "- {}: {}",
            collapse_text(key, 48),
            collapse_text(value, 72)
        ));
    }
    Some(lines.join("\n"))
}

fn summarize_todo_result(metadata: Option<&serde_json::Value>) -> Option<String> {
    let todos = metadata?.get("todos").and_then(|value| value.as_array())?;
    if todos.is_empty() {
        return None;
    }
    let mut lines = vec![format!("Todo list ({})", todos.len())];
    for todo in todos.iter().take(5) {
        let content = todo.get("content").and_then(|value| value.as_str())?;
        let status = todo
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("pending");
        lines.push(format!(
            "- [{}] {}",
            prettify_token(status),
            collapse_text(content, 88)
        ));
    }
    Some(lines.join("\n"))
}

fn collapse_text(input: &str, max_chars: usize) -> String {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if index >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Extract todo items from a todowrite tool call's arguments so they can be
/// stored in the TodoManager (single authority for todo state — Art. 5).
fn extract_todo_items_from_args(
    tool_args: &serde_json::Value,
) -> Option<Vec<rocode_session::TodoInfo>> {
    let todos = tool_args.get("todos")?.as_array()?;
    if todos.is_empty() {
        return None;
    }
    let items = todos
        .iter()
        .filter_map(|todo| {
            let content = todo.get("content").and_then(|v| v.as_str())?.to_string();
            let status = todo
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string();
            let priority = todo
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string();
            Some(rocode_session::TodoInfo {
                content,
                status,
                priority,
            })
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn apply_scheduler_decision_metadata(stage_name: &str, message: &mut SessionMessage) {
    clear_scheduler_decision_metadata(message);
    let text = message.get_text();
    let body = scheduler_stage_body(&text);
    match stage_name {
        "route" => {
            let Some(decision) = parse_route_decision(&body) else {
                return;
            };
            write_scheduler_route_metadata(message, &decision);
        }
        "coordination-gate" | "autonomous-gate" => {
            let Some(decision) = parse_execution_gate_decision(&body) else {
                return;
            };
            write_scheduler_gate_metadata(message, &decision);
        }
        _ => {}
    }
}

fn clear_scheduler_decision_metadata(message: &mut SessionMessage) {
    for key in [
        "scheduler_decision_kind",
        "scheduler_decision_title",
        "scheduler_decision_fields",
        "scheduler_decision_sections",
        "scheduler_gate_status",
        "scheduler_gate_summary",
        "scheduler_gate_next_input",
        "scheduler_gate_final_response",
    ] {
        message.metadata.remove(key);
    }
}

fn write_scheduler_route_metadata(message: &mut SessionMessage, decision: &RouteDecision) {
    let mut fields = Vec::new();
    let (outcome, outcome_tone) = route_outcome_field(decision);
    fields.push(decision_field("Outcome", &outcome, Some(outcome_tone)));
    if let Some(preset) = decision.preset.as_deref().filter(|value| !value.is_empty()) {
        fields.push(decision_field(
            "Preset",
            &prettify_decision_value(preset),
            Some("info"),
        ));
    }
    if let Some(review_mode) = decision.review_mode {
        let raw = format!("{:?}", review_mode).to_ascii_lowercase();
        fields.push(decision_field(
            "Review",
            &prettify_decision_value(&raw),
            Some(if raw == "skip" { "warning" } else { "success" }),
        ));
    }
    if let Some(insert_plan_stage) = decision.insert_plan_stage {
        fields.push(decision_field(
            "Plan Stage",
            if insert_plan_stage { "Yes" } else { "No" },
            Some(if insert_plan_stage {
                "success"
            } else {
                "muted"
            }),
        ));
    }
    if !decision.rationale_summary.trim().is_empty() {
        fields.push(decision_field(
            "Why",
            decision.rationale_summary.trim(),
            None,
        ));
    }
    let mut sections = Vec::new();
    if let Some(context_append) = decision
        .context_append
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(decision_section("Appended Context", context_append));
    }
    if let Some(direct_response) = decision
        .direct_response
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(decision_section("Response", direct_response));
    }

    write_scheduler_decision_metadata(message, "route", "Decision", fields, sections);
}

fn write_scheduler_gate_metadata(
    message: &mut SessionMessage,
    decision: &SchedulerExecutionGateDecision,
) {
    let status = format!("{:?}", decision.status).to_ascii_lowercase();
    let mut fields = vec![decision_field(
        "Outcome",
        &gate_outcome_label(&status),
        Some("status"),
    )];
    if !decision.summary.is_empty() {
        fields.push(decision_field("Why", &decision.summary, None));
    }
    if let Some(next_input) = decision
        .next_input
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        fields.push(decision_field("Next Action", next_input, Some("warning")));
    }
    // Keep final_response out of the stage card. It is rendered as the final
    // assistant delivery; duplicating it here makes the conclusion appear
    // before the execution trace in both TUI and Web.
    write_scheduler_decision_metadata(message, "gate", "Decision", fields, Vec::new());
    message.metadata.insert(
        "scheduler_gate_status".to_string(),
        serde_json::json!(status),
    );
    if !decision.summary.is_empty() {
        message.metadata.insert(
            "scheduler_gate_summary".to_string(),
            serde_json::json!(decision.summary),
        );
    }
    if let Some(next_input) = decision
        .next_input
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        message.metadata.insert(
            "scheduler_gate_next_input".to_string(),
            serde_json::json!(next_input),
        );
    }
    if let Some(final_response) = decision
        .final_response
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        message.metadata.insert(
            "scheduler_gate_final_response".to_string(),
            serde_json::json!(final_response),
        );
    }
}

fn write_scheduler_decision_metadata(
    message: &mut SessionMessage,
    kind: &str,
    title: &str,
    fields: Vec<serde_json::Value>,
    sections: Vec<serde_json::Value>,
) {
    message.metadata.insert(
        "scheduler_decision_kind".to_string(),
        serde_json::json!(kind),
    );
    message.metadata.insert(
        "scheduler_decision_title".to_string(),
        serde_json::json!(title),
    );
    message.metadata.insert(
        "scheduler_decision_fields".to_string(),
        serde_json::json!(fields),
    );
    message.metadata.insert(
        "scheduler_decision_sections".to_string(),
        serde_json::json!(sections),
    );
    message.metadata.insert(
        "scheduler_decision_spec".to_string(),
        scheduler_decision_render_spec_json(),
    );
}

fn decision_field(label: &str, value: &str, tone: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "value": value,
        "tone": tone,
    })
}

fn decision_section(title: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "body": body,
    })
}

fn scheduler_decision_render_spec_json() -> serde_json::Value {
    serde_json::json!({
        "version": "decision-card/v1",
        "show_header_divider": true,
        "field_order": "as-provided",
        "field_label_emphasis": "bold",
        "status_palette": "semantic",
        "section_spacing": "loose",
        "update_policy": "stable-shell-live-runtime-append-decision",
    })
}

fn prettify_decision_value(raw: &str) -> String {
    raw.split(['-', '_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn route_outcome_field(decision: &RouteDecision) -> (String, &'static str) {
    match decision.mode {
        rocode_orchestrator::RouteMode::Direct => match decision.direct_kind {
            Some(rocode_orchestrator::DirectKind::Reply) => ("Direct Reply".to_string(), "warning"),
            Some(rocode_orchestrator::DirectKind::Clarify) => {
                ("Direct Clarification".to_string(), "warning")
            }
            None => ("Direct".to_string(), "warning"),
        },
        rocode_orchestrator::RouteMode::Orchestrate => ("Orchestrate".to_string(), "success"),
    }
}

fn gate_outcome_label(status: &str) -> String {
    match status {
        "continue" => "Continue".to_string(),
        "done" => "Done".to_string(),
        "blocked" => "Blocked".to_string(),
        other => prettify_decision_value(other),
    }
}

#[cfg(test)]
pub(crate) fn scheduler_stage_title(scheduler_profile: &str, stage_name: &str) -> String {
    format!(
        "{} · {}",
        pretty_scheduler_profile_name(scheduler_profile),
        pretty_scheduler_stage_name(stage_name)
    )
}

pub(crate) fn scheduler_stage_focus(stage_name: &str) -> &'static str {
    match stage_name {
        "route" => "Decide the correct workflow and preserve request intent.",
        "interview" => "Clarify scope, requirements, and blocking ambiguities.",
        "plan" => "Draft the executable plan and its guardrails.",
        "review" => "Audit the current artifact for gaps and readiness.",
        "handoff" => "Prepare the next-step handoff for execution or approval.",
        "execution-orchestration" => "Drive the active execution workflow to concrete results.",
        "synthesis" => "Merge stage outputs into a final user-facing delivery.",
        "coordination-verification" => "Verify delegated work against actual evidence.",
        "coordination-gate" => "Decide whether the coordination loop can finish.",
        "coordination-retry" => "Prepare the bounded retry focus for the next round.",
        "autonomous-verification" => "Verify autonomous execution against the task boundary.",
        "autonomous-gate" => "Decide whether autonomous execution is complete.",
        "autonomous-retry" => "Prepare the bounded recovery retry.",
        _ => "Advance the current scheduler stage.",
    }
}

pub(crate) const SCHEDULER_STAGE_PENDING_LAST_EVENT_KEY: &str =
    "scheduler_stage_pending_last_event";
pub(crate) const SCHEDULER_STAGE_PENDING_COMPACTION_PHASE_KEY: &str =
    "scheduler_stage_pending_compaction_phase";

fn pretty_scheduler_stage_name(stage_name: &str) -> String {
    prettify_token(stage_name)
}

fn prettify_token(token: &str) -> String {
    token
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pretty_scheduler_profile_name(profile: &str) -> String {
    prettify_token(profile)
}

pub(crate) async fn emit_scheduler_stage_message(input: SchedulerStageMessageInput<'_>) {
    let SchedulerStageMessageInput {
        state,
        session_id,
        scheduler_profile,
        stage_name,
        stage_index,
        stage_total,
        content,
        exec_ctx,
        output_hook,
    } = input;

    let body = content.trim();
    if body.is_empty() {
        return;
    }

    let mut sessions = state.sessions.lock().await;
    let Some(mut session) = sessions.get(session_id).cloned() else {
        return;
    };
    let pending_compaction_notice = take_pending_scheduler_stage_compaction_notice(&mut session);

    let message = session.add_assistant_message();
    let stage_id = format!("stage_{}", uuid::Uuid::new_v4().simple());
    message.metadata.insert(
        "scheduler_stage_id".to_string(),
        serde_json::json!(&stage_id),
    );
    message.metadata.insert(
        "scheduler_profile".to_string(),
        serde_json::json!(scheduler_profile),
    );
    message
        .metadata
        .insert("scheduler_stage".to_string(), serde_json::json!(stage_name));
    message.metadata.insert(
        "scheduler_stage_index".to_string(),
        serde_json::json!(stage_index),
    );
    message.metadata.insert(
        "scheduler_stage_total".to_string(),
        serde_json::json!(stage_total),
    );
    message.metadata.insert(
        "scheduler_stage_emitted".to_string(),
        serde_json::json!(true),
    );
    message.metadata.insert(
        "scheduler_stage_agent".to_string(),
        serde_json::json!(exec_ctx.agent_name.clone()),
    );
    message.metadata.insert(
        "scheduler_stage_status".to_string(),
        serde_json::json!(if body.starts_with("Stage error:") {
            "blocked"
        } else {
            "done"
        }),
    );
    message.metadata.insert(
        "scheduler_stage_focus".to_string(),
        serde_json::json!(scheduler_stage_focus(stage_name)),
    );
    message.metadata.insert(
        "scheduler_stage_last_event".to_string(),
        serde_json::json!(if body.starts_with("Stage error:") {
            "Stage failed"
        } else {
            "Stage completed"
        }),
    );
    if let Some((notice, phase)) = pending_compaction_notice {
        message.metadata.insert(
            "scheduler_stage_last_event".to_string(),
            serde_json::json!(notice),
        );
        message.metadata.insert(
            "context_compaction_notice".to_string(),
            serde_json::json!("Context compacted"),
        );
        if let Some(phase) = phase {
            message.metadata.insert(
                "context_compaction_phase".to_string(),
                serde_json::json!(phase),
            );
        }
    }
    message.metadata.insert(
        "scheduler_stage_waiting_on".to_string(),
        serde_json::json!("none"),
    );
    if let Some(observability) = scheduler_stage_observability(scheduler_profile, stage_name) {
        message.metadata.insert(
            "scheduler_stage_projection".to_string(),
            serde_json::json!(observability.projection),
        );
        message.metadata.insert(
            "scheduler_stage_tool_policy".to_string(),
            serde_json::json!(observability.tool_policy),
        );
        message.metadata.insert(
            "scheduler_stage_loop_budget".to_string(),
            serde_json::json!(observability.loop_budget),
        );
    }
    message.add_text(body.to_string());
    apply_scheduler_decision_metadata(stage_name, message);
    let message_snapshot = message.clone();
    session.touch();
    sessions.update(session);
    drop(sessions);

    let _ = state
        .runtime_telemetry
        .refresh_stage_summary_from_message(session_id, &message_snapshot)
        .await;
    if let Some(block) = scheduler_stage_block_from_message(&message_snapshot) {
        emit_output_block_via_hook(
            output_hook,
            OutputBlockEvent {
                session_id: session_id.to_string(),
                block: OutputBlock::SchedulerStage(Box::new(block)),
                id: Some(message_snapshot.id.clone()),
            },
        )
        .await;
    }
}

fn take_pending_scheduler_stage_compaction_notice(
    session: &mut rocode_session::Session,
) -> Option<(String, Option<String>)> {
    let notice = session
        .remove_metadata(SCHEDULER_STAGE_PENDING_LAST_EVENT_KEY)
        .and_then(|value| value.as_str().map(str::to_string))?;
    let phase = session
        .remove_metadata(SCHEDULER_STAGE_PENDING_COMPACTION_PHASE_KEY)
        .and_then(|value| value.as_str().map(str::to_string));
    Some((notice, phase))
}

pub(crate) struct SchedulerStageMessageInput<'a> {
    pub state: &'a Arc<ServerState>,
    pub session_id: &'a str,
    pub scheduler_profile: &'a str,
    pub stage_name: &'a str,
    pub stage_index: u32,
    pub stage_total: u32,
    pub content: &'a str,
    pub exec_ctx: &'a OrchestratorExecutionContext,
    pub output_hook: Option<&'a OutputBlockHook>,
}

pub fn assistant_visible_text(message: &SessionMessage) -> String {
    let mut out = String::new();
    for part in &message.parts {
        if let PartType::Text { text, ignored, .. } = &part.part_type {
            if ignored.unwrap_or(false) {
                continue;
            }
            out.push_str(text);
        }
    }
    rocode_session::sanitize_display_text(&out)
}

pub fn visible_assistant_text_from_orchestrator_output(output: &str) -> String {
    let trimmed = output.trim();
    if let Some(decision) = parse_route_decision(trimmed) {
        if matches!(decision.mode, rocode_orchestrator::RouteMode::Direct) {
            if let Some(response) = decision
                .direct_response
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return rocode_session::sanitize_display_text(response);
            }
        }
    }

    rocode_session::sanitize_display_text(output)
}

pub fn scheduler_stage_block_from_message(message: &SessionMessage) -> Option<SchedulerStageBlock> {
    let metadata = &message.metadata;
    let text = assistant_visible_text(message);

    // Delegate bulk field extraction to the shared canonical path.
    let mut block = SchedulerStageBlock::from_metadata(&text, metadata)?;

    // Override title when from_metadata() produced an empty title (no ## heading).
    if block.title.is_empty() {
        block.title = pretty_scheduler_stage_title(metadata, &block.stage);
    }

    // Enrich with decision block (requires full text + stage for contextual parsing).
    block.decision = scheduler_decision_block(metadata, &block.stage, &text);

    Some(block)
}

fn scheduler_decision_block(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    stage: &str,
    text: &str,
) -> Option<SchedulerDecisionBlock> {
    decision_from_metadata(metadata).or_else(|| decision_from_stage_text(stage, text))
}

fn decision_from_metadata(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<SchedulerDecisionBlock> {
    let kind = metadata
        .get("scheduler_decision_kind")
        .and_then(|value| value.as_str())?
        .to_string();
    let title = metadata
        .get("scheduler_decision_title")
        .and_then(|value| value.as_str())
        .unwrap_or("Decision")
        .to_string();
    Some(SchedulerDecisionBlock {
        kind,
        title,
        spec: decision_spec_from_metadata(metadata).unwrap_or_else(default_decision_render_spec),
        fields: metadata
            .get("scheduler_decision_fields")
            .and_then(|value| value.as_array())
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| {
                        Some(SchedulerDecisionField {
                            label: field.get("label")?.as_str()?.to_string(),
                            value: field.get("value")?.as_str()?.to_string(),
                            tone: field
                                .get("tone")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sections: metadata
            .get("scheduler_decision_sections")
            .and_then(|value| value.as_array())
            .map(|sections| {
                sections
                    .iter()
                    .filter_map(|section| {
                        Some(SchedulerDecisionSection {
                            title: section.get("title")?.as_str()?.to_string(),
                            body: section.get("body")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub fn decision_from_stage_text(stage: &str, text: &str) -> Option<SchedulerDecisionBlock> {
    let body = scheduler_stage_body(text);
    match stage {
        "route" => {
            let decision = parse_route_decision(&body)?;
            let mut fields = Vec::new();
            let (outcome, outcome_tone) = route_outcome_field(&decision);
            fields.push(SchedulerDecisionField {
                label: "Outcome".to_string(),
                value: outcome,
                tone: Some(outcome_tone.to_string()),
            });
            if let Some(preset) = decision.preset.as_deref().filter(|value| !value.is_empty()) {
                fields.push(SchedulerDecisionField {
                    label: "Preset".to_string(),
                    value: prettify_decision_value(preset),
                    tone: Some("info".to_string()),
                });
            }
            if let Some(review_mode) = decision.review_mode {
                let raw = format!("{:?}", review_mode).to_ascii_lowercase();
                fields.push(SchedulerDecisionField {
                    label: "Review".to_string(),
                    value: prettify_decision_value(&raw),
                    tone: Some(if raw == "skip" { "warning" } else { "success" }.to_string()),
                });
            }
            if let Some(insert_plan_stage) = decision.insert_plan_stage {
                fields.push(SchedulerDecisionField {
                    label: "Plan Stage".to_string(),
                    value: if insert_plan_stage { "Yes" } else { "No" }.to_string(),
                    tone: Some(
                        if insert_plan_stage {
                            "success"
                        } else {
                            "muted"
                        }
                        .to_string(),
                    ),
                });
            }
            if !decision.rationale_summary.trim().is_empty() {
                fields.push(SchedulerDecisionField {
                    label: "Why".to_string(),
                    value: decision.rationale_summary.trim().to_string(),
                    tone: None,
                });
            }
            let mut sections = Vec::new();
            if let Some(context_append) = decision
                .context_append
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sections.push(SchedulerDecisionSection {
                    title: "Appended Context".to_string(),
                    body: context_append.to_string(),
                });
            }
            if let Some(direct_response) = decision
                .direct_response
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                sections.push(SchedulerDecisionSection {
                    title: "Response".to_string(),
                    body: direct_response.to_string(),
                });
            }
            Some(SchedulerDecisionBlock {
                kind: "route".to_string(),
                title: "Decision".to_string(),
                spec: default_decision_render_spec(),
                fields,
                sections,
            })
        }
        "coordination-gate" | "autonomous-gate" => {
            let decision = parse_execution_gate_decision(&body)?;
            let mut fields = vec![SchedulerDecisionField {
                label: "Outcome".to_string(),
                value: gate_outcome_label(&format!("{:?}", decision.status).to_ascii_lowercase()),
                tone: Some("status".to_string()),
            }];
            if !decision.summary.is_empty() {
                fields.push(SchedulerDecisionField {
                    label: "Why".to_string(),
                    value: decision.summary,
                    tone: None,
                });
            }
            if let Some(next_input) = decision.next_input.filter(|value| !value.is_empty()) {
                fields.push(SchedulerDecisionField {
                    label: "Next Action".to_string(),
                    value: next_input,
                    tone: Some("warning".to_string()),
                });
            }
            Some(SchedulerDecisionBlock {
                kind: "gate".to_string(),
                title: "Decision".to_string(),
                spec: default_decision_render_spec(),
                fields,
                sections: Vec::new(),
            })
        }
        _ => None,
    }
}

fn decision_spec_from_metadata(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<SchedulerDecisionRenderSpec> {
    let spec = metadata.get("scheduler_decision_spec")?;
    Some(SchedulerDecisionRenderSpec {
        version: spec.get("version")?.as_str()?.to_string(),
        show_header_divider: spec.get("show_header_divider")?.as_bool()?,
        field_order: spec.get("field_order")?.as_str()?.to_string(),
        field_label_emphasis: spec.get("field_label_emphasis")?.as_str()?.to_string(),
        status_palette: spec.get("status_palette")?.as_str()?.to_string(),
        section_spacing: spec.get("section_spacing")?.as_str()?.to_string(),
        update_policy: spec.get("update_policy")?.as_str()?.to_string(),
    })
}

fn default_decision_render_spec() -> SchedulerDecisionRenderSpec {
    SchedulerDecisionRenderSpec {
        version: "decision-card/v1".to_string(),
        show_header_divider: true,
        field_order: "as-provided".to_string(),
        field_label_emphasis: "bold".to_string(),
        status_palette: "semantic".to_string(),
        section_spacing: "loose".to_string(),
        update_policy: "stable-shell-live-runtime-append-decision".to_string(),
    }
}

fn scheduler_stage_body(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("## ") {
        if let Some((_, body)) = rest.split_once('\n') {
            return body.trim_start().to_string();
        }
    }
    trimmed.to_string()
}

fn pretty_scheduler_stage_title(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
    stage: &str,
) -> String {
    let stage_title = prettify_decision_value(stage);
    match metadata
        .get("scheduler_profile")
        .and_then(|value| value.as_str())
    {
        Some(profile) if !profile.is_empty() => {
            format!("{} · {stage_title}", pretty_scheduler_profile_name(profile))
        }
        _ => stage_title,
    }
}

pub(crate) fn first_user_message_text(session: &Session) -> Option<String> {
    session
        .record()
        .messages
        .iter()
        .find(|message| matches!(message.role, MessageRole::User))
        .map(|message| message.get_text())
        .map(|text| rocode_session::sanitize_session_title_source(&text))
        .filter(|text| !text.is_empty())
}

pub(crate) async fn ensure_default_session_title(
    session: &mut Session,
    provider: Arc<dyn Provider>,
    model_id: &str,
) {
    let Some((_, fallback)) = rocode_session::compose_session_title_source(session) else {
        return;
    };

    let old_session_id = session.record().id.clone();
    let old_session_title = session.record().title.clone();
    if !session.allows_auto_title_regeneration() && old_session_title.trim() != fallback.trim() {
        return;
    }

    let generated_title =
        rocode_session::generate_session_title_for_session(session, provider, model_id).await;
    if !generated_title.trim().is_empty() {
        tracing::info!(
            session_id = %old_session_id,
            old_title = %old_session_title,
            new_title = %generated_title,
            "Session title refined by LLM"
        );
        session.set_title(generated_title);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
