use super::*;
use rocode_command::cli_prompt::{read_inline_prompt_line, PromptHistory, PromptResult};

pub(super) async fn run_chat_session(
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    thinking_requested: bool,
    interactive_mode: InteractiveCliMode,
    working_dir: PathBuf,
    runtime_context: &FrontendRuntimeContext,
) -> anyhow::Result<()> {
    let config = load_config(&working_dir)?;
    let command_registry = CommandRegistry::new();
    let provider_registry = Arc::new(setup_providers_for_dir(&config, &working_dir).await?);

    if provider_registry.list().is_empty() {
        eprintln!("Error: No API keys configured.");
        println!("Set one of the following environment variables:");
        eprintln!("  - ANTHROPIC_API_KEY");
        eprintln!("  - OPENAI_API_KEY");
        eprintln!("  - OPENROUTER_API_KEY");
        eprintln!("  - GOOGLE_API_KEY");
        eprintln!("  - MISTRAL_API_KEY");
        eprintln!("  - GROQ_API_KEY");
        eprintln!("  - XAI_API_KEY");
        eprintln!("  - DEEPSEEK_API_KEY");
        eprintln!("  - COHERE_API_KEY");
        eprintln!("  - TOGETHER_API_KEY");
        eprintln!("  - PERPLEXITY_API_KEY");
        eprintln!("  - CEREBRAS_API_KEY");
        eprintln!("  - DEEPINFRA_API_KEY");
        eprintln!("  - VERCEL_API_KEY");
        eprintln!("  - GITLAB_TOKEN");
        eprintln!("  - GITHUB_COPILOT_TOKEN");
        eprintln!("  - GOOGLE_VERTEX_API_KEY + GOOGLE_VERTEX_PROJECT_ID + GOOGLE_VERTEX_LOCATION");
        std::process::exit(1);
    }

    let agent_registry_arc = Arc::new(AgentRegistry::from_config(&config));
    let server_url = runtime_context
        .discover_or_start_server_with_request(crate::ServerDiscoveryRequest {
            port_override: None,
            cwd: Some(working_dir.clone()),
        })
        .await?;
    let api_client = Arc::new(CliApiClient::new(server_url.clone()));
    let server_context = api_client.get_workspace_context().await.ok();
    let recent_session_info = cli_load_recent_session_info(&api_client, &working_dir).await;

    let (carry_model, carry_provider) = recent_session_info
        .as_ref()
        .and_then(|info| info.model_label.clone())
        .map(|label| {
            let (p, m) = parse_model_and_provider(Some(label));
            (m, p)
        })
        .unwrap_or((None, None));

    let carry_preset = recent_session_info
        .as_ref()
        .and_then(|info| info.preset_label.as_deref())
        .and_then(|label| {
            if label.starts_with("agent:") {
                None
            } else {
                Some(label.to_string())
            }
        });

    let selection = CliRunSelection {
        model: model.or(carry_model),
        provider: provider.or(carry_provider),
        requested_agent,
        requested_scheduler_profile: requested_scheduler_profile.or(carry_preset),
        show_thinking: cli_resolve_show_thinking(
            thinking_requested,
            server_context.as_ref().map(|context| &context.config),
            false,
        ),
    };

    let mut runtime = build_cli_execution_runtime(CliRuntimeBuildInput {
        config: &config,
        agent_registry: agent_registry_arc.clone(),
        selection: &selection,
    })
    .await?;
    let repl_style = CliStyle::detect();

    let session_info = api_client
        .create_session(None, selection.requested_scheduler_profile.clone())
        .await?;
    let server_session_id = session_info.id.clone();
    runtime.api_client = Some(api_client.clone());
    cli_set_root_server_session(&mut runtime, server_session_id.clone());

    tracing::info!(
        server_url = %server_url,
        session_id = %server_session_id,
        mode = ?interactive_mode,
        "CLI connected to server and created session"
    );

    let mut dispatch_rx = match interactive_mode {
        InteractiveCliMode::Rich => {
            let server_models = api_client.get_all_providers().await.ok().map(|response| {
                let mut models: Vec<String> = response
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
            });
            Some(attach_rich_prompt(
                &mut runtime,
                &repl_style,
                &working_dir,
                &config,
                provider_registry.as_ref(),
                agent_registry_arc.as_ref(),
                recent_session_info.as_ref(),
                server_models,
            )?)
        }
        InteractiveCliMode::Compact => {
            print!(
                "{}{}",
                cli_render_startup_banner(&repl_style, recent_session_info.as_ref()),
                repl_style.dim(
                    "Compact interactive mode: native terminal scrollback, line-based input.\n\n"
                )
            );
            io::stdout().flush()?;
            None
        }
    };

    let (sse_tx, mut sse_rx) = mpsc::unbounded_channel::<CliServerEvent>();
    let sse_cancel = CancellationToken::new();
    let _sse_handle = event_stream::spawn_sse_subscriber(
        server_url.clone(),
        server_session_id.clone(),
        sse_tx,
        sse_cancel.clone(),
    );

    cli_refresh_server_info(
        &api_client,
        &runtime.frontend_projection,
        Some(&server_session_id),
    )
    .await;

    let mut compact_history = PromptHistory::new(200);

    loop {
        let queued = {
            let mut queue = runtime.queued_inputs.lock().await;
            let next = queue.pop_front();
            let remaining = queue.len();
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.queue_len = remaining;
            }
            next
        };

        let trimmed = match queued {
            Some(line) => line,
            None => match interactive_mode {
                InteractiveCliMode::Rich => {
                    let Some(dispatch_rx) = dispatch_rx.as_mut() else {
                        anyhow::bail!("interactive prompt receiver missing");
                    };
                    match wait_for_rich_input(
                        &runtime,
                        &api_client,
                        dispatch_rx,
                        &mut sse_rx,
                        &repl_style,
                    )
                    .await?
                    {
                        Some(line) => line,
                        None => {
                            sse_cancel.cancel();
                            return Ok(());
                        }
                    }
                }
                InteractiveCliMode::Compact => {
                    drain_available_sse_events(&runtime, &api_client, &mut sse_rx, &repl_style)
                        .await;
                    match read_compact_input(&runtime, &mut compact_history, &repl_style)? {
                        Some(line) => line,
                        None => {
                            sse_cancel.cancel();
                            return Ok(());
                        }
                    }
                }
            },
        };

        if trimmed.is_empty() {
            continue;
        }

        if let Some(resolved) = cli_resolve_registry_ui_action(&command_registry, &trimmed) {
            match cli_execute_ui_action(
                resolved.action_id,
                resolved.argument.as_deref(),
                &mut runtime,
                &api_client,
                &mut sse_rx,
                &provider_registry,
                &agent_registry_arc,
                &working_dir,
                &repl_style,
            )
            .await?
            {
                CliUiActionOutcome::Break => break,
                CliUiActionOutcome::Continue => continue,
            }
        }

        if let Some(cmd) = parse_interactive_command(&trimmed) {
            if matches!(cmd, InteractiveCommand::Unknown(_)) {
                // Forward unknown slash commands to the server-side command registry
                // so built-in/custom scheduler commands like `/autoresearch` still work.
            } else {
                if let Some(invocation) = cmd.ui_action_invocation() {
                    match cli_execute_ui_action(
                        invocation.action_id,
                        invocation.argument.as_deref(),
                        &mut runtime,
                        &api_client,
                        &mut sse_rx,
                        &provider_registry,
                        &agent_registry_arc,
                        &working_dir,
                        &repl_style,
                    )
                    .await?
                    {
                        CliUiActionOutcome::Break => break,
                        CliUiActionOutcome::Continue => continue,
                    }
                }
                match cmd {
                    InteractiveCommand::Abort => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No active run to abort. Use /abort while a response is running.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ExecuteRecovery(selector) => {
                        let Some(action) = cli_select_recovery_action(&runtime, &selector) else {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::warning(format!(
                                    "Unknown recovery action: {}",
                                    selector
                                ))),
                                &repl_style,
                            );
                            cli_print_recovery_actions(&runtime);
                            continue;
                        };
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::title(format!("↺ {}", action.label))),
                            &repl_style,
                        );
                        run_server_prompt(
                            &mut runtime,
                            &api_client,
                            &mut sse_rx,
                            &action.prompt,
                            &repl_style,
                            false,
                        )
                        .await?;
                    }
                    InteractiveCommand::ClearScreen => {
                        if let Some(surface) = runtime.terminal_surface.as_ref() {
                            let _ = surface.clear_transcript();
                        } else {
                            print!("\x1B[2J\x1B[1;1H");
                            io::stdout().flush()?;
                        }
                    }
                    InteractiveCommand::ListChildSessions => {
                        cli_list_child_sessions(&runtime);
                    }
                    InteractiveCommand::FocusChildSession(session_id) => {
                        match cli_focus_child_session(&runtime, &session_id) {
                            Ok(true) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused child session: {}",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(false) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::warning(format!(
                                        "Unknown child session: {}. Use /child list first.",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to focus child session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusNextChildSession => {
                        match cli_cycle_child_session(&runtime, true) {
                            Ok(Some((session_id, index, total))) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused child session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No child sessions available. Use /child list to inspect the cache.",
                            )),
                            &repl_style,
                        );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to next child session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusPreviousChildSession => {
                        match cli_cycle_child_session(&runtime, false) {
                            Ok(Some((session_id, index, total))) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused child session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No child sessions available. Use /child list to inspect the cache.",
                            )),
                            &repl_style,
                        );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to previous child session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::BackToRootSession => match cli_focus_root_session(&runtime)
                    {
                        Ok(true) => {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::title(
                                    "Returned to root session view.",
                                )),
                                &repl_style,
                            );
                        }
                        Ok(false) => {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::warning(
                                    "Already viewing the root session.",
                                )),
                                &repl_style,
                            );
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(&runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to restore root session view: {}",
                                    error
                                ))),
                                &repl_style,
                            );
                        }
                    },
                    InteractiveCommand::Compact(_) => {}
                    InteractiveCommand::ShowTask(id) => {
                        cli_show_task(&id, Some(&runtime));
                    }
                    InteractiveCommand::KillTask(id) => {
                        cli_kill_task(&id, Some(&runtime));
                    }
                    InteractiveCommand::ToggleActive => {
                        let _ = print_block(
                        Some(&runtime),
                        OutputBlock::Status(StatusBlock::warning(
                            "CLI mode renders stage activity inline in the transcript; no separate active panel is kept onscreen.",
                        )),
                        &repl_style,
                    );
                    }
                    InteractiveCommand::ScrollUp => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "Use your terminal's native scrollback in CLI mode.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ScrollDown => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "Use your terminal's native scrollback in CLI mode.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ScrollBottom => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "Use your terminal's native scrollback in CLI mode.",
                            )),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::ShowRuntime => {
                        cli_print_runtime_snapshot(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowUsage => {
                        cli_print_usage_snapshot(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowInsights => {
                        cli_print_session_insights(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowEvents(raw_filter) => {
                        cli_print_session_events(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_filter.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemory(search) => {
                        cli_print_memory_list(
                            &runtime,
                            &api_client,
                            &repl_style,
                            search.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryPreview(query) => {
                        cli_print_memory_retrieval_preview(
                            &runtime,
                            &api_client,
                            &repl_style,
                            query.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryDetail(record_id) => {
                        cli_print_memory_detail(&runtime, &api_client, &repl_style, &record_id)
                            .await;
                    }
                    InteractiveCommand::ShowMemoryValidation(record_id) => {
                        cli_print_memory_validation_report(
                            &runtime,
                            &api_client,
                            &repl_style,
                            &record_id,
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryConflicts(record_id) => {
                        cli_print_memory_conflicts(&runtime, &api_client, &repl_style, &record_id)
                            .await;
                    }
                    InteractiveCommand::ShowMemoryRulePacks => {
                        cli_print_memory_rule_packs(&runtime, &api_client, &repl_style).await;
                    }
                    InteractiveCommand::ShowMemoryRuleHits(raw_query) => {
                        cli_print_memory_rule_hits(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_query.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::ShowMemoryConsolidationRuns => {
                        cli_print_memory_consolidation_runs(&runtime, &api_client, &repl_style)
                            .await;
                    }
                    InteractiveCommand::RunMemoryConsolidation(raw_request) => {
                        cli_run_memory_consolidation(
                            &runtime,
                            &api_client,
                            &repl_style,
                            raw_request.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::InspectStage(stage_filter) => {
                        cli_print_session_events(
                            &runtime,
                            &api_client,
                            &repl_style,
                            stage_filter.as_deref(),
                        )
                        .await;
                    }
                    InteractiveCommand::Unknown(name) => {
                        let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(format!(
                                "Unknown command: /{}. Type /help for available commands.",
                                name
                            ))),
                            &repl_style,
                        );
                    }
                    InteractiveCommand::Exit
                    | InteractiveCommand::ShowHelp
                    | InteractiveCommand::ShowRecovery
                    | InteractiveCommand::NewSession
                    | InteractiveCommand::ShowStatus
                    | InteractiveCommand::ListModels
                    | InteractiveCommand::ListProviders
                    | InteractiveCommand::ConnectProvider(_)
                    | InteractiveCommand::ListThemes
                    | InteractiveCommand::ListPresets
                    | InteractiveCommand::ListSessions
                    | InteractiveCommand::ParentSession
                    | InteractiveCommand::ListTasks
                    | InteractiveCommand::ListAgents
                    | InteractiveCommand::Copy
                    | InteractiveCommand::ToggleSidebar
                    | InteractiveCommand::SelectModel(_)
                    | InteractiveCommand::SelectPreset(_)
                    | InteractiveCommand::SelectAgent(_) => {}
                }
                continue;
            }
        }

        runtime.busy_flag.store(true, Ordering::SeqCst);
        run_server_prompt(
            &mut runtime,
            &api_client,
            &mut sse_rx,
            &trimmed,
            &repl_style,
            true,
        )
        .await?;

        drain_available_sse_events(&runtime, &api_client, &mut sse_rx, &repl_style).await;

        runtime.busy_flag.store(false, Ordering::SeqCst);
        if let Some(surface) = runtime.terminal_surface.as_ref() {
            let _ = surface.ensure_prompt_visible();
        }
        if runtime.exit_requested.load(Ordering::SeqCst)
            && runtime.queued_inputs.lock().await.is_empty()
        {
            break;
        }
    }

    sse_cancel.cancel();
    Ok(())
}

fn attach_rich_prompt(
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
        busy_flag.clone(),
    ));
    let prompt_chrome = Arc::new(CliPromptChrome::new(
        runtime,
        repl_style,
        current_dir,
        config,
        provider_registry,
        agent_registry,
    ));
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
    terminal_surface.set_prompt_session(prompt_session.clone());
    terminal_surface.print_text(&cli_render_startup_banner(repl_style, recent_session_info))?;
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
        let prompt_session = prompt_session.clone();
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
                            let _ = prompt_session.refresh();
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
                    PromptSessionEvent::Interrupt => {}
                }
            }
        }
    });

    Ok(dispatch_rx)
}

async fn wait_for_rich_input(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    dispatch_rx: &mut mpsc::UnboundedReceiver<CliDispatchInput>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    repl_style: &CliStyle,
) -> anyhow::Result<Option<String>> {
    loop {
        tokio::select! {
            dispatch = dispatch_rx.recv() => {
                return Ok(match dispatch {
                    Some(CliDispatchInput::Line(line)) => Some(line),
                    Some(CliDispatchInput::Eof) | None => None,
                });
            }
            sse_event = sse_rx.recv() => {
                if let Some(event) = sse_event {
                    handle_interactive_sse_event(runtime, api_client, event, repl_style).await;
                }
            }
        }
    }
}

async fn drain_available_sse_events(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    repl_style: &CliStyle,
) {
    while let Ok(event) = sse_rx.try_recv() {
        handle_interactive_sse_event(runtime, api_client, event, repl_style).await;
    }
}

async fn handle_interactive_sse_event(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    event: CliServerEvent,
    repl_style: &CliStyle,
) {
    match event {
        CliServerEvent::ConfigUpdated => {
            cli_handle_config_updated_from_sse(runtime, api_client).await;
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id: _,
            questions_json,
        } => {
            handle_question_from_sse(runtime, api_client, &request_id, &questions_json).await;
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            info_json,
        } => {
            if cli_tracks_related_session(runtime, &session_id) {
                handle_permission_from_sse(runtime, api_client, &permission_id, &info_json).await;
            }
        }
        other => {
            handle_sse_event(runtime, other, repl_style);
        }
    }
}

fn read_compact_input(
    runtime: &CliExecutionRuntime,
    history: &mut PromptHistory,
    repl_style: &CliStyle,
) -> anyhow::Result<Option<String>> {
    let prompt = render_compact_prompt(runtime);
    match read_inline_prompt_line(&prompt, history, repl_style)? {
        PromptResult::Line(line) => {
            if !line.trim().is_empty() {
                history.push(&line);
            }
            Ok(Some(line.trim().to_string()))
        }
        PromptResult::Interrupt => Ok(Some(String::new())),
        PromptResult::Eof => Ok(None),
    }
}

fn render_compact_prompt(runtime: &CliExecutionRuntime) -> String {
    let _ = runtime;
    "rocode> ".to_string()
}
