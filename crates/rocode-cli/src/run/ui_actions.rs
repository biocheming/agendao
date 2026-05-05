fn cli_resolve_registry_ui_action(
    registry: &CommandRegistry,
    input: &str,
) -> Option<ResolvedUiCommand> {
    registry.resolve_ui_slash_input(input)
}

fn cli_normalize_model_ref(model_ref: &str) -> String {
    let trimmed = model_ref.trim();
    let (provider, model_id) = parse_model_and_provider(Some(trimmed.to_string()));
    match (provider, model_id) {
        (Some(provider), Some(model_id)) if !provider.is_empty() && !model_id.is_empty() => {
            format!("{provider}/{model_id}")
        }
        _ => trimmed.to_string(),
    }
}

fn cli_display_protocol_id(protocol: &str) -> &str {
    if protocol.eq_ignore_ascii_case("anthropic")
        || protocol.eq_ignore_ascii_case("anthropic-messages")
        || protocol.eq_ignore_ascii_case("messages")
    {
        "Ethnopic"
    } else {
        protocol
    }
}

enum CliModelCommand {
    List,
    Refresh,
    Select(String),
    Add {
        provider_id: String,
        model_key: String,
        config: rocode_config::ModelConfig,
    },
    Edit {
        provider_id: String,
        model_key: String,
        config: rocode_config::ModelConfig,
    },
    Delete {
        provider_id: String,
        model_key: String,
    },
}

fn cli_split_provider_model_key(value: &str) -> anyhow::Result<(String, String)> {
    let trimmed = value.trim();
    let Some((provider_id, model_key)) = trimmed.split_once('/') else {
        anyhow::bail!("Expected <provider>/<model-key>.");
    };
    let provider_id = provider_id.trim();
    let model_key = model_key.trim();
    if provider_id.is_empty() || model_key.is_empty() {
        anyhow::bail!("Expected <provider>/<model-key>.");
    }
    Ok((provider_id.to_string(), model_key.to_string()))
}

fn cli_model_optional_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed == "-"
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn cli_model_optional_bool(raw: &str) -> anyhow::Result<Option<bool>> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed == "-"
    {
        return Ok(None);
    }
    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    ) {
        return Ok(Some(true));
    }
    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    ) {
        return Ok(Some(false));
    }
    anyhow::bail!("Invalid boolean value `{trimmed}`; use true/false.");
}

fn cli_apply_model_config_assignments(
    config: &mut rocode_config::ModelConfig,
    assignments: &[&str],
) -> anyhow::Result<()> {
    for assignment in assignments {
        let Some((raw_key, raw_value)) = assignment.split_once('=') else {
            anyhow::bail!("Expected key=value assignment, got `{assignment}`.");
        };
        let key = raw_key.trim().to_ascii_lowercase().replace('-', "_");
        let value = raw_value.trim();
        match key.as_str() {
            "id" | "model" => config.model = cli_model_optional_string(value),
            "name" => config.name = cli_model_optional_string(value),
            "base_url" | "baseurl" => config.base_url = cli_model_optional_string(value),
            "family" => config.family = cli_model_optional_string(value),
            "status" => config.status = cli_model_optional_string(value),
            "release_date" | "releasedate" => {
                config.release_date = cli_model_optional_string(value)
            }
            "reasoning" => config.reasoning = cli_model_optional_bool(value)?,
            "tool_call" | "tools" => config.tool_call = cli_model_optional_bool(value)?,
            "attachment" => config.attachment = cli_model_optional_bool(value)?,
            "temperature" => config.temperature = cli_model_optional_bool(value)?,
            "experimental" => config.experimental = cli_model_optional_bool(value)?,
            _ => anyhow::bail!(
                "Unknown model field `{raw_key}`. Supported fields: id, name, base_url, family, status, release_date, reasoning, tool_call, attachment, temperature, experimental."
            ),
        }
    }
    Ok(())
}

fn cli_parse_model_command(
    argument: Option<&str>,
    current_config: Option<&rocode_config::Config>,
) -> anyhow::Result<CliModelCommand> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(CliModelCommand::List);
    };
    if argument.eq_ignore_ascii_case("refresh") {
        return Ok(CliModelCommand::Refresh);
    }

    let tokens = argument.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Ok(CliModelCommand::List);
    }

    match tokens[0].to_ascii_lowercase().as_str() {
        "add" => {
            if tokens.len() < 2 {
                anyhow::bail!(
                    "Usage: /model add <provider>/<model-key> [id=...] [name=...] [base_url=...] [family=...] [reasoning=true|false] ..."
                );
            }
            let (provider_id, model_key) = cli_split_provider_model_key(tokens[1])?;
            let mut config = rocode_config::ModelConfig::default();
            cli_apply_model_config_assignments(&mut config, &tokens[2..])?;
            Ok(CliModelCommand::Add {
                provider_id,
                model_key,
                config,
            })
        }
        "edit" => {
            if tokens.len() < 3 {
                anyhow::bail!(
                    "Usage: /model edit <provider>/<model-key> key=value [key=value ...]"
                );
            }
            let (provider_id, model_key) = cli_split_provider_model_key(tokens[1])?;
            let mut config = current_config
                .and_then(|cfg| cfg.provider.as_ref())
                .and_then(|providers| providers.get(&provider_id))
                .and_then(|provider| provider.models.as_ref())
                .and_then(|models| models.get(&model_key))
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No configured model override `{provider_id}/{model_key}` found."
                    )
                })?;
            cli_apply_model_config_assignments(&mut config, &tokens[2..])?;
            Ok(CliModelCommand::Edit {
                provider_id,
                model_key,
                config,
            })
        }
        "delete" | "remove" | "rm" => {
            if tokens.len() != 2 {
                anyhow::bail!("Usage: /model delete <provider>/<model-key>");
            }
            let (provider_id, model_key) = cli_split_provider_model_key(tokens[1])?;
            Ok(CliModelCommand::Delete {
                provider_id,
                model_key,
            })
        }
        _ => Ok(CliModelCommand::Select(argument.to_string())),
    }
}

async fn cli_prompt_action_select(
    runtime: &CliExecutionRuntime,
    header: Option<&str>,
    question: &str,
    options: Vec<SelectOption>,
) -> anyhow::Result<Option<String>> {
    let prompt_session = runtime
        .prompt_session_slot
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let already_suspended = runtime
        .terminal_surface
        .as_ref()
        .map_or(false, |s| s.prompt_suspended.load(Ordering::Relaxed));
    if !already_suspended {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.suspend();
        }
    }

    {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(
            stdout,
            crossterm::cursor::Show,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        );
        let _ = stdout.flush();
    }

    let header = header.map(str::to_string);
    let question = question.to_string();
    let style = CliStyle::detect();
    let result = tokio::task::spawn_blocking(move || {
        interactive_select(&question, header.as_deref(), &options, &style)
    })
    .await
    .map_err(|error| anyhow::anyhow!("select task failed: {}", error))?;

    if let Some(prompt_session) = prompt_session.as_ref() {
        let _ = prompt_session.resume();
    }
    if let Some(surface) = runtime.terminal_surface.as_ref() {
        surface.prompt_suspended.store(false, Ordering::Relaxed);
    }

    match result {
        Ok(SelectResult::Selected(choices)) => Ok(choices.into_iter().next()),
        Ok(SelectResult::Other(text)) => Ok(Some(text)),
        Ok(SelectResult::Cancelled) => Ok(None),
        Err(error) => Err(anyhow::anyhow!("selection failed: {}", error)),
    }
}

async fn cli_prompt_action_text(
    runtime: &CliExecutionRuntime,
    header: Option<&str>,
    question: &str,
) -> anyhow::Result<Option<String>> {
    let prompt_session = runtime
        .prompt_session_slot
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let already_suspended = runtime
        .terminal_surface
        .as_ref()
        .map_or(false, |s| s.prompt_suspended.load(Ordering::Relaxed));
    if !already_suspended {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.suspend();
        }
    }

    {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(
            stdout,
            crossterm::cursor::Show,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        );
        let _ = stdout.flush();
    }

    let header = header.map(str::to_string);
    let question = question.to_string();
    let style = CliStyle::detect();
    let result =
        tokio::task::spawn_blocking(move || prompt_free_text(&question, header.as_deref(), &style))
            .await
            .map_err(|error| anyhow::anyhow!("prompt task failed: {}", error))?;

    if let Some(prompt_session) = prompt_session.as_ref() {
        let _ = prompt_session.resume();
    }
    if let Some(surface) = runtime.terminal_surface.as_ref() {
        surface.prompt_suspended.store(false, Ordering::Relaxed);
    }

    match result {
        Ok(SelectResult::Other(text)) => Ok(Some(text)),
        Ok(SelectResult::Cancelled) | Ok(SelectResult::Selected(_)) => Ok(None),
        Err(error) => Err(anyhow::anyhow!("prompt failed: {}", error)),
    }
}

async fn cli_capture_voice_prompt(
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    current_dir: &Path,
    style: &CliStyle,
) -> anyhow::Result<()> {
    let workspace_context = api_client.get_workspace_context().await.ok();
    let config = workspace_context
        .map(|context| context.config)
        .or_else(|| load_config(current_dir).ok())
        .unwrap_or_default();
    let multimodal = rocode_multimodal::MultimodalAuthority::from_config(&config);
    if !multimodal.resolved().allow_audio_input {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "Audio input is disabled by current multimodal policy.",
            )),
            style,
        );
        return Ok(());
    }
    let duration_seconds = multimodal.voice_config().duration_seconds;
    let capture_voice_config = rocode_multimodal::MultimodalAuthority::merged_voice_config(&config);

    let prompt_session = runtime
        .prompt_session_slot
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let already_suspended = runtime
        .terminal_surface
        .as_ref()
        .map_or(false, |s| s.prompt_suspended.load(Ordering::Relaxed));
    if !already_suspended {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.suspend();
        }
    }

    {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(
            stdout,
            crossterm::cursor::Show,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        );
        let _ = stdout.flush();
    }

    println!();
    println!(
        "  Recording voice input for up to {} seconds...",
        duration_seconds
    );
    println!("  Configure `multimodal.voice.record.command` / `multimodal.voice.transcribe.command` if autodetect is not enough.");
    println!();
    let _ = io::stdout().flush();

    let capture = tokio::task::spawn_blocking(move || {
        rocode_voice::capture_voice(rocode_voice::VoiceCaptureOptions {
            config: Some(capture_voice_config),
        })
    })
    .await
    .map_err(|error| anyhow::anyhow!("voice capture task failed: {}", error))?;

    let capture = match capture {
        Ok(capture) => capture,
        Err(error) => {
            if let Some(prompt_session) = prompt_session.as_ref() {
                let _ = prompt_session.resume();
            }
            if let Some(surface) = runtime.terminal_surface.as_ref() {
                surface.prompt_suspended.store(false, Ordering::Relaxed);
            }
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(format!(
                    "Voice capture failed: {}",
                    error
                ))),
                style,
            );
            return Ok(());
        }
    };

    let mut multimodal_parts = Vec::new();
    if let Some(attachment) = capture.attachment.as_ref() {
        multimodal_parts.push(multimodal.voice_part_from_data_url(
            attachment.data_url.clone(),
            attachment.filename.clone(),
            attachment.mime.clone(),
            attachment.bytes,
        ));
    }
    let parts = rocode_multimodal::SessionPartAdapter::to_session_parts(&multimodal_parts);
    let summary =
        multimodal.build_display_summary(capture.transcript.as_deref(), &multimodal_parts);
    let preflight_request = crate::api_client::MultimodalPreflightRequest {
        model: None,
        parts: Vec::new(),
        session_parts: parts.clone(),
    };

    if !preflight_request.parts.is_empty() {
        match api_client.preflight_multimodal(&preflight_request).await {
            Ok(preflight) => {
                for warning in preflight
                    .warnings
                    .iter()
                    .chain(preflight.result.warnings.iter())
                {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning(warning.clone())),
                        style,
                    );
                }
                if preflight.result.hard_block {
                    if let Some(prompt_session) = prompt_session.as_ref() {
                        let _ = prompt_session.resume();
                    }
                    if let Some(surface) = runtime.terminal_surface.as_ref() {
                        surface.prompt_suspended.store(false, Ordering::Relaxed);
                    }
                    return Ok(());
                }
            }
            Err(error) => {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(format!(
                        "Multimodal preflight unavailable: {}",
                        error
                    ))),
                    style,
                );
            }
        }
    }

    if capture.transcript.is_none() && !multimodal_parts.is_empty() {
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::warning(
                "Voice captured without transcript; sending audio attachment only.",
            )),
            style,
        );
    }

    let result = run_server_prompt_with_parts(
        runtime,
        api_client,
        sse_rx,
        capture.transcript.as_deref().unwrap_or_default(),
        if summary.compact_label.is_empty() {
            "[voice input]"
        } else {
            summary.compact_label.as_str()
        },
        (!parts.is_empty()).then_some(parts),
        style,
        true,
    )
    .await;

    if let Some(prompt_session) = prompt_session.as_ref() {
        let _ = prompt_session.resume();
    }
    if let Some(surface) = runtime.terminal_surface.as_ref() {
        surface.prompt_suspended.store(false, Ordering::Relaxed);
    }

    result
}

async fn cli_execute_ui_action(
    action_id: UiActionId,
    argument: Option<&str>,
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    provider_registry: &ProviderRegistry,
    agent_registry: &AgentRegistry,
    current_dir: &Path,
    repl_style: &CliStyle,
) -> anyhow::Result<CliUiActionOutcome> {
    match action_id {
        UiActionId::AbortExecution => {
            let handle = { runtime.active_abort.lock().await.clone() };
            let Some(handle) = handle else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(
                        "No active run to abort. Use /abort while a response is running.",
                    )),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            if cli_trigger_abort(handle).await {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning("Cancellation requested.")),
                    repl_style,
                );
            } else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::error("Failed to request cancellation.")),
                    repl_style,
                );
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::Exit => Ok(CliUiActionOutcome::Break),
        UiActionId::VoiceInput => {
            cli_capture_voice_prompt(runtime, api_client, sse_rx, current_dir, repl_style).await?;
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ShowHelp => {
            let style = CliStyle::detect();
            let rendered = render_help(&style);
            if let Some(surface) = runtime.terminal_surface.as_ref() {
                let _ = surface.print_text(&rendered);
            } else {
                print!("{}", rendered);
                let _ = io::stdout().flush();
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenRecoveryList => {
            cli_print_recovery_actions(runtime);
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::NewSession => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            cli_execute_new_session_action(runtime, api_client, repl_style).await;
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::RenameSession => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            let Some(session_id) = runtime.server_session_id.clone() else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(
                        "No active server session to rename.",
                    )),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            let Some(next_title) = cli_prompt_action_text(
                runtime,
                Some("rename session"),
                "Enter a new title for the current session:",
            )
            .await?
            else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning("Session rename cancelled.")),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            match api_client
                .update_session_title(&session_id, next_title.trim())
                .await
            {
                Ok(updated) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title(format!(
                            "Session renamed: {}",
                            updated.title
                        ))),
                        repl_style,
                    );
                    cli_refresh_server_info(
                        api_client,
                        &runtime.frontend_projection,
                        Some(&session_id),
                    )
                    .await;
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to rename session: {}",
                            error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ShareSession => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            let Some(session_id) = runtime.server_session_id.clone() else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning("No active server session to share.")),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            match api_client.share_session(&session_id).await {
                Ok(shared) => {
                    let label = if shared.url.trim().is_empty() {
                        "Session shared.".to_string()
                    } else {
                        format!("Share link: {}", shared.url)
                    };
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title(label)),
                        repl_style,
                    );
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to share session: {}",
                            error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::UnshareSession => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            let Some(session_id) = runtime.server_session_id.clone() else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(
                        "No active server session to unshare.",
                    )),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            match api_client.unshare_session(&session_id).await {
                Ok(_) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title("Session unshared.")),
                        repl_style,
                    );
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to unshare session: {}",
                            error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ForkSession => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            cli_execute_fork_session_action(runtime, api_client, repl_style).await;
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::CompactSession => {
            cli_execute_compact_session_action(
                runtime,
                api_client,
                repl_style,
                argument.as_deref(),
            )
            .await;
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ShowStatus => {
            if argument.is_some() {
                return Ok(CliUiActionOutcome::Continue);
            }
            let style = CliStyle::detect();
            let current_session_id = cli_current_observed_session_id(runtime);

            cli_refresh_server_info(
                api_client,
                &runtime.frontend_projection,
                current_session_id.as_deref(),
            )
            .await;

            let (
                phase,
                active_label,
                queue_len,
                token_stats,
                mcp_servers,
                lsp_servers,
                session_runtime,
                stage_summaries,
                telemetry_topology,
            ) = runtime
                .frontend_projection
                .lock()
                .map(|projection| {
                    (
                        match projection.phase {
                            CliFrontendPhase::Idle => "idle",
                            CliFrontendPhase::Busy => "busy",
                            CliFrontendPhase::Waiting => "waiting",
                            CliFrontendPhase::Cancelling => "cancelling",
                            CliFrontendPhase::Failed => "failed",
                        }
                        .to_string(),
                        projection.active_label.clone(),
                        projection.queue_len,
                        projection.token_stats.clone(),
                        projection.mcp_servers.clone(),
                        projection.lsp_servers.clone(),
                        projection.session_runtime.clone(),
                        projection.stage_summaries.clone(),
                        projection.telemetry_topology.clone(),
                    )
                })
                .unwrap_or_else(|_| {
                    (
                        "unknown".to_string(),
                        None,
                        0,
                        CliSessionTokenStats::default(),
                        Vec::new(),
                        Vec::new(),
                        None,
                        Vec::new(),
                        None,
                    )
                });
            let mut lines = vec![
                format!("Agent: {}", runtime.resolved_agent_name),
                format!("Model: {}", runtime.resolved_model_label),
                format!("Directory: {}", current_dir.display()),
                format!("Runtime: {}", phase),
            ];
            if let Some(ref profile) = runtime.scheduler_profile_name {
                lines.push(format!("Scheduler: {}", profile));
            }
            if let Some(active_label) = active_label.filter(|value| !value.trim().is_empty()) {
                lines.push(format!("Active: {}", active_label));
            }
            lines.push(format!("Queue: {}", queue_len));
            if let Some(runtime_snapshot) = session_runtime.as_ref() {
                lines.push(format!(
                    "Server runtime: {}",
                    runtime_snapshot.run_status.as_ref_label()
                ));
                if let Some(active_stage_id) = runtime_snapshot.active_stage_id.as_deref() {
                    let active_stage = stage_summaries
                        .iter()
                        .find(|stage| stage.stage_id == active_stage_id)
                        .map(cli_format_stage_summary_brief)
                        .unwrap_or_else(|| active_stage_id.to_string());
                    lines.push(format!("Active stage: {}", active_stage));
                }
            }
            if let Some(topology) = telemetry_topology.as_ref() {
                lines.push(format!(
                    "Topology: active {} · running {} · waiting {}",
                    topology.active_count, topology.running_count, topology.waiting_count
                ));
            }

            if token_stats.total_tokens > 0 {
                lines.push(String::new());
                lines.push(format!(
                    "Tokens: {} total",
                    format_token_count(token_stats.total_tokens)
                ));
                lines.push(format!(
                    "  Input:     {}",
                    format_token_count(token_stats.input_tokens)
                ));
                lines.push(format!(
                    "  Output:    {}",
                    format_token_count(token_stats.output_tokens)
                ));
                if token_stats.reasoning_tokens > 0 {
                    lines.push(format!(
                        "  Reasoning: {}",
                        format_token_count(token_stats.reasoning_tokens)
                    ));
                }
                if token_stats.cache_read_tokens > 0 {
                    lines.push(format!(
                        "  Cache R:   {}",
                        format_token_count(token_stats.cache_read_tokens)
                    ));
                }
                if token_stats.cache_miss_tokens > 0 {
                    lines.push(format!(
                        "  Cache M:   {}",
                        format_token_count(token_stats.cache_miss_tokens)
                    ));
                }
                if token_stats.cache_write_tokens > 0 {
                    lines.push(format!(
                        "  Cache W:   {}",
                        format_token_count(token_stats.cache_write_tokens)
                    ));
                }
                lines.push(format!("Cost: ${:.4}", token_stats.total_cost));
            }

            if !mcp_servers.is_empty() {
                lines.push(String::new());
                lines.push("MCP Servers:".to_string());
                for server in &mcp_servers {
                    let detail = if server.tools > 0 {
                        format!(" ({} tools)", server.tools)
                    } else {
                        String::new()
                    };
                    lines.push(format!("  {} [{}]{}", server.name, server.status, detail));
                    if let Some(ref err) = server.error {
                        lines.push(format!("    ↳ {}", err));
                    }
                }
            }

            if !lsp_servers.is_empty() {
                lines.push(String::new());
                lines.push("LSP Servers:".to_string());
                for server in &lsp_servers {
                    lines.push(format!("  {}", server));
                }
            }

            if let Some(ref sid) = current_session_id {
                lines.push(String::new());
                lines.push(format!("Server: {}", api_client.base_url()));
                lines.push(format!("Session: {}", sid));
            }

            let _ =
                print_cli_list_on_surface(Some(runtime), "Session Status", None, &lines, &style);
            cli_print_execution_topology(&runtime.observed_topology, Some(runtime), &style);
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenModelList => {
            let current_config = api_client.get_config().await.ok();
            let parsed_command = match cli_parse_model_command(argument, current_config.as_ref()) {
                Ok(command) => command,
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Invalid /model usage: {}",
                            error
                        ))),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                }
            };

            match parsed_command {
                CliModelCommand::Refresh => {
                    match api_client.refresh_provider_catalog().await {
                        Ok(result) => {
                            let message = result.status_message();
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(if result.error_message.is_some() {
                                    StatusBlock::error(message)
                                } else {
                                    StatusBlock::title(message)
                                }),
                                repl_style,
                            );
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to refresh model catalogue: {}",
                                    error
                                ))),
                                repl_style,
                            );
                        }
                    }
                    return Ok(CliUiActionOutcome::Continue);
                }
                CliModelCommand::Select(model_ref) => {
                    let normalized_model_ref = cli_normalize_model_ref(&model_ref);
                    let exists = match api_client.get_all_providers().await {
                        Ok(response) => response.all.into_iter().any(|provider| {
                            provider.models.into_iter().any(|model| {
                                format!("{}/{}", provider.id, model.id) == normalized_model_ref
                            })
                        }),
                        Err(_) => {
                            let mut fallback_exists = false;
                            for provider in provider_registry.list() {
                                for model in provider.models() {
                                    if format!("{}/{}", provider.id(), model.id)
                                        == normalized_model_ref
                                    {
                                        fallback_exists = true;
                                        break;
                                    }
                                }
                                if fallback_exists {
                                    break;
                                }
                            }
                            fallback_exists
                        }
                    };
                    if exists {
                        runtime.resolved_model_label = normalized_model_ref.clone();
                        if let Ok(mut projection) = runtime.frontend_projection.lock() {
                            projection.current_model_label = Some(normalized_model_ref.clone());
                        }
                        cli_save_recent_model_ref(&api_client, &normalized_model_ref).await;
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::title(format!(
                                "Model set to {}",
                                normalized_model_ref
                            ))),
                            repl_style,
                        );
                    } else {
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::warning(format!(
                                "Unknown model: {}",
                                model_ref
                            ))),
                            repl_style,
                        );
                    }
                    return Ok(CliUiActionOutcome::Continue);
                }
                CliModelCommand::Add {
                    provider_id,
                    model_key,
                    config,
                }
                | CliModelCommand::Edit {
                    provider_id,
                    model_key,
                    config,
                } => {
                    match api_client
                        .put_provider_model_config(&provider_id, &model_key, &config)
                        .await
                    {
                        Ok(_) => {
                            let label = config.model.as_deref().unwrap_or(model_key.as_str());
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::title(format!(
                                    "Saved model override {} / {} -> {}",
                                    provider_id, model_key, label
                                ))),
                                repl_style,
                            );
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to save model override {} / {}: {}",
                                    provider_id, model_key, error
                                ))),
                                repl_style,
                            );
                        }
                    }
                    return Ok(CliUiActionOutcome::Continue);
                }
                CliModelCommand::Delete {
                    provider_id,
                    model_key,
                } => {
                    match api_client
                        .delete_provider_model_config(&provider_id, &model_key)
                        .await
                    {
                        Ok(_) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::title(format!(
                                    "Removed model override {} / {}",
                                    provider_id, model_key
                                ))),
                                repl_style,
                            );
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to remove model override {} / {}: {}",
                                    provider_id, model_key, error
                                ))),
                                repl_style,
                            );
                        }
                    }
                    return Ok(CliUiActionOutcome::Continue);
                }
                CliModelCommand::List => {}
            }

            let style = CliStyle::detect();
            let mut lines = match api_client.get_all_providers().await {
                Ok(response) => response
                    .all
                    .into_iter()
                    .flat_map(|provider| {
                        let provider_id = provider.id;
                        provider
                            .models
                            .into_iter()
                            .map(move |model| format!("{}/{}", provider_id, model.id))
                    })
                    .collect::<Vec<_>>(),
                Err(_) => {
                    let mut fallback = Vec::new();
                    for p in provider_registry.list() {
                        for m in p.models() {
                            fallback.push(format!("{}/{}", p.id(), m.id));
                        }
                    }
                    fallback
                }
            };
            lines.sort();
            lines.dedup();
            let recent = api_client.get_recent_models().await.unwrap_or_default();
            if !recent.is_empty() {
                let recent_lines = recent
                    .iter()
                    .map(|entry| format!("{}/{}", entry.provider, entry.model))
                    .filter(|model_ref| lines.iter().any(|line| line == model_ref))
                    .collect::<Vec<_>>();
                if !recent_lines.is_empty() {
                    lines.retain(|line| !recent_lines.iter().any(|recent| recent == line));
                    let mut ordered = Vec::with_capacity(lines.len() + recent_lines.len() + 2);
                    ordered.push("Recent:".to_string());
                    ordered.extend(recent_lines);
                    ordered.push(String::new());
                    ordered.extend(lines);
                    lines = ordered;
                }
            }
            if let Some(config) = current_config.as_ref() {
                let configured = config
                    .provider
                    .as_ref()
                    .map(|providers| {
                        let mut items = providers
                            .iter()
                            .flat_map(|(provider_id, provider)| {
                                provider
                                    .models
                                    .as_ref()
                                    .into_iter()
                                    .flat_map(move |models| {
                                        models.iter().map(move |(model_key, model)| {
                                            let target = model
                                                .model
                                                .as_deref()
                                                .unwrap_or(model_key.as_str());
                                            let display = model.name.as_deref().unwrap_or(target);
                                            format!(
                                                "{} / {} -> {} ({})",
                                                provider_id, model_key, target, display
                                            )
                                        })
                                    })
                            })
                            .collect::<Vec<_>>();
                        items.sort();
                        items
                    })
                    .unwrap_or_default();
                if !configured.is_empty() {
                    lines.push(String::new());
                    lines.push("Configured model overrides:".to_string());
                    lines.extend(configured);
                }
            }
            let _ =
                print_cli_list_on_surface(Some(runtime), "Available Models", None, &lines, &style);
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenModeList => {
            if let Some(mode_ref) = argument.map(str::trim).filter(|value| !value.is_empty()) {
                match api_client.list_execution_modes().await {
                    Ok(modes) => {
                        let normalized = mode_ref.to_ascii_lowercase();
                        let found = modes.into_iter().find(|mode| {
                            let key = format!("{}:{}", mode.kind, mode.id).to_ascii_lowercase();
                            key == normalized
                                || mode.id.to_ascii_lowercase() == normalized
                                || mode.name.to_ascii_lowercase() == normalized
                                || format!("{}:{}", mode.kind, mode.name).to_ascii_lowercase()
                                    == normalized
                        });
                        if let Some(mode) = found {
                            runtime.scheduler_profile_name = match mode.kind.as_str() {
                                "preset" | "profile" => Some(mode.id.clone()),
                                _ => None,
                            };
                            runtime.resolved_agent_name = if mode.kind == "agent" {
                                mode.id.clone()
                            } else {
                                runtime.resolved_agent_name.clone()
                            };
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::title(format!(
                                    "Mode set to {}:{}",
                                    mode.kind, mode.id
                                ))),
                                repl_style,
                            );
                        } else {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::warning(format!(
                                    "Unknown mode: {}",
                                    mode_ref
                                ))),
                                repl_style,
                            );
                        }
                    }
                    Err(error) => {
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::error(format!(
                                "Failed to load modes: {}",
                                error
                            ))),
                            repl_style,
                        );
                    }
                }
                return Ok(CliUiActionOutcome::Continue);
            }
            let style = CliStyle::detect();
            match api_client.list_execution_modes().await {
                Ok(modes) => {
                    let lines = modes
                        .into_iter()
                        .filter(|mode| !mode.hidden.unwrap_or(false))
                        .map(|mode| {
                            let detail = mode
                                .description
                                .filter(|value| !value.trim().is_empty())
                                .unwrap_or_else(|| mode.kind.clone());
                            format!("{} [{}] — {}", mode.id, mode.kind, detail)
                        })
                        .collect::<Vec<_>>();
                    let _ = print_cli_list_on_surface(
                        Some(runtime),
                        "Available Modes",
                        None,
                        &lines,
                        &style,
                    );
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to load modes: {}",
                            error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ConnectProvider => {
            let schema = match api_client.get_provider_connect_schema().await {
                Ok(schema) => schema,
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to load provider connect schema: {}",
                            error
                        ))),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                }
            };

            let query = if let Some(raw) = argument.map(str::trim).filter(|value| !value.is_empty())
            {
                raw.to_string()
            } else {
                let Some(input) = cli_prompt_action_text(
                    runtime,
                    Some("connect provider"),
                    "Search provider or type a custom provider id:",
                )
                .await?
                else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                };
                input
            };

            let query = query.trim().to_string();
            if query.is_empty() {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            }

            let mut resolved = match api_client.resolve_provider_connect(&query).await {
                Ok(response) => response,
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to resolve provider connect query: {}",
                            error
                        ))),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                }
            };

            if resolved.matches.len() > 1 && !resolved.exact_match {
                let mut options = resolved
                    .matches
                    .iter()
                    .map(|provider| {
                        let mut detail = provider.name.clone();
                        if provider.connected {
                            detail.push_str(" · connected");
                        }
                        if let Some(protocol) = provider.protocol.as_deref() {
                            detail.push_str(&format!(" · {}", cli_display_protocol_id(protocol)));
                        }
                        if let Some(base_url) = provider.base_url.as_deref() {
                            detail.push_str(&format!(" · {}", base_url));
                        }
                        if !provider.env.is_empty() {
                            detail.push_str(&format!(" · {}", provider.env.join(", ")));
                        }
                        SelectOption {
                            label: provider.id.clone(),
                            description: Some(detail),
                        }
                    })
                    .collect::<Vec<_>>();
                options.push(SelectOption {
                    label: format!("custom:{}", query),
                    description: Some(format!("Use `{}` as a custom provider id", query)),
                });

                let Some(selected) = cli_prompt_action_select(
                    runtime,
                    Some("connect provider"),
                    "Multiple known providers matched. Select one, or choose the custom draft:",
                    options,
                )
                .await?
                else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                };

                if selected == format!("custom:{}", query) {
                    resolved.draft = resolved.custom_draft.clone();
                } else {
                    resolved = match api_client.resolve_provider_connect(&selected).await {
                        Ok(response) => response,
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to resolve provider connect query: {}",
                                    error
                                ))),
                                repl_style,
                            );
                            return Ok(CliUiActionOutcome::Continue);
                        }
                    };
                }
            }

            let mut draft = resolved.draft.clone();

            let Some(api_key) = cli_prompt_action_text(
                runtime,
                Some("connect provider"),
                &format!(
                    "Enter API key for {}{}:",
                    draft.provider_id,
                    if draft.env.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", draft.env.join(", "))
                    }
                ),
            )
            .await?
            else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            let wants_advanced = if draft.mode == crate::api_client::ProviderConnectDraftMode::Known
            {
                let options = vec![
                    SelectOption {
                        label: "Use suggested settings".to_string(),
                        description: Some("Connect with the known provider defaults".to_string()),
                    },
                    SelectOption {
                        label: "Edit advanced fields".to_string(),
                        description: Some(
                            "Adjust base URL and protocol before connecting".to_string(),
                        ),
                    },
                    SelectOption {
                        label: "Use custom provider id".to_string(),
                        description: Some(
                            "Ignore the known match and use the custom draft".to_string(),
                        ),
                    },
                ];
                let Some(choice) = cli_prompt_action_select(
                    runtime,
                    Some("connect provider"),
                    "Choose how to connect this provider:",
                    options,
                )
                .await?
                else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                };
                match choice.as_str() {
                    "Use suggested settings" => false,
                    "Edit advanced fields" => true,
                    "Use custom provider id" => {
                        draft = resolved.custom_draft.clone();
                        true
                    }
                    _ => false,
                }
            } else {
                true
            };

            let connect_result = if wants_advanced {
                let Some(base_url_input) = cli_prompt_action_text(
                    runtime,
                    Some("connect provider"),
                    &format!(
                        "Enter provider base URL{}:",
                        draft
                            .base_url
                            .as_deref()
                            .map(|value| format!(" [{} leave empty to keep]", value))
                            .unwrap_or_default()
                    ),
                )
                .await?
                else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                };

                let base_url = if base_url_input.trim().is_empty() {
                    draft.base_url.clone().unwrap_or_default()
                } else {
                    base_url_input.trim().to_string()
                };
                if base_url.trim().is_empty() {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(
                            "Provider base URL is required for advanced connect.".to_string(),
                        )),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                }

                let mut protocol_options = schema
                    .protocols
                    .iter()
                    .map(|protocol| SelectOption {
                        label: protocol.id.clone(),
                        description: Some(cli_display_protocol_id(&protocol.name).to_string()),
                    })
                    .collect::<Vec<_>>();
                if let Some(current_protocol) = draft.protocol.as_deref() {
                    protocol_options.sort_by_key(|option| {
                        if option.label == current_protocol {
                            0
                        } else {
                            1
                        }
                    });
                }
                let Some(protocol) = cli_prompt_action_select(
                    runtime,
                    Some("connect provider"),
                    "Select the upstream protocol family:",
                    protocol_options,
                )
                .await?
                else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning("Provider connect cancelled.")),
                        repl_style,
                    );
                    return Ok(CliUiActionOutcome::Continue);
                };

                api_client
                    .connect_provider(
                        &draft.provider_id,
                        api_key.trim(),
                        Some(base_url),
                        Some(protocol.trim().to_string()),
                    )
                    .await
            } else {
                api_client
                    .set_auth(&draft.provider_id, api_key.trim())
                    .await
            };

            match connect_result {
                Ok(()) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title(format!(
                            "Connected to {}",
                            draft.provider_id
                        ))),
                        repl_style,
                    );
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to connect provider `{}`: {}",
                            draft.provider_id, error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenThemeList => {
            if argument.is_some() {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(
                        "Theme switching is not yet supported in CLI mode.",
                    )),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            }
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::warning(
                    "Theme switching is not yet supported in CLI mode.",
                )),
                repl_style,
            );
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenAgentList => {
            if let Some(agent_name) = argument.map(str::trim).filter(|value| !value.is_empty()) {
                let agents = agent_registry.list();
                let found = agents.iter().find(|info| info.name == agent_name);
                if let Some(info) = found {
                    runtime.resolved_agent_name = info.name.clone();
                    runtime.scheduler_profile_name = None;
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title(format!(
                            "Agent set to {}",
                            info.name
                        ))),
                        repl_style,
                    );
                } else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning(format!(
                            "Unknown agent: {}",
                            agent_name
                        ))),
                        repl_style,
                    );
                }
                return Ok(CliUiActionOutcome::Continue);
            }
            let style = CliStyle::detect();
            let mut lines = Vec::new();
            for info in agent_registry.list() {
                let active = if info.name == runtime.resolved_agent_name {
                    " ← active".to_string()
                } else {
                    String::new()
                };
                let model_info = info
                    .model
                    .as_ref()
                    .map(|m| format!(" ({}/{})", m.provider_id, m.model_id))
                    .unwrap_or_default();
                lines.push(format!("{}{}{}", info.name, model_info, active));
            }
            let _ =
                print_cli_list_on_surface(Some(runtime), "Available Agents", None, &lines, &style);
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenPresetList => {
            if let Some(preset_name) = argument.map(str::trim).filter(|value| !value.is_empty()) {
                let presets = cli_available_presets(&load_config(current_dir)?);
                if presets.iter().any(|preset| preset == preset_name) {
                    runtime.scheduler_profile_name = Some(preset_name.to_string());
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::title(format!(
                            "Preset set to {}",
                            preset_name
                        ))),
                        repl_style,
                    );
                } else {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning(format!(
                            "Unknown preset: {}",
                            preset_name
                        ))),
                        repl_style,
                    );
                }
                return Ok(CliUiActionOutcome::Continue);
            }
            cli_list_presets(
                &load_config(current_dir)?,
                runtime.scheduler_profile_name.as_deref(),
                Some(runtime),
            );
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::OpenSessionList => {
            if let Some(target) = argument.map(str::trim).filter(|value| !value.is_empty()) {
                match target {
                    "list" => {}
                    "new" => {
                        cli_execute_new_session_action(runtime, api_client, repl_style).await;
                        return Ok(CliUiActionOutcome::Continue);
                    }
                    "fork" => {
                        cli_execute_fork_session_action(runtime, api_client, repl_style).await;
                        return Ok(CliUiActionOutcome::Continue);
                    }
                    "compact" => {
                        cli_execute_compact_session_action(runtime, api_client, repl_style, None)
                            .await;
                        return Ok(CliUiActionOutcome::Continue);
                    }
                    _ => match api_client.list_sessions(Some(target), Some(20)).await {
                        Ok(sessions) => {
                            if let Some(session) = sessions.into_iter().find(|session| {
                                session.id == target
                                    || session.id.starts_with(target)
                                    || session
                                        .title
                                        .to_ascii_lowercase()
                                        .contains(&target.to_ascii_lowercase())
                            }) {
                                cli_set_root_server_session(runtime, session.id.clone());
                                let _ = print_block(
                                    Some(runtime),
                                    OutputBlock::Status(StatusBlock::title(format!(
                                        "Session switched: {}",
                                        session.id
                                    ))),
                                    repl_style,
                                );
                                cli_refresh_server_info(
                                    api_client,
                                    &runtime.frontend_projection,
                                    Some(&session.id),
                                )
                                .await;
                                return Ok(CliUiActionOutcome::Continue);
                            }
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::warning(format!(
                                    "Session not found: {}",
                                    target
                                ))),
                                repl_style,
                            );
                            return Ok(CliUiActionOutcome::Continue);
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to load sessions: {}",
                                    error
                                ))),
                                repl_style,
                            );
                            return Ok(CliUiActionOutcome::Continue);
                        }
                    },
                }
            }
            cli_list_sessions(Some(runtime)).await;
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::NavigateParentSession => {
            let Some(current_session_id) = runtime.server_session_id.clone() else {
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::warning(
                        "No active server session to navigate from.",
                    )),
                    repl_style,
                );
                return Ok(CliUiActionOutcome::Continue);
            };

            match api_client.get_session(&current_session_id).await {
                Ok(session) => {
                    let Some(parent_id) = session.parent_id else {
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::warning(
                                "Current session has no parent.",
                            )),
                            repl_style,
                        );
                        return Ok(CliUiActionOutcome::Continue);
                    };

                    match api_client.get_session(&parent_id).await {
                        Ok(parent) => {
                            cli_set_root_server_session(runtime, parent.id.clone());
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::title(format!(
                                    "Switched to parent session: {}",
                                    &parent.id[..parent.id.len().min(8)]
                                ))),
                                repl_style,
                            );
                            cli_refresh_server_info(
                                api_client,
                                &runtime.frontend_projection,
                                Some(&parent.id),
                            )
                            .await;
                        }
                        Err(error) => {
                            let _ = print_block(
                                Some(runtime),
                                OutputBlock::Status(StatusBlock::error(format!(
                                    "Failed to load parent session: {}",
                                    error
                                ))),
                                repl_style,
                            );
                        }
                    }
                }
                Err(error) => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::error(format!(
                            "Failed to load current session: {}",
                            error
                        ))),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ListTasks => {
            cli_list_tasks(Some(runtime));
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::CopySession => {
            match cli_copy_target_transcript(runtime).filter(|text| !text.trim().is_empty()) {
                Some(text) => match Clipboard::write_text(&text) {
                    Ok(()) => {
                        let label = if cli_focused_session_id(runtime).is_some() {
                            "Focused session transcript copied to clipboard."
                        } else {
                            "Session transcript copied to clipboard."
                        };
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::title(label)),
                            repl_style,
                        );
                    }
                    Err(error) => {
                        let _ = print_block(
                            Some(runtime),
                            OutputBlock::Status(StatusBlock::error(format!(
                                "Failed to copy transcript to clipboard: {}",
                                error
                            ))),
                            repl_style,
                        );
                    }
                },
                None => {
                    let _ = print_block(
                        Some(runtime),
                        OutputBlock::Status(StatusBlock::warning(
                            "No transcript available for the current session view.",
                        )),
                        repl_style,
                    );
                }
            }
            Ok(CliUiActionOutcome::Continue)
        }
        UiActionId::ToggleSidebar => {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::warning(
                    "CLI mode no longer keeps a persistent sidebar; use terminal scrollback and /status.",
                )),
                repl_style,
            );
            Ok(CliUiActionOutcome::Continue)
        }
        _ => Ok(CliUiActionOutcome::Continue),
    }
}
