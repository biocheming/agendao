//! HTTP SSE transport — Server-Sent Events via HTTP.
//!
//! Old TUI: spawn_server_event_listener_task() with reqwest + reqwest_eventsource.
//! New: same pattern, events → EventBus sender.
//!
//! Connects to /event?session={id}&tier=tui, parses FrontendEvent from SSE payload.

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use reqwest_eventsource::{Event as SseEvent, EventSource};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use agendao_server_core::frontend_events::FrontendEvent;

/// Spawn a background task that connects to the HTTP SSE endpoint.
/// Mirrors old TUI's spawn_server_event_listener_task().
pub fn spawn_http_event_source(
    tx: UnboundedSender<FrontendEvent>,
    base_url: String,
    server_password: Option<String>,
    handle: &tokio::runtime::Handle,
    session_filter: watch::Receiver<Option<String>>,
) -> Option<JoinHandle<()>> {
    let jh = handle.spawn(async move {
        let mut session_filter_rx = session_filter;
        let mut headers = HeaderMap::new();
        if let Some(ref password) = server_password {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {password}")) {
                headers.insert(AUTHORIZATION, value);
            }
        }

        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .default_headers(headers)
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(%err, "failed to create SSE client");
                return;
            }
        };

        let base_event_url = format!("{}/event", base_url.trim_end_matches('/'));
        let mut recovery_sync = false;

        loop {
            let connected = session_filter_rx.borrow().clone();
            let Some(ref sid) = connected else {
                recovery_sync = false;
                if session_filter_rx.changed().await.is_err() { break; }
                continue;
            };

            let event_url = build_url(&base_event_url, Some(sid));
            let mut source = match EventSource::new(client.get(event_url.clone())) {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(%err, url=%event_url, "SSE connect failed");
                    recovery_sync = true;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    continue;
                }
            };

            recovery_sync = consume_stream(
                &mut source, &tx, &mut session_filter_rx, connected, &mut recovery_sync,
            ).await;

            if recovery_sync {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    });
    Some(jh)
}

async fn consume_stream(
    source: &mut EventSource,
    tx: &UnboundedSender<FrontendEvent>,
    filter_rx: &mut watch::Receiver<Option<String>>,
    connected: Option<String>,
    recovery: &mut bool,
) -> bool {
    while let Some(event) = source.next().await {
        match event {
            Ok(SseEvent::Open) => {
                tracing::debug!(filter=?connected, "SSE connected");
                if *recovery { *recovery = false; }
            }
            Ok(SseEvent::Message(msg)) => {
                if let Some(fe) = parse_event(&msg.data) {
                    let sid = event_session_id(&fe);
                    if filter_rx.borrow().as_deref() == sid {
                        if tx.send(fe).is_err() { return true; }
                    }
                }
                // Check if session filter changed
                if *filter_rx.borrow() != connected {
                    source.close();
                    return false;
                }
            }
            Err(err) => {
                tracing::debug!(%err, "SSE disconnected");
                return true; // reconnect
            }
        }
    }
    false
}

fn build_url(base: &str, session_id: Option<&str>) -> Url {
    let mut url = Url::parse(base).expect("invalid SSE base URL");
    if let Some(sid) = session_id {
        url.query_pairs_mut().append_pair("session", sid);
    }
    url.query_pairs_mut().append_pair("tier", "tui");
    url
}

fn parse_event(payload: &str) -> Option<FrontendEvent> {
    let payload = payload.trim();
    if payload.is_empty() { return None; }
    serde_json::from_str::<FrontendEvent>(payload).ok()
}

fn event_session_id(event: &FrontendEvent) -> Option<&str> {
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
