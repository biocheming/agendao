use super::*;
use std::sync::Arc;

pub(super) async fn fetch_server_model_list(
    api_client: &Arc<CliApiClient>,
    local_state: &Option<Arc<agendao_server::ServerState>>,
    transport: &Option<Arc<agendao_client::FrontendTransport>>,
) -> Option<Vec<String>> {
    let response = crate::local_dispatch::get_all_providers(local_state, transport, api_client)
        .await
        .ok();
    response.map(|r| {
        let mut models: Vec<String> = r
            .all
            .into_iter()
            .flat_map(|provider| {
                let provider_id = provider.id;
                provider
                    .models
                    .into_iter()
                    .map(move |model| format!("{}/{}", provider_id, model.id))
            })
            .collect();
        models.sort();
        models.dedup();
        models
    })
}

pub(super) fn attach_rich_prompt(
    runtime: &mut CliExecutionRuntime,
    repl_style: &CliStyle,
    current_dir: &Path,
    config: &Config,
    provider_registry: &ProviderRegistry,
    agent_registry: &AgentRegistry,
    recent_session_info: Option<&CliRecentSessionInfo>,
    server_model_list: Option<Vec<String>>,
) -> anyhow::Result<mpsc::UnboundedReceiver<CliDispatchInput>> {
    let shared_frontend_projection = runtime.frontend_projection.clone();
    let queued_inputs = runtime.queued_inputs.clone();
    let busy_flag = runtime.busy_flag.clone();
    let exit_requested = runtime.exit_requested.clone();
    let active_abort = runtime.active_abort.clone();
    let terminal_surface = Arc::new(CliTerminalSurface::new(
        repl_style.clone(),
        runtime.frontend_projection.clone(),
    ));
    let mut prompt_chrome = CliPromptChrome::new(
        runtime,
        repl_style,
        current_dir,
        config,
        provider_registry,
        agent_registry,
    );
    prompt_chrome.set_show_transcript_tail(false);
    let prompt_chrome = Arc::new(prompt_chrome);
    if let Some(models) = server_model_list {
        prompt_chrome.update_model_catalog(models);
    }
    let (prompt_event_tx, mut prompt_event_rx) = mpsc::unbounded_channel();
    let prompt_session = Arc::new(PromptSession::spawn(
        Arc::new({
            let prompt_chrome = prompt_chrome.clone();
            move |line, cursor_pos| prompt_chrome.frame(line, cursor_pos)
        }),
        Some(Arc::new({
            let prompt_chrome = prompt_chrome.clone();
            move |line, cursor_pos| prompt_chrome.assist(line, cursor_pos).completion
        })),
        prompt_event_tx,
    )?);
    tokio::spawn({
        let frontend_projection = shared_frontend_projection.clone();
        let terminal_surface = terminal_surface.clone();
        let exit_requested = exit_requested.clone();
        let busy_flag = busy_flag.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                let should_animate = frontend_projection
                    .lock()
                    .map(|projection| projection.footer_should_animate())
                    .unwrap_or(false);
                if should_animate {
                    let _ = terminal_surface.refresh_prompt();
                }
                if exit_requested.load(Ordering::SeqCst) && !busy_flag.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    });
    terminal_surface.set_prompt_chrome(prompt_chrome.clone());
    terminal_surface.set_prompt_session(prompt_session.clone());
    terminal_surface.set_busy_flag(busy_flag.clone());
    terminal_surface
        .print_ephemeral_text(&cli_render_startup_banner(repl_style, recent_session_info))?;
    cli_attach_interactive_handles(
        runtime,
        CliInteractiveHandles {
            terminal_surface: terminal_surface.clone(),
            prompt_chrome,
            prompt_session: prompt_session.clone(),
            queued_inputs: queued_inputs.clone(),
            busy_flag: busy_flag.clone(),
            exit_requested: exit_requested.clone(),
            active_abort: active_abort.clone(),
        },
    );

    let (dispatch_tx, dispatch_rx) = mpsc::unbounded_channel::<CliDispatchInput>();
    tokio::spawn({
        let queued_inputs = queued_inputs.clone();
        let busy_flag = busy_flag.clone();
        let exit_requested = exit_requested.clone();
        let active_abort = active_abort.clone();
        let frontend_projection = shared_frontend_projection.clone();
        let terminal_surface = terminal_surface.clone();
        async move {
            while let Some(event) = prompt_event_rx.recv().await {
                match event {
                    PromptSessionEvent::Line(line) => {
                        let trimmed = line.trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if busy_flag.load(Ordering::SeqCst) {
                            if matches!(
                                parse_interactive_command(&trimmed),
                                Some(InteractiveCommand::Abort)
                            ) {
                                let handle = { active_abort.lock().await.clone() };
                                let aborted = match handle {
                                    Some(handle) => cli_trigger_abort(handle).await,
                                    None => false,
                                };
                                let _ =
                                    terminal_surface.print_block(OutputBlock::Status(if aborted {
                                        StatusBlock::warning("Abort requested for active run.")
                                    } else {
                                        StatusBlock::warning("No active run to abort.")
                                    }));
                                continue;
                            }
                            let queue_len = {
                                let mut queue = queued_inputs.lock().await;
                                queue.push_back(trimmed.clone());
                                queue.len()
                            };
                            if let Ok(mut projection) = frontend_projection.lock() {
                                projection.queue_len = queue_len;
                            }
                            let _ = terminal_surface.refresh_prompt();
                            let _ = terminal_surface.print_block(OutputBlock::QueueItem(
                                QueueItemBlock {
                                    position: queue_len,
                                    text: truncate_text(&trimmed, 72),
                                },
                            ));
                        } else if dispatch_tx.send(CliDispatchInput::Line(trimmed)).is_err() {
                            break;
                        }
                    }
                    PromptSessionEvent::Eof => {
                        if busy_flag.load(Ordering::SeqCst) {
                            exit_requested.store(true, Ordering::SeqCst);
                            let _ = terminal_surface.print_block(OutputBlock::Status(
                                StatusBlock::muted("Exit requested after current run."),
                            ));
                        } else {
                            let _ = dispatch_tx.send(CliDispatchInput::Eof);
                            break;
                        }
                    }
                    PromptSessionEvent::ModeCycle { reverse } => {
                        if busy_flag.load(Ordering::SeqCst) {
                            continue;
                        }
                        if dispatch_tx
                            .send(CliDispatchInput::ModeCycle { reverse })
                            .is_err()
                        {
                            break;
                        }
                    }
                    PromptSessionEvent::Interrupt => {}
                }
            }
        }
    });

    Ok(dispatch_rx)
}

pub(super) async fn drain_sse_events<H, F>(
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    mut handle_sse_event: H,
) where
    H: FnMut(CliServerEvent) -> F,
    F: std::future::Future<Output = ()>,
{
    while let Ok(event) = sse_rx.try_recv() {
        handle_sse_event(event).await;
    }
}
