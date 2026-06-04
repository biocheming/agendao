use std::convert::Infallible;
use std::sync::Arc;

pub use agendao_server_core::runtime_events::{
    DiffEntry, EventBusTelemetry, QuestionResolutionKind, ReconcileReason, ServerEvent,
};
use agendao_session::prompt::{OutputBlockEvent, OutputBlockHook};
use axum::response::sse::Event;
use tokio::sync::mpsc;

use crate::ServerState;

pub(crate) fn server_output_block_event(event: &OutputBlockEvent) -> ServerEvent {
    ServerEvent::output_block(
        event.session_id.clone(),
        &event.block,
        event.id.as_deref(),
        event.live_identity.clone(),
    )
}

pub(crate) async fn send_sse_server_event(
    tx: &mpsc::Sender<std::result::Result<Event, Infallible>>,
    event: &ServerEvent,
) {
    let sse_event = Event::default()
        .event(event.event_name())
        .json_data(event)
        .ok();
    if let Some(sse_event) = sse_event {
        if let Err(error) = tx.send(Ok(sse_event)).await {
            tracing::debug!(
                error = %error,
                "Failed to send SSE server event to runtime subscriber"
            );
        }
    }
}

pub(crate) fn broadcast_server_event(state: &ServerState, event: &ServerEvent) {
    if let Some(payload) = event.to_json_string() {
        state.broadcast(&payload);
    }
}

pub(crate) fn broadcast_output_block_event(state: &ServerState, event: &OutputBlockEvent) {
    let server_event = server_output_block_event(event);
    broadcast_server_event(state, &server_event);
}

pub(crate) fn server_output_block_hook(state: Arc<ServerState>) -> OutputBlockHook {
    Arc::new(move |event| {
        let state = state.clone();
        Box::pin(async move {
            broadcast_output_block_event(state.as_ref(), &event);
        })
    })
}

pub(crate) async fn emit_output_block_via_hook(
    output_hook: Option<&OutputBlockHook>,
    event: OutputBlockEvent,
) {
    let Some(output_hook) = output_hook else {
        return;
    };
    output_hook(event).await;
}

pub(crate) fn sse_output_block_hook(
    tx: mpsc::Sender<std::result::Result<Event, Infallible>>,
) -> OutputBlockHook {
    Arc::new(move |event| {
        let tx = tx.clone();
        Box::pin(async move {
            let server_event = server_output_block_event(&event);
            send_sse_server_event(&tx, &server_event).await;
        })
    })
}

pub(crate) fn broadcast_session_reconcile(
    state: &ServerState,
    session_id: impl Into<String>,
    reason: ReconcileReason,
) {
    let session_id = session_id.into();
    let source = reason.as_str();
    broadcast_server_event(
        state,
        &ServerEvent::SessionUpdated {
            session_id: session_id.clone(),
            source: source.to_string(),
        },
    );
    let telemetry = state.runtime_telemetry.clone();
    tokio::spawn(async move {
        telemetry.record_session_updated(&session_id, &source).await;
    });
}

pub(crate) fn broadcast_config_updated(state: &ServerState) {
    broadcast_server_event(state, &ServerEvent::ConfigUpdated);
}

#[cfg(test)]
mod tests {
    use super::{broadcast_session_reconcile, EventBusTelemetry, ReconcileReason};
    use crate::ServerState;

    #[test]
    fn broadcast_session_reconcile_emits_server_event_payload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async {
            let state = ServerState::new();
            let mut rx = state.event_bus.subscribe();

            broadcast_session_reconcile(&state, "session-1", ReconcileReason::TurnFinal);

            let payload = rx.recv().await.expect("session.updated payload");
            let value: serde_json::Value =
                serde_json::from_str(&payload).expect("valid json payload");
            assert_eq!(value["type"], "session.updated");
            assert_eq!(value["sessionID"], "session-1");
            assert_eq!(value["source"], "turn.final");
        });
    }

    #[test]
    fn event_bus_telemetry_snapshot_reports_counters() {
        let telemetry = EventBusTelemetry::default();
        telemetry.record_send(3);
        telemetry.record_send_error();

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.send_count, 1);
        assert_eq!(snapshot.send_error_count, 1);
        assert_eq!(snapshot.max_receivers, 3);
        assert!(snapshot.last_send_at_ms > 0);
        assert!(snapshot.last_send_error_at_ms > 0);
    }
}
