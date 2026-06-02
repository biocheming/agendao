use super::*;
use rocode_command::cli_prompt::{read_inline_prompt_line, PromptHistory, PromptResult};

pub(super) async fn run_chat_session(
    model: Option<String>,
    provider: Option<String>,
    requested_agent: Option<String>,
    requested_scheduler_profile: Option<String>,
    thinking_requested: bool,
    interactive_mode: InteractiveCliMode,
    port_override: Option<u16>,
    working_dir: PathBuf,
    runtime_context: &FrontendRuntimeContext,
    local: bool,
    unix_socket: Option<String>,
) -> anyhow::Result<()> {
    let super::bootstrap_shared::InteractiveSessionBootstrap {
        working_dir,
        config,
        command_registry,
        provider_registry,
        agent_registry: agent_registry_arc,
        api_client,
        recent_session_info,
        selection: _selection,
        mut runtime,
        repl_style,
        server_url,
        server_session_id,
        local_state,
        transport,
    } = super::bootstrap_shared::bootstrap_interactive_session(
        model,
        provider,
        requested_agent,
        requested_scheduler_profile,
        thinking_requested,
        port_override,
        working_dir,
        runtime_context,
        local,
        unix_socket.clone(),
    )
    .await?;

    tracing::info!(
        server_url = %server_url,
        session_id = %server_session_id,
        mode = ?interactive_mode,
        "CLI connected to server and created session"
    );

    let mut dispatch_rx = match interactive_mode {
        InteractiveCliMode::Rich => {
            let server_models =
                super::prompt_shared::fetch_server_model_list(&api_client, &local_state, &transport)
                    .await;
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

    let super::stream_shared::InteractiveSessionStream {
        mut sse_rx,
        sse_cancel,
    } = super::stream_shared::bootstrap_interactive_stream(
        &server_url,
        &server_session_id,
        &api_client,
        &runtime,
        local,
        &local_state,
        &transport,
        unix_socket.clone(),
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
                    match super::compact_event_loop::wait_for_rich_input(
                        &mut runtime,
                        &config,
                        agent_registry_arc.as_ref(),
                        &api_client,
                        &local_state,
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
                    super::compact_event_loop::drain_available_events(
                        &runtime,
                        &api_client,
                        &local_state,
                        &mut sse_rx,
                        &repl_style,
                    )
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
                &local_state,
                &transport,
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
                        &local_state,
                        &transport,
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
                            &local_state,
                            &transport,
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
                    InteractiveCommand::ListAttachedSessions => {
                        cli_list_attached_sessions(&runtime);
                    }
                    InteractiveCommand::FocusAttachedSession(session_id) => {
                        match cli_focus_attached_session(&runtime, &session_id) {
                            Ok(true) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session: {}",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(false) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::warning(format!(
                                        "Unknown attached session: {}. Use /attached list first.",
                                        session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to focus attached session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusNextAttachedSession => {
                        match cli_cycle_attached_session(&runtime, true) {
                            Ok(Some((session_id, index, total))) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No attached sessions available. Use /attached list to inspect the cache.",
                            )),
                            &repl_style,
                        );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to next attached session: {}",
                                        error
                                    ))),
                                    &repl_style,
                                );
                            }
                        }
                    }
                    InteractiveCommand::FocusPreviousAttachedSession => {
                        match cli_cycle_attached_session(&runtime, false) {
                            Ok(Some((session_id, index, total))) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Focused attached session [{}/{}]: {}",
                                        index, total, session_id
                                    ))),
                                    &repl_style,
                                );
                            }
                            Ok(None) => {
                                let _ = print_block(
                            Some(&runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "No attached sessions available. Use /attached list to inspect the cache.",
                            )),
                            &repl_style,
                        );
                            }
                            Err(error) => {
                                let _ = print_block(
                                    Some(&runtime),
                                    OutputBlock::Status(StatusBlock::error(format!(
                                        "Failed to switch to previous attached session: {}",
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
                    InteractiveCommand::ShowConfigValidation => {
                        cli_print_config_validation(&runtime, &api_client, &repl_style).await;
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
            &local_state,
            &transport,
            &trimmed,
            &repl_style,
            true,
        )
        .await?;

        super::compact_event_loop::drain_available_events(
            &runtime,
            &api_client,
            &local_state,
            &mut sse_rx,
            &repl_style,
        )
        .await;

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
