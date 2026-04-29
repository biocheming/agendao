fn pending_command_from_session(
    session: &crate::api_client::SessionInfo,
    question_id: &str,
) -> Option<crate::api_client::PendingCommandInvocation> {
    let metadata = session.metadata.as_ref()?;
    let pending = metadata.get("pending_command_invocation")?.clone();
    let pending =
        serde_json::from_value::<crate::api_client::PendingCommandInvocation>(pending).ok()?;
    if pending
        .question_id
        .as_deref()
        .is_some_and(|candidate| candidate != question_id)
    {
        return None;
    }
    Some(pending)
}

fn shell_quote_command_value(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '*' | ':'))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn split_repeatable_answer(answer: &str) -> Vec<String> {
    answer
        .split(|ch: char| matches!(ch, '\n' | ',' | '\t'))
        .flat_map(|segment| segment.split_whitespace())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn merge_pending_command_arguments(
    pending: &crate::api_client::PendingCommandInvocation,
    answers: &[Vec<String>],
) -> String {
    let mut parts = Vec::new();
    let raw = pending.raw_arguments.trim();
    if !raw.is_empty() {
        parts.push(raw.to_string());
    }

    for (index, field) in pending.missing_fields.iter().enumerate() {
        let answer_values = answers.get(index).cloned().unwrap_or_default();
        let expanded_values = answer_values
            .into_iter()
            .flat_map(|value| {
                if value.contains('\n') || value.contains(',') {
                    split_repeatable_answer(&value)
                } else {
                    vec![value]
                }
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if expanded_values.is_empty() {
            continue;
        }
        parts.push(format!("--{}", field));
        parts.extend(
            expanded_values
                .iter()
                .map(|value| shell_quote_command_value(value)),
        );
    }

    parts.join(" ").trim().to_string()
}

fn question_defs_from_info(
    info: &crate::api_client::QuestionInfo,
) -> Vec<rocode_tool::QuestionDef> {
    if !info.items.is_empty() {
        return info
            .items
            .iter()
            .map(|item| rocode_tool::QuestionDef {
                question: item.question.clone(),
                header: item.header.clone(),
                options: item
                    .options
                    .iter()
                    .map(|option| rocode_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: item.multiple,
            })
            .collect();
    }

    info.questions
        .iter()
        .enumerate()
        .map(|(index, question)| rocode_tool::QuestionDef {
            question: question.clone(),
            header: None,
            options: info
                .options
                .as_ref()
                .and_then(|all| all.get(index))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|label| rocode_tool::QuestionOption {
                    label,
                    description: None,
                })
                .collect(),
            multiple: false,
        })
        .collect()
}

async fn resolve_prompt_submission(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    session_id: &str,
    style: &CliStyle,
    prompt_response: crate::api_client::PromptResponse,
) -> anyhow::Result<(crate::api_client::PromptResponse, std::collections::HashSet<String>)> {
    let mut response = prompt_response;
    let mut ignored_question_ids = std::collections::HashSet::new();

    loop {
        if response.status != "awaiting_user" {
            return Ok((response, ignored_question_ids));
        }

        let Some(question_id) = response.pending_question_id.clone() else {
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::error(
                    "Command is awaiting user input, but no question id was returned.",
                )),
                style,
            );
            anyhow::bail!("prompt returned awaiting_user without pending_question_id");
        };
        ignored_question_ids.insert(question_id.clone());

        let questions = api_client
            .list_questions()
            .await?
            .into_iter()
            .find(|question| question.id == question_id)
            .map(|question| question_defs_from_info(&question))
            .unwrap_or_default();
        if questions.is_empty() {
            anyhow::bail!("pending question `{}` was not available to answer", question_id);
        }

        let guard = runtime
            .spinner_guard
            .lock()
            .map(|spinner| spinner.clone())
            .unwrap_or_else(|_| SpinnerGuard::noop());
        let answers = cli_ask_question(
            questions,
            runtime.observed_topology.clone(),
            runtime.frontend_projection.clone(),
            runtime.prompt_session_slot.clone(),
            runtime.terminal_surface.clone(),
            guard,
        )
        .await
        .map_err(|error| anyhow::anyhow!("command question failed: {}", error))?;
        api_client.reply_question(&question_id, answers.clone()).await?;

        let session = api_client.get_session(session_id).await?;
        let Some(pending) = pending_command_from_session(&session, &question_id) else {
            return Ok((response, ignored_question_ids));
        };
        let arguments = merge_pending_command_arguments(&pending, &answers);
        response = api_client
            .send_command_prompt(
                session_id,
                pending.command.clone(),
                (!arguments.trim().is_empty()).then_some(arguments),
                (runtime.resolved_model_label != "auto")
                    .then(|| runtime.resolved_model_label.clone()),
                None,
            )
            .await?;
    }
}

async fn run_server_prompt(
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    input: &str,
    style: &CliStyle,
    update_recovery_base: bool,
) -> anyhow::Result<()> {
    run_server_prompt_with_parts(
        runtime,
        api_client,
        sse_rx,
        input,
        input,
        None,
        style,
        update_recovery_base,
    )
    .await
}

async fn run_server_prompt_with_parts(
    runtime: &mut CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    sse_rx: &mut mpsc::UnboundedReceiver<CliServerEvent>,
    input: &str,
    display_input: &str,
    parts: Option<Vec<crate::api_client::PromptPart>>,
    style: &CliStyle,
    update_recovery_base: bool,
) -> anyhow::Result<()> {
    if update_recovery_base {
        runtime.recovery_base_prompt = Some(display_input.to_string());
    }
    if let Ok(mut topology) = runtime.observed_topology.lock() {
        topology.reset_for_run(
            &runtime.resolved_agent_name,
            runtime.resolved_scheduler_profile_name.as_deref(),
        );
    }
    if let Ok(mut snapshots) = runtime.scheduler_stage_snapshots.lock() {
        snapshots.clear();
    }
    cli_frontend_set_phase(
        &runtime.frontend_projection,
        CliFrontendPhase::Busy,
        Some(
            runtime
                .resolved_scheduler_profile_name
                .as_deref()
                .map(|profile| format!("preset {}", profile))
                .unwrap_or_else(|| "assistant response".to_string()),
        ),
    );
    print_block(
        Some(runtime),
        OutputBlock::Message(MessageBlock::full(
            OutputMessageRole::User,
            display_input.to_string(),
        )),
        style,
    )?;

    let Some(session_id) = runtime.server_session_id.clone() else {
        anyhow::bail!("CLI server session is not initialized");
    };

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = Some(CliActiveAbortHandle::Server {
            api_client: api_client.clone(),
            session_id: session_id.clone(),
        });
    }

    let prompt_agent = cli_prompt_agent_override(
        &runtime.resolved_agent_name,
        runtime.resolved_scheduler_profile_name.as_deref(),
    );

    let prompt_response = match api_client
        .send_prompt(
            &session_id,
            input.to_string(),
            parts,
            prompt_agent,
            runtime.resolved_scheduler_profile_name.clone(),
            (runtime.resolved_model_label != "auto").then(|| runtime.resolved_model_label.clone()),
            None,
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
        cli_frontend_set_phase(
            &runtime.frontend_projection,
            CliFrontendPhase::Failed,
            Some("send prompt failed".to_string()),
        );
        let _ = print_block(
            Some(runtime),
            OutputBlock::Status(StatusBlock::error(format!(
                "Failed to send prompt: {}",
                error
            ))),
            style,
        );
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = None;
        cli_frontend_clear(runtime);
        return Ok(());
        }
    };

    let (_accepted_response, ignored_question_ids) =
        resolve_prompt_submission(runtime, api_client, &session_id, style, prompt_response).await?;

    loop {
        match sse_rx.recv().await {
            Some(CliServerEvent::QuestionCreated {
                request_id,
                session_id,
                questions_json,
            }) => {
                if ignored_question_ids.contains(&request_id) {
                    continue;
                }
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_question_from_sse(runtime, api_client, &request_id, &questions_json)
                        .await;
                }
            }
            Some(CliServerEvent::QuestionResolved { request_id })
                if ignored_question_ids.contains(&request_id) =>
            {
                continue;
            }
            Some(CliServerEvent::PermissionRequested {
                session_id,
                permission_id,
                info_json,
            }) => {
                if cli_tracks_related_session(runtime, &session_id) {
                    handle_permission_from_sse(runtime, api_client, &permission_id, &info_json)
                        .await;
                }
            }
            Some(CliServerEvent::ConfigUpdated) => {
                cli_handle_config_updated_from_sse(runtime, api_client).await;
            }
            Some(CliServerEvent::SessionUpdated { session_id, source }) => {
                handle_session_updated_from_sse(
                    runtime,
                    api_client,
                    &session_id,
                    source.as_deref(),
                    style,
                )
                .await;
            }
            Some(CliServerEvent::SessionIdle {
                session_id: idle_session_id,
            }) => {
                let is_current_session = runtime
                    .server_session_id
                    .as_deref()
                    .is_some_and(|current| current == idle_session_id);
                handle_sse_event(
                    runtime,
                    CliServerEvent::SessionIdle {
                        session_id: idle_session_id,
                    },
                    style,
                );
                if !is_current_session {
                    continue;
                }
                handle_session_updated_from_sse(
                    runtime,
                    api_client,
                    &session_id,
                    Some("prompt.done"),
                    style,
                )
                .await;
                if let Ok(mut topology) = runtime.observed_topology.lock() {
                    topology.finish_run(Some("Completed".to_string()));
                }
                cli_frontend_clear(runtime);
                let _ = print_block(
                    Some(runtime),
                    OutputBlock::Status(StatusBlock::success("Done.")),
                    style,
                );
                break;
            }
            Some(other) => {
                handle_sse_event(runtime, other, style);
            }
            None => break,
        }
    }

    {
        let mut active_abort = runtime.active_abort.lock().await;
        *active_abort = None;
    }
    Ok(())
}

fn cli_prompt_agent_override(
    resolved_agent_name: &str,
    resolved_scheduler_profile_name: Option<&str>,
) -> Option<String> {
    if resolved_scheduler_profile_name.is_some() {
        None
    } else {
        Some(resolved_agent_name.to_string())
    }
}

async fn cli_handle_config_updated_from_sse(
    runtime: &CliExecutionRuntime,
    _api_client: &CliApiClient,
) {
    let _ = runtime;
}

/// Handle an incoming SSE event from the server — update topology,
/// frontend projection, and render output blocks.
fn handle_sse_event(runtime: &CliExecutionRuntime, event: CliServerEvent, style: &CliStyle) {
    let root_session_id = runtime.server_session_id.as_deref();
    let focused_session_id = cli_focused_session_id(runtime);
    let is_root_session = |event_session_id: &str| {
        root_session_id.is_none_or(|sid| event_session_id.is_empty() || sid == event_session_id)
    };
    let is_related_session =
        |event_session_id: &str| cli_tracks_related_session(runtime, event_session_id);

    match event {
        CliServerEvent::ConfigUpdated => {
            tracing::debug!("config.updated reached sync handler");
        }
        CliServerEvent::SessionUpdated { session_id, source } => {
            if !is_root_session(&session_id) {
                return;
            }
            tracing::debug!(session_id, ?source, "session updated");
        }
        CliServerEvent::SessionBusy { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            cli_frontend_set_phase(
                &runtime.frontend_projection,
                CliFrontendPhase::Busy,
                Some("server processing".to_string()),
            );
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::SessionIdle { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            cli_frontend_set_phase(&runtime.frontend_projection, CliFrontendPhase::Idle, None);
            cli_refresh_prompt(runtime);
        }
        CliServerEvent::SessionRetrying { session_id } => {
            if !is_root_session(&session_id) {
                return;
            }
            let _ = print_block(
                Some(runtime),
                OutputBlock::Status(StatusBlock::warning("Retrying…")),
                style,
            );
        }
        CliServerEvent::QuestionCreated {
            request_id,
            session_id,
            ..
        } => {
            tracing::warn!(
                request_id,
                session_id,
                "question.created reached sync handler — skipping"
            );
        }
        CliServerEvent::QuestionResolved { request_id } => {
            tracing::debug!(request_id, "question resolved");
        }
        CliServerEvent::PermissionRequested {
            session_id,
            permission_id,
            ..
        } => {
            tracing::warn!(
                session_id,
                permission_id,
                "permission.requested reached sync handler — skipping"
            );
        }
        CliServerEvent::PermissionResolved { permission_id } => {
            tracing::debug!(permission_id, "permission resolved");
        }
        CliServerEvent::ToolCallStarted {
            session_id,
            tool_call_id,
            tool_name,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.active = true;
            }
            tracing::debug!(tool_call_id, tool_name, "tool call started");
            if !is_root_session(&session_id) {
                return;
            }
        }
        CliServerEvent::ToolCallCompleted {
            session_id,
            tool_call_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(tool_call_id, "tool call completed");
        }
        CliServerEvent::ChildSessionAttached {
            parent_id,
            child_id,
        } => {
            if cli_track_child_session(runtime, &parent_id, &child_id) {
                tracing::debug!(parent_id, child_id, "tracked child session");
            }
        }
        CliServerEvent::ChildSessionDetached {
            parent_id,
            child_id,
        } => {
            if cli_untrack_child_session(runtime, &parent_id, &child_id) {
                tracing::debug!(parent_id, child_id, "untracked child session");
            }
        }
        CliServerEvent::OutputBlock {
            session_id,
            id,
            payload,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            let block_payload = payload.get("block").unwrap_or(&payload);
            let Some(block) = parse_output_block(block_payload) else {
                tracing::debug!(?id, payload = %block_payload, "failed to parse output_block");
                return;
            };
            cli_observe_terminal_stream_block(runtime, &session_id, id.as_deref(), &block);
            if matches!(block, OutputBlock::Reasoning(_))
                && !runtime.show_thinking.load(Ordering::SeqCst)
            {
                return;
            }
            if let Ok(mut topology) = runtime.observed_topology.lock() {
                topology.observe_block(&block);
            }
            if let OutputBlock::SchedulerStage(stage) = &block {
                if let Some(child_id) = stage.child_session_id.as_deref() {
                    let _ = cli_track_child_session(runtime, &session_id, child_id);
                }
            }
            cli_frontend_observe_block(&runtime.frontend_projection, &block);
            if !is_root_session(&session_id) {
                let rendered = cli_render_session_block(runtime, &session_id, &block, style);
                cli_cache_child_session_rendered(runtime, &session_id, &rendered);
                if focused_session_id.as_deref() == Some(session_id.as_str()) {
                    let _ = print_rendered(runtime.terminal_surface.as_deref(), &rendered);
                }
                return;
            }
            match &block {
                OutputBlock::SchedulerStage(stage)
                    if !cli_should_emit_scheduler_stage_block(
                        &runtime.scheduler_stage_snapshots,
                        stage,
                    ) => {}
                OutputBlock::SchedulerStage(stage)
                    if !cli_is_terminal_stage_status(stage.status.as_deref()) =>
                {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = Some(stage.as_ref().clone());
                        projection.active_collapsed = false;
                    }
                    cli_refresh_prompt(runtime);
                }
                OutputBlock::SchedulerStage(_) => {
                    if let Ok(mut projection) = runtime.frontend_projection.lock() {
                        projection.active_stage = None;
                        projection.active_collapsed = true;
                    }
                    cli_refresh_prompt(runtime);
                    let rendered = cli_render_session_block(runtime, "", &block, style);
                    cli_cache_root_session_rendered(runtime, &rendered);
                    if cli_is_root_focused(runtime) {
                        let _ = print_rendered(runtime.terminal_surface.as_deref(), &rendered);
                    }
                }
                _ => {
                    let rendered = cli_render_session_block(runtime, "", &block, style);
                    cli_cache_root_session_rendered(runtime, &rendered);
                    if cli_is_root_focused(runtime) {
                        let _ = print_rendered(runtime.terminal_surface.as_deref(), &rendered);
                    }
                }
            }
        }
        CliServerEvent::Error {
            session_id,
            error,
            message_id,
            done,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            if !is_root_session(&session_id) {
                tracing::error!(session_id, error, ?message_id, ?done, "child session error");
                return;
            }
            tracing::error!(error, ?message_id, ?done, "server error");
            let status = OutputBlock::Status(StatusBlock::error(error));
            if cli_is_root_focused(runtime) {
                let _ = print_block(Some(runtime), status, style);
            } else {
                cli_cache_root_session_block(runtime, &status, style);
            }
        }
        CliServerEvent::Usage {
            session_id,
            prompt_tokens,
            completion_tokens,
            message_id,
        } => {
            if !is_related_session(&session_id) {
                return;
            }
            tracing::debug!(prompt_tokens, completion_tokens, ?message_id, "token usage");
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.last_turn_tokens.input_tokens = prompt_tokens;
                projection.last_turn_tokens.output_tokens = completion_tokens;
                projection.token_stats.input_tokens = projection
                    .token_stats
                    .input_tokens
                    .saturating_add(prompt_tokens);
                projection.token_stats.output_tokens = projection
                    .token_stats
                    .output_tokens
                    .saturating_add(completion_tokens);
            }
            if !is_root_session(&session_id) {
                return;
            }
            if prompt_tokens > 0 || completion_tokens > 0 {
                let status = OutputBlock::Status(StatusBlock::success(format!(
                    "tokens: prompt={} completion={}",
                    prompt_tokens, completion_tokens
                )));
                if cli_is_root_focused(runtime) {
                    let _ = print_block(Some(runtime), status, style);
                } else {
                    cli_cache_root_session_block(runtime, &status, style);
                }
            }
        }
        CliServerEvent::Unknown { event, data } => {
            tracing::trace!("Ignoring unknown SSE event: {} ({})", event, data);
        }
    }
}

async fn handle_question_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    request_id: &str,
    questions_json: &serde_json::Value,
) {
    let questions: Vec<rocode_tool::QuestionDef> =
        match serde_json::from_value(questions_json.clone()) {
            Ok(questions) => questions,
            Err(error) => {
                tracing::warn!("Failed to parse questions from SSE: {}", error);
                if let Err(reject_error) = api_client.reject_question(request_id).await {
                    tracing::warn!(
                        request_id,
                        error = %reject_error,
                        "Failed to reject malformed question request"
                    );
                }
                return;
            }
        };

    if questions.is_empty() {
        tracing::debug!("Empty question list from SSE — rejecting");
        if let Err(error) = api_client.reject_question(request_id).await {
            tracing::warn!(
                request_id,
                error = %error,
                "Failed to reject empty question request"
            );
        }
        return;
    }

    let guard = runtime
        .spinner_guard
        .lock()
        .map(|spinner| spinner.clone())
        .unwrap_or_else(|_| SpinnerGuard::noop());
    let result = cli_ask_question(
        questions,
        runtime.observed_topology.clone(),
        runtime.frontend_projection.clone(),
        runtime.prompt_session_slot.clone(),
        runtime.terminal_surface.clone(),
        guard,
    )
    .await;

    match result {
        Ok(answers) => {
            if let Err(error) = api_client.reply_question(request_id, answers).await {
                tracing::error!("Failed to reply question `{}`: {}", request_id, error);
            }
        }
        Err(_) => {
            if let Err(error) = api_client.reject_question(request_id).await {
                tracing::error!("Failed to reject question `{}`: {}", request_id, error);
            }
        }
    }
}

async fn handle_permission_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    permission_id: &str,
    info_json: &serde_json::Value,
) {
    let info: crate::api_client::PermissionRequestInfo =
        match serde_json::from_value(info_json.clone()) {
            Ok(info) => info,
            Err(error) => {
                tracing::warn!(permission_id, %error, "failed to parse permission info from SSE");
                let _ = api_client
                    .reply_permission(
                        permission_id,
                        "reject",
                        Some("Invalid permission request payload".to_string()),
                    )
                    .await;
                return;
            }
        };

    let input = info.input.as_object().cloned().unwrap_or_default();
    let permission = input
        .get("permission")
        .and_then(|value| value.as_str())
        .unwrap_or(info.tool.as_str())
        .to_string();
    let patterns = input
        .get("patterns")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let metadata = input
        .get("metadata")
        .and_then(|value| value.as_object())
        .map(|map| {
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    {
        let memory = runtime.permission_memory.lock().await;
        if memory.is_granted(&permission, &patterns) {
            let _ = api_client
                .reply_permission(permission_id, "once", Some("auto-approved".to_string()))
                .await;
            return;
        }
    }

    let guard = runtime
        .spinner_guard
        .lock()
        .map(|spinner| spinner.clone())
        .unwrap_or_else(|_| SpinnerGuard::noop());
    guard.pause();

    let decision = {
        let permission = permission.clone();
        let patterns = patterns.clone();
        let metadata = metadata.clone();
        tokio::task::spawn_blocking(move || {
            let style = CliStyle::detect();
            prompt_permission(&permission, &patterns, &metadata, &style)
        })
        .await
    };

    guard.resume();

    let decision = match decision {
        Ok(Ok(decision)) => decision,
        Ok(Err(error)) => {
            tracing::error!(permission_id, %error, "permission prompt IO error");
            let _ = api_client
                .reply_permission(
                    permission_id,
                    "reject",
                    Some(format!("Permission prompt IO error: {}", error)),
                )
                .await;
            return;
        }
        Err(error) => {
            tracing::error!(permission_id, %error, "permission prompt task failed");
            let _ = api_client
                .reply_permission(
                    permission_id,
                    "reject",
                    Some(format!("Permission prompt failed: {}", error)),
                )
                .await;
            return;
        }
    };

    let (reply, message) = match decision {
        PermissionDecision::Allow => ("once", Some("approved".to_string())),
        PermissionDecision::AllowAlways => {
            let mut memory = runtime.permission_memory.lock().await;
            memory.grant_always(&permission, &patterns);
            ("always", Some("approved always".to_string()))
        }
        PermissionDecision::Deny => ("reject", Some("rejected".to_string())),
    };

    if let Err(error) = api_client
        .reply_permission(permission_id, reply, message)
        .await
    {
        tracing::error!(permission_id, %error, "failed to reply permission");
    }
}

async fn cli_refresh_server_info(
    api_client: &CliApiClient,
    projection: &Arc<Mutex<CliFrontendProjection>>,
    server_session_id: Option<&str>,
) {
    match api_client.get_all_providers().await {
        Ok(response) => {
            let mut model_catalog = std::collections::HashMap::new();
            for provider in response.all {
                for model in provider.models {
                    model_catalog.insert(
                        format!("{}/{}", provider.id, model.id),
                        CliModelCatalogEntry {
                            context_window: model.context_window,
                            cost_per_million_input: model.cost_per_million_input,
                            cost_per_million_output: model.cost_per_million_output,
                        },
                    );
                }
            }
            if let Ok(mut projection) = projection.lock() {
                projection.model_catalog = model_catalog;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh provider catalogue: {}", error);
        }
    }

    match api_client.get_mcp_status().await {
        Ok(servers) => {
            let statuses: Vec<CliMcpServerStatus> = servers.into_iter().map(Into::into).collect();
            if let Ok(mut projection) = projection.lock() {
                projection.mcp_servers = statuses;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh MCP status: {}", error);
        }
    }

    match api_client.get_lsp_servers().await {
        Ok(servers) => {
            if let Ok(mut projection) = projection.lock() {
                projection.lsp_servers = servers;
            }
        }
        Err(error) => {
            tracing::debug!("Failed to refresh LSP status: {}", error);
        }
    }

    if let Some(session_id) = server_session_id {
        match api_client.get_session_telemetry(session_id).await {
            Ok(telemetry) => {
                if let Ok(mut projection) = projection.lock() {
                    projection.session_runtime = Some(telemetry.runtime.clone());
                    projection.stage_summaries = telemetry.stages.clone();
                    projection.telemetry_topology = Some(telemetry.topology.clone());
                    projection.token_stats.sync_from_usage(
                        telemetry.usage.input_tokens,
                        telemetry.usage.output_tokens,
                        telemetry.usage.reasoning_tokens,
                        telemetry.usage.cache_read_tokens,
                        telemetry.usage.cache_miss_tokens,
                        telemetry.usage.cache_write_tokens,
                        telemetry.usage.context_tokens,
                        telemetry.usage.total_cost,
                    );
                }
            }
            Err(error) => {
                tracing::debug!("Failed to refresh session telemetry: {}", error);
            }
        }
    }
}

async fn handle_session_updated_from_sse(
    runtime: &CliExecutionRuntime,
    api_client: &Arc<CliApiClient>,
    session_id: &str,
    source: Option<&str>,
    _style: &CliStyle,
) {
    let server_session_id = match runtime.server_session_id.as_deref() {
        Some(server_session_id) if server_session_id == session_id => server_session_id,
        _ => return,
    };
    if !cli_session_update_requires_refresh(source) {
        return;
    }
    match api_client.get_session(server_session_id).await {
        Ok(session) => {
            if let Ok(mut projection) = runtime.frontend_projection.lock() {
                projection.session_title = Some(session.title);
            }
        }
        Err(error) => {
            tracing::debug!(
                "Failed to refresh session title after session.updated: {}",
                error
            );
        }
    }
    cli_refresh_server_info(
        api_client,
        &runtime.frontend_projection,
        Some(server_session_id),
    )
    .await;
}
