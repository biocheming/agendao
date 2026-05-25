use super::*;

pub(super) struct InteractiveSessionStream {
    pub(super) sse_rx: mpsc::UnboundedReceiver<CliServerEvent>,
    pub(super) sse_cancel: CancellationToken,
}

pub(super) async fn bootstrap_interactive_stream(
    server_url: &str,
    server_session_id: &str,
    api_client: &Arc<CliApiClient>,
    runtime: &CliExecutionRuntime,
) -> InteractiveSessionStream {
    let (sse_tx, sse_rx) = mpsc::unbounded_channel::<CliServerEvent>();
    let sse_cancel = CancellationToken::new();
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
