use super::*;
use std::sync::Arc;

pub(super) struct InteractiveSessionStream {
    pub(super) sse_rx: mpsc::UnboundedReceiver<CliServerEvent>,
    pub(super) sse_cancel: CancellationToken,
}

pub(super) async fn bootstrap_interactive_stream(
    server_url: &str,
    server_session_id: &str,
    api_client: &Arc<CliApiClient>,
    runtime: &CliExecutionRuntime,
    local: bool,
    local_state: &Option<Arc<crate::local_server_bridge::CliLocalServerState>>,
    transport: &Option<Arc<agendao_client::FrontendTransport>>,
    unix_socket_path: Option<String>,
) -> InteractiveSessionStream {
    let (sse_tx, sse_rx) = mpsc::unbounded_channel::<CliServerEvent>();
    let sse_cancel = CancellationToken::new();

    if local {
        // Direct mode: use the shared DirectEventBridge (agendao-server).
        // A thin adapter converts DirectEvent → CliServerEvent.
        if let Some(state) = local_state {
            let direct_rx = crate::local_server_bridge::spawn_direct_event_loop(
                Arc::clone(state),
                server_session_id.to_string(),
                sse_cancel.clone(),
            );
            let tx = sse_tx.clone();
            tokio::spawn(async move {
                cli_direct_event_adapter(direct_rx, tx).await;
            });
        }
        return InteractiveSessionStream { sse_rx, sse_cancel };
    }

    // Unix socket: subscribe to events via JSON-RPC.
    if let Some(agendao_client::FrontendTransport::Unix(_unix)) = transport.as_deref() {
        let socket_path = unix_socket_path.clone().unwrap_or_default();
        match _unix.subscribe_events(server_session_id).await {
            Ok(json_rx) => {
                let tx = sse_tx.clone();
                let sid = server_session_id.to_string();
                tokio::spawn(async move {
                    cli_socket_event_loop(&socket_path, sid, json_rx, tx).await;
                });
                return InteractiveSessionStream { sse_rx, sse_cancel };
            }
            Err(e) => {
                tracing::warn!(%e, "socket subscribe_events failed, falling back to SSE");
            }
        }
    }

    let _sse_handle = event_stream::spawn_sse_subscriber(
        server_url.to_string(),
        server_session_id.to_string(),
        sse_tx,
        sse_cancel.clone(),
    );

    cli_refresh_server_info(
        api_client,
        &runtime.frontend_projection,
        Some(server_session_id),
    )
    .await;

    InteractiveSessionStream { sse_rx, sse_cancel }
}

fn direct_event_to_cli_event(
    event: crate::local_server_bridge::CliDirectEvent,
) -> Option<CliServerEvent> {
    use crate::local_server_bridge::CliDirectEvent as DirectEvent;
    Some(match event {
        DirectEvent::SessionBusy { session_id } => CliServerEvent::SessionBusy { session_id },
        DirectEvent::SessionIdle { session_id } => CliServerEvent::SessionIdle { session_id },
        DirectEvent::SessionUpdated { session_id } => CliServerEvent::SessionUpdated {
            session_id,
            source: Some("direct_bridge".to_string()),
        },
        DirectEvent::QuestionCreated {
            session_id,
            request_id,
            questions_json,
        } => CliServerEvent::QuestionCreated {
            session_id,
            request_id,
            questions_json: questions_json.unwrap_or(serde_json::Value::Null),
        },
        DirectEvent::QuestionResolved { request_id } => {
            CliServerEvent::QuestionResolved { request_id }
        }
        DirectEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        } => CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json: info_json.unwrap_or(serde_json::Value::Null),
        },
        DirectEvent::PermissionResolved {
            session_id,
            permission_id,
        } => CliServerEvent::PermissionResolved {
            session_id,
            permission_id,
        },
        DirectEvent::ToolCallStarted { session_id } => CliServerEvent::ToolCallStarted {
            session_id,
            tool_call_id: String::new(),
            tool_name: String::new(),
        },
        DirectEvent::ToolCallCompleted { session_id } => CliServerEvent::ToolCallCompleted {
            session_id,
            tool_call_id: String::new(),
        },
        DirectEvent::OutputBlock { session_id, block } => CliServerEvent::OutputBlock {
            session_id,
            id: None,
            payload: block,
            live_identity: None,
        },
        DirectEvent::ConfigUpdated => CliServerEvent::ConfigUpdated,
        DirectEvent::TopologyChanged { session_id } => CliServerEvent::SessionUpdated {
            session_id,
            source: Some("direct_topology".to_string()),
        },
        DirectEvent::ControlInputTransition { .. }
        | DirectEvent::DiffUpdated { .. }
        | DirectEvent::SessionTreeChanged { .. } => return None,
    })
}

/// Thin adapter: DirectEvent → CliServerEvent. Runs until the sender
/// channel closes or the receiver is exhausted.
async fn cli_socket_event_loop(
    socket_path: &str,
    session_id: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
    tx: tokio::sync::mpsc::UnboundedSender<CliServerEvent>,
) {
    loop {
        while let Some(json) = rx.recv().await {
            if let Ok(direct) =
                serde_json::from_value::<crate::local_server_bridge::CliDirectEvent>(json)
            {
                if let Some(cli) = direct_event_to_cli_event(direct) {
                    if tx.send(cli).is_err() {
                        return;
                    }
                }
            }
        }
        // Stream ended — reconnect.
        let transport =
            agendao_client::transport::UnixSocketTransport::new(socket_path.to_string());
        match transport.subscribe_events(&session_id).await {
            Ok(new_rx) => rx = new_rx,
            Err(e) => {
                tracing::warn!(%e, "socket subscribe_events reconnect failed");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

async fn cli_direct_event_adapter(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<crate::local_server_bridge::CliDirectEvent>,
    tx: tokio::sync::mpsc::UnboundedSender<CliServerEvent>,
) {
    while let Some(event) = rx.recv().await {
        if let Some(cli_event) = direct_event_to_cli_event(event) {
            if tx.send(cli_event).is_err() {
                break;
            }
        }
    }
}
