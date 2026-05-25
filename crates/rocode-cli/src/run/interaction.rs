// ── CLI interactive question handler ─────────────────────────────────

async fn cli_ask_question(
    questions: Vec<rocode_tool::QuestionDef>,
    observed_topology: Arc<Mutex<CliObservedExecutionTopology>>,
    frontend_projection: Arc<Mutex<CliFrontendProjection>>,
    prompt_session_slot: Arc<std::sync::Mutex<Option<Arc<PromptSession>>>>,
    terminal_surface: Option<Arc<CliTerminalSurface>>,
    spinner_guard: SpinnerGuard,
) -> Result<Vec<Vec<String>>, rocode_tool::ToolError> {
    spinner_guard.pause();
    let style = CliStyle::detect();
    let prompt_session = prompt_session_slot
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    let suspended_by_surface = match terminal_surface.as_ref() {
        Some(surface) => surface
            .suspend_modal_prompt()
            .map_err(|error| rocode_tool::ToolError::ExecutionError(error.to_string()))?,
        None => false,
    };
    let suspended_directly = !suspended_by_surface && prompt_session.is_some();
    if suspended_directly {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.suspend();
        }
    }

    {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(stdout, crossterm::cursor::Show);
        let _ = stdout.flush();
    }

    if let Ok(mut topology) = observed_topology.lock() {
        topology.start_question(questions.len());
    }
    let mut all_answers = Vec::with_capacity(questions.len());

    for question in &questions {
        cli_frontend_set_phase(
            &frontend_projection,
            CliFrontendPhase::Waiting,
            Some(
                question
                    .header
                    .clone()
                    .unwrap_or_else(|| "question".to_string()),
            ),
        );
        let options: Vec<SelectOption> = question
            .options
            .iter()
            .map(|option| SelectOption {
                label: option.label.clone(),
                description: option.description.clone(),
            })
            .collect();

        let question_text = question.question.clone();
        let question_header = question.header.clone();
        let question_multiple = question.multiple;
        let style_clone = style.clone();
        let result = tokio::task::spawn_blocking(move || {
            tracing::info!(
                question = %question_text,
                options_count = options.len(),
                multiple = question_multiple,
                style_color = style_clone.color,
                "CLI question: presenting selector"
            );
            if options.is_empty() {
                prompt_free_text(&question_text, question_header.as_deref(), &style_clone)
            } else if question_multiple {
                interactive_multi_select(
                    &question_text,
                    question_header.as_deref(),
                    &options,
                    &style_clone,
                )
            } else {
                interactive_select(
                    &question_text,
                    question_header.as_deref(),
                    &options,
                    &style_clone,
                )
            }
        })
        .await
        .unwrap_or_else(|error| Err(io::Error::other(format!("Selector task panicked: {}", error))));

        match result {
            Ok(SelectResult::Selected(choices)) => {
                all_answers.push(choices);
                cli_frontend_set_phase(
                    &frontend_projection,
                    CliFrontendPhase::Busy,
                    Some("assistant response".to_string()),
                );
            }
            Ok(SelectResult::Other(text)) => {
                all_answers.push(vec![text]);
                cli_frontend_set_phase(
                    &frontend_projection,
                    CliFrontendPhase::Busy,
                    Some("assistant response".to_string()),
                );
            }
            Ok(SelectResult::Cancelled) => {
                if let Ok(mut topology) = observed_topology.lock() {
                    topology.finish_question("cancelled");
                }
                cli_frontend_set_phase(
                    &frontend_projection,
                    CliFrontendPhase::Failed,
                    Some("question cancelled".to_string()),
                );
                if let Some(surface) = terminal_surface.as_ref() {
                    let _ = surface.resume_modal_prompt(suspended_by_surface);
                } else if suspended_directly {
                    if let Some(prompt_session) = prompt_session.as_ref() {
                        let _ = prompt_session.resume();
                    }
                }
                spinner_guard.resume();
                return Err(rocode_tool::ToolError::ExecutionError(
                    "User cancelled the question".to_string(),
                ));
            }
            Err(error) => {
                if let Ok(mut topology) = observed_topology.lock() {
                    topology.finish_question("failed");
                }
                cli_frontend_set_phase(
                    &frontend_projection,
                    CliFrontendPhase::Failed,
                    Some("question failed".to_string()),
                );
                if let Some(surface) = terminal_surface.as_ref() {
                    let _ = surface.resume_modal_prompt(suspended_by_surface);
                } else if suspended_directly {
                    if let Some(prompt_session) = prompt_session.as_ref() {
                        let _ = prompt_session.resume();
                    }
                }
                spinner_guard.resume();
                return Err(rocode_tool::ToolError::ExecutionError(
                    format!("Interactive prompt error: {}", error),
                ));
            }
        }
    }

    if let Ok(mut topology) = observed_topology.lock() {
        topology.finish_question("answered");
    }
    cli_frontend_set_phase(
        &frontend_projection,
        CliFrontendPhase::Busy,
        Some("assistant response".to_string()),
    );
    if let Some(surface) = terminal_surface.as_ref() {
        let _ = surface.resume_modal_prompt(suspended_by_surface);
    } else if suspended_directly {
        if let Some(prompt_session) = prompt_session.as_ref() {
            let _ = prompt_session.resume();
        }
    }
    spinner_guard.resume();
    Ok(all_answers)
}

fn prompt_free_text(
    question: &str,
    header: Option<&str>,
    style: &CliStyle,
) -> io::Result<SelectResult> {
    let mut rendered_plain_rows = Vec::new();
    if let Some(header) = header {
        println!("  {} {}", style.bold_cyan(style.bullet()), style.bold(header));
        rendered_plain_rows.push(format!("  {} {}", style.bullet(), header));
    }
    println!("  {}", question);
    rendered_plain_rows.push(format!("  {}", question));
    print!("  {} ", style.bold_cyan("›"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_string();
    rendered_plain_rows.push(format!(
        "  › {}",
        input.trim_end_matches(['\n', '\r'])
    ));
    let mut stdout = io::stdout();
    let width = crossterm::terminal::size()
        .map(|(width, _)| usize::from(width).max(1))
        .unwrap_or(80);
    let rows_to_clear = rendered_plain_rows
        .iter()
        .map(|row| {
            let visible_width = rocode_command::cli_panel::display_width(row);
            if visible_width == 0 {
                1
            } else {
                visible_width.div_ceil(width)
            }
        })
        .sum::<usize>();
    for _ in 0..rows_to_clear {
        let _ = crossterm::execute!(
            stdout,
            crossterm::cursor::MoveUp(1),
            crossterm::cursor::MoveToColumn(0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine)
        );
    }
    let _ = stdout.flush();

    if answer.is_empty() {
        Ok(SelectResult::Cancelled)
    } else {
        Ok(SelectResult::Other(answer))
    }
}
