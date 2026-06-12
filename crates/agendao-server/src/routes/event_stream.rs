use super::*;
use agendao_server_core::frontend_events::FrontendEvent;

#[derive(Debug, Deserialize)]
pub(super) struct EventStreamQuery {
    /// Optional session ID to filter events by. When set, only events belonging
    /// to this session (or global events like `config.updated`) are forwarded.
    #[serde(default)]
    session: Option<String>,
    /// P2-1: subscription tier override (tui, web, cli). When absent, the
    /// server applies the legacy compatible default (full capabilities).
    #[serde(default)]
    tier: Option<String>,
}

pub(super) async fn event_stream(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<EventStreamQuery>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let subscription =
        agendao_api::ResolvedFrontendSubscription::from_wire_tier(query.tier.as_deref());
    tracing::debug!(
        tier = query.tier.as_deref().unwrap_or("default"),
        is_legacy = subscription.is_legacy_compat,
        "resolved frontend subscription for /event SSE"
    );
    stream_frontend_events(state.frontend_bus.subscribe(), query.session, subscription)
}

const EVENT_OUTPUT_BLOCK_BATCH_MS: u64 = 16;

pub(crate) fn stream_server_events(
    mut rx: broadcast::Receiver<String>,
    session_filter: Option<String>,
    subscription: agendao_api::ResolvedFrontendSubscription,
    event_bus_telemetry: Option<std::sync::Arc<crate::session_runtime::events::EventBusTelemetry>>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let (tx, out_rx) = mpsc::channel(128);

    tokio::spawn(async move {
        let mut pending: Option<ServerEvent> = None;
        let mut pending_due_at: Option<tokio::time::Instant> = None;
        let delay = std::time::Duration::from_millis(EVENT_OUTPUT_BLOCK_BATCH_MS);

        let matches_filter = |event: &ServerEvent| -> bool {
            let Some(ref filter) = session_filter else {
                return true;
            };
            match event.session_id() {
                Some(sid) => sid == filter.as_str(),
                None => true,
            }
        };

        let mut snapshot_coalescer = match event_bus_telemetry {
            Some(ref telemetry) => LiveSnapshotCoalescer::with_telemetry(telemetry.clone()),
            None => LiveSnapshotCoalescer::new(),
        };

        let caps = subscription.capabilities;
        let skipped_count = std::sync::atomic::AtomicU64::new(0);
        let subscribable = |event: &ServerEvent| -> bool {
            let ok = event_passes_subscription_caps(event, &caps);
            if !ok {
                skipped_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            ok
        };

        let raw_matches_filter = |raw: &str| -> bool {
            let Some(ref filter) = session_filter else {
                return true;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
                return true;
            };
            match value.get("sessionID").and_then(|v| v.as_str()) {
                Some(sid) => sid == filter.as_str(),
                None => match value.get("parentID").and_then(|v| v.as_str()) {
                    Some(pid) => pid == filter.as_str(),
                    None => true,
                },
            }
        };

        loop {
            if pending.is_some() {
                let due_at = pending_due_at.unwrap_or_else(|| tokio::time::Instant::now() + delay);
                tokio::select! {
                    recv = rx.recv() => {
                        match recv {
                            Ok(raw) => {
                                if let Some(next) = parse_server_event(&raw) {
                                    if !matches_filter(&next) {
                                        continue;
                                    }
                                    let next = snapshot_coalescer.coalesce(next);
                                    if !subscribable(&next) {
                                        continue;
                                    }
                                    if let Some(current) = pending.as_mut() {
                                        if merge_output_block_delta(current, &next) {
                                            continue;
                                        }
                                    }
                                    if let Some(flushed) = pending.take() {
                                        pending_due_at = None;
                                        if send_server_event_json(&tx, &flushed).await.is_err() {
                                            break;
                                        }
                                    }
                                    if is_mergeable_output_delta(&next) {
                                        pending = Some(next);
                                        pending_due_at = Some(tokio::time::Instant::now() + delay);
                                    } else if send_server_event_json(&tx, &next).await.is_err() {
                                        break;
                                    }
                                } else {
                                    if !raw_matches_filter(&raw) {
                                        continue;
                                    }
                                    if let Some(flushed) = pending.take() {
                                        pending_due_at = None;
                                        if send_server_event_json(&tx, &flushed).await.is_err() {
                                            break;
                                        }
                                    }
                                    if send_raw_server_event(&tx, raw).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                if let Some(flushed) = pending.take() {
                                    pending_due_at = None;
                                    if send_server_event_json(&tx, &flushed).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                let skipped =
                                    skipped_count.load(std::sync::atomic::Ordering::Relaxed);
                                if skipped > 0 {
                                    tracing::debug!(
                                        skipped,
                                        tier = ?subscription.tier,
                                        "SSE event stream closed; subscription-filtered events skipped"
                                    );
                                }
                                if let Some(flushed) = pending.take() {
                                    if let Err(error) = send_server_event_json(&tx, &flushed).await {
                                        let _ = error;
                                        tracing::debug!(
                                            "Failed to flush pending server event after broadcast channel closed"
                                        );
                                    }
                                }
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(due_at) => {
                        if let Some(flushed) = pending.take() {
                            pending_due_at = None;
                            if send_server_event_json(&tx, &flushed).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            } else {
                match rx.recv().await {
                    Ok(raw) => {
                        if let Some(event) = parse_server_event(&raw) {
                            if !matches_filter(&event) {
                                continue;
                            }
                            let event = snapshot_coalescer.coalesce(event);
                            if !subscribable(&event) {
                                continue;
                            }
                            if is_mergeable_output_delta(&event) {
                                pending = Some(event);
                                pending_due_at = Some(tokio::time::Instant::now() + delay);
                            } else if send_server_event_json(&tx, &event).await.is_err() {
                                break;
                            }
                        } else {
                            if !raw_matches_filter(&raw) {
                                continue;
                            }
                            if send_raw_server_event(&tx, raw).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(out_rx))
}

pub(crate) fn stream_frontend_events(
    mut rx: broadcast::Receiver<String>,
    session_filter: Option<String>,
    subscription: agendao_api::ResolvedFrontendSubscription,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let (tx, out_rx) = mpsc::channel(128);

    tokio::spawn(async move {
        let caps = subscription.capabilities;
        let skipped_count = std::sync::atomic::AtomicU64::new(0);
        loop {
            match rx.recv().await {
                Ok(raw) => {
                    if !frontend_raw_matches_filter(&raw, session_filter.as_deref()) {
                        continue;
                    }
                    let Ok(event) = serde_json::from_str::<FrontendEvent>(&raw) else {
                        continue;
                    };
                    if !frontend_event_passes_subscription_caps(&event, &caps) {
                        skipped_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        continue;
                    }
                    if send_raw_server_event(&tx, raw).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    let skipped = skipped_count.load(std::sync::atomic::Ordering::Relaxed);
                    if skipped > 0 {
                        tracing::debug!(
                            skipped,
                            tier = ?subscription.tier,
                            "SSE frontend event stream closed; subscription-filtered events skipped"
                        );
                    }
                    break;
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(out_rx))
}

pub(super) struct LiveSnapshotCoalescer {
    pub(super) accum: std::collections::HashMap<String, String>,
    telemetry: Option<std::sync::Arc<crate::session_runtime::events::EventBusTelemetry>>,
}

fn key_for(session_id: &str, identity: &agendao_types::LiveMessagePartIdentity) -> String {
    format!(
        "{}:{}:{}",
        session_id, identity.message_id, identity.part_key
    )
}

impl LiveSnapshotCoalescer {
    pub(super) fn new() -> Self {
        Self {
            accum: std::collections::HashMap::new(),
            telemetry: None,
        }
    }

    pub(super) fn with_telemetry(
        telemetry: std::sync::Arc<crate::session_runtime::events::EventBusTelemetry>,
    ) -> Self {
        Self {
            accum: std::collections::HashMap::new(),
            telemetry: Some(telemetry),
        }
    }

    pub(super) fn coalesce(&mut self, event: ServerEvent) -> ServerEvent {
        let ServerEvent::OutputBlock {
            session_id,
            mut block,
            id,
            live_identity,
        } = event
        else {
            return event;
        };
        let Some(ref identity) = live_identity else {
            if let Some(ref telemetry) = self.telemetry {
                telemetry.record_identity_missing();
            }
            return ServerEvent::OutputBlock {
                session_id,
                block,
                id,
                live_identity,
            };
        };

        let coalesce_field = match identity.part_kind {
            agendao_types::LiveMessagePartKind::AssistantText
            | agendao_types::LiveMessagePartKind::AssistantReasoning => "text",
            agendao_types::LiveMessagePartKind::ToolCall => "detail",
            _ => {
                return ServerEvent::OutputBlock {
                    session_id,
                    block,
                    id,
                    live_identity,
                };
            }
        };

        if identity.phase == agendao_types::LivePartPhase::End {
            let key = key_for(&session_id, identity);
            self.accum.remove(&key);
            return ServerEvent::OutputBlock {
                session_id,
                block,
                id,
                live_identity,
            };
        }

        if !matches!(
            identity.phase,
            agendao_types::LivePartPhase::Append | agendao_types::LivePartPhase::Snapshot
        ) {
            return ServerEvent::OutputBlock {
                session_id,
                block,
                id,
                live_identity,
            };
        }

        let key = key_for(&session_id, identity);
        let text = block
            .get(coalesce_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let accumulated = if identity.phase == agendao_types::LivePartPhase::Append {
            self.accum.entry(key.clone()).or_default().push_str(text);
            self.accum[&key].clone()
        } else {
            let merged = merge_snapshot_text(self.accum.get(&key).map(String::as_str), text);
            self.accum.insert(key, merged.clone());
            merged
        };

        if let Some(obj) = block.as_object_mut() {
            obj.insert(coalesce_field.to_string(), serde_json::json!(accumulated));
            obj.insert("phase".to_string(), serde_json::json!("full"));
        }
        if let Some(ref telemetry) = self.telemetry {
            telemetry.record_coalesced_snapshot();
            telemetry.record_full_snapshot_emitted();
        }
        ServerEvent::OutputBlock {
            session_id,
            block,
            id,
            live_identity: Some(agendao_types::LiveMessagePartIdentity {
                phase: agendao_types::LivePartPhase::Snapshot,
                ..identity.clone()
            }),
        }
    }
}

fn merge_snapshot_text(existing: Option<&str>, incoming: &str) -> String {
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

    let overlap = suffix_prefix_overlap(existing, incoming);
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

fn parse_server_event(raw: &str) -> Option<ServerEvent> {
    serde_json::from_str(raw).ok()
}

fn frontend_raw_matches_filter(raw: &str, session_filter: Option<&str>) -> bool {
    let Some(filter) = session_filter else {
        return true;
    };
    let Ok(event) = serde_json::from_str::<FrontendEvent>(raw) else {
        return false;
    };
    frontend_event_session_id(&event) == Some(filter)
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
        | FrontendEvent::OutputBlockAppended { session_id, .. } => Some(session_id.as_str()),
    }
}

fn frontend_event_passes_subscription_caps(
    event: &FrontendEvent,
    caps: &agendao_api::FrontendSubscriptionCapabilities,
) -> bool {
    if !caps.final_only
        && caps.reasoning_delta
        && caps.message_text_delta
        && caps.tool_progress
        && caps.runtime_live_view
    {
        return true;
    }

    match event {
        FrontendEvent::OutputBlockAppended { block, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "reasoning" => !caps.final_only && (phase != "delta" || caps.reasoning_delta),
                "message" => !caps.final_only && caps.message_text_delta,
                "scheduler_stage" => !caps.final_only && caps.tool_progress,
                "tool" => {
                    matches!(phase, "done" | "error") || (!caps.final_only && caps.tool_progress)
                }
                _ => !caps.final_only,
            }
        }
        FrontendEvent::SessionRuntimeReplaced { .. }
        | FrontendEvent::SessionProjectionReplaced { .. }
        | FrontendEvent::QuestionUpsert { .. }
        | FrontendEvent::QuestionRemoved { .. }
        | FrontendEvent::PermissionUpsert { .. }
        | FrontendEvent::PermissionRemoved { .. }
        | FrontendEvent::ToolCallUpsert { .. }
        | FrontendEvent::DiffReplaced { .. } => true,
    }
}

pub(super) fn event_passes_subscription_caps(
    event: &ServerEvent,
    caps: &agendao_api::FrontendSubscriptionCapabilities,
) -> bool {
    if !caps.final_only
        && caps.reasoning_delta
        && caps.message_text_delta
        && caps.tool_progress
        && caps.runtime_live_view
    {
        return true;
    }
    match event {
        ServerEvent::OutputBlock { block, .. } => {
            let kind = block.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let phase = block.get("phase").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "reasoning" => !caps.final_only && (phase != "delta" || caps.reasoning_delta),
                "message" => !caps.final_only && caps.message_text_delta,
                "scheduler_stage" => !caps.final_only && caps.tool_progress,
                "tool" => {
                    matches!(phase, "done" | "error") || (!caps.final_only && caps.tool_progress)
                }
                _ => !caps.final_only,
            }
        }
        ServerEvent::Usage { .. } => !caps.final_only && caps.runtime_live_view,
        ServerEvent::SessionUpdated { .. }
        | ServerEvent::SessionStatus { .. }
        | ServerEvent::Error { .. }
        | ServerEvent::PermissionRequested { .. }
        | ServerEvent::PermissionResolved { .. }
        | ServerEvent::QuestionCreated { .. }
        | ServerEvent::QuestionResolved { .. }
        | ServerEvent::ToolCallLifecycle { .. }
        | ServerEvent::ConfigUpdated
        | ServerEvent::TopologyChanged { .. }
        | ServerEvent::AttachedSessionAttached { .. }
        | ServerEvent::AttachedSessionDetached { .. }
        | ServerEvent::DiffUpdated { .. }
        | ServerEvent::ControlInputTransition { .. } => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeableLiveTextMode {
    AppendDelta,
    ReplaceSnapshot,
}

fn mergeable_live_text_mode(event: &ServerEvent) -> Option<MergeableLiveTextMode> {
    let ServerEvent::OutputBlock {
        id,
        block,
        live_identity,
        ..
    } = event
    else {
        return None;
    };
    if id.as_deref().is_none_or(str::is_empty) {
        return None;
    }
    let kind = block.get("kind").and_then(|value| value.as_str())?;
    if !matches!(kind, "message" | "reasoning") {
        return None;
    }
    match block.get("phase").and_then(|value| value.as_str()) {
        Some("delta") => Some(MergeableLiveTextMode::AppendDelta),
        Some("full")
            if live_identity.as_ref().is_some_and(|identity| {
                matches!(
                    identity.part_kind,
                    agendao_types::LiveMessagePartKind::AssistantText
                        | agendao_types::LiveMessagePartKind::AssistantReasoning
                ) && identity.phase == agendao_types::LivePartPhase::Snapshot
            }) =>
        {
            Some(MergeableLiveTextMode::ReplaceSnapshot)
        }
        _ => None,
    }
}

pub(super) fn is_mergeable_output_delta(event: &ServerEvent) -> bool {
    mergeable_live_text_mode(event).is_some()
}

pub(super) fn merge_output_block_delta(current: &mut ServerEvent, next: &ServerEvent) -> bool {
    let Some(current_mode) = mergeable_live_text_mode(current) else {
        return false;
    };
    let Some(next_mode) = mergeable_live_text_mode(next) else {
        return false;
    };
    if current_mode != next_mode {
        return false;
    }

    let (
        ServerEvent::OutputBlock {
            session_id: current_session,
            id: current_id,
            block: current_block,
            live_identity: current_identity,
            ..
        },
        ServerEvent::OutputBlock {
            session_id: next_session,
            id: next_id,
            block: next_block,
            live_identity: next_identity,
            ..
        },
    ) = (current, next)
    else {
        return false;
    };

    if current_session != next_session || current_id != next_id {
        return false;
    }

    let current_kind = current_block.get("kind").and_then(|value| value.as_str());
    let next_kind = next_block.get("kind").and_then(|value| value.as_str());
    if current_kind != next_kind {
        return false;
    }
    if current_kind == Some("message")
        && current_block.get("role").and_then(|value| value.as_str())
            != next_block.get("role").and_then(|value| value.as_str())
    {
        return false;
    }

    match current_mode {
        MergeableLiveTextMode::AppendDelta => {
            let Some(next_text) = next_block.get("text").and_then(|value| value.as_str()) else {
                return false;
            };
            let Some(current_text) = current_block
                .get_mut("text")
                .and_then(|value| value.as_str())
            else {
                return false;
            };

            current_block["text"] = serde_json::Value::String(format!("{current_text}{next_text}"));
            true
        }
        MergeableLiveTextMode::ReplaceSnapshot => {
            let (Some(current_identity_ref), Some(next_identity_ref)) =
                (current_identity.as_ref(), next_identity.as_ref())
            else {
                return false;
            };
            if current_identity_ref.message_id != next_identity_ref.message_id
                || current_identity_ref.part_key != next_identity_ref.part_key
                || current_identity_ref.part_kind != next_identity_ref.part_kind
            {
                return false;
            }
            *current_block = next_block.clone();
            *current_identity = Some(next_identity_ref.clone());
            true
        }
    }
}

async fn send_raw_server_event(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    raw: String,
) -> std::result::Result<(), ()> {
    tx.send(Ok(Event::default().data(raw)))
        .await
        .map_err(|_| ())
}

async fn send_server_event_json(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    event: &ServerEvent,
) -> std::result::Result<(), ()> {
    let Some(json) = event.to_json_string() else {
        return Ok(());
    };
    send_raw_server_event(tx, json).await
}
