use super::*;

pub(super) fn format_theme_option_label(theme_id: &str) -> String {
    if let Some((base, variant)) = split_theme_variant(theme_id) {
        return format!("{base} ({variant})");
    }
    theme_id.to_string()
}

fn split_theme_variant(theme_id: &str) -> Option<(&str, &str)> {
    let (base, variant) = theme_id
        .rsplit_once('@')
        .or_else(|| theme_id.rsplit_once(':'))?;
    if base.is_empty() || !matches!(variant, "dark" | "light") {
        return None;
    }
    Some((base, variant))
}

pub(super) fn status_line_from_block(block: StatusBlock) -> StatusLine {
    match block.tone {
        BlockTone::Title => StatusLine::title(block.text),
        BlockTone::Normal => StatusLine::normal(block.text),
        BlockTone::Muted => StatusLine::muted(block.text),
        BlockTone::Success => StatusLine::success(block.text),
        BlockTone::Warning => StatusLine::warning(block.text),
        BlockTone::Error => StatusLine::error(block.text),
    }
}

pub(super) fn append_execution_status_node(
    blocks: &mut Vec<StatusBlock>,
    node: &SessionExecutionNode,
    prefix: &str,
    is_last: bool,
) {
    let branch = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };
    let status = match node.status {
        ApiExecutionStatus::Running => "running",
        ApiExecutionStatus::Waiting => "waiting",
        ApiExecutionStatus::Cancelling => "cancelling",
        ApiExecutionStatus::Retry => "retry",
        ApiExecutionStatus::Done => "done",
    };
    let mut line = format!(
        "{}{}{} · {}",
        prefix,
        branch,
        node.label.as_deref().unwrap_or(node.id.as_str()),
        status
    );
    if let Some(waiting_on) = node.waiting_on.as_deref() {
        line.push_str(&format!(" · waiting {}", waiting_on));
    }
    if let Some(recent_event) = node.recent_event.as_deref() {
        line.push_str(&format!(" · {}", recent_event));
    }
    blocks.push(match node.status {
        ApiExecutionStatus::Running => StatusBlock::success(line),
        ApiExecutionStatus::Waiting => StatusBlock::warning(line),
        ApiExecutionStatus::Done => StatusBlock::muted(line),
        ApiExecutionStatus::Cancelling | ApiExecutionStatus::Retry => StatusBlock::error(line),
    });

    let child_prefix = if prefix.is_empty() {
        if is_last {
            "   ".to_string()
        } else {
            "│  ".to_string()
        }
    } else if is_last {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    };

    for (index, child) in node.children.iter().enumerate() {
        append_execution_status_node(
            blocks,
            child,
            &child_prefix,
            index + 1 == node.children.len(),
        );
    }
}

pub(super) fn recovery_status_blocks_from_protocol(
    recovery: &SessionRecoveryProtocol,
) -> Vec<StatusBlock> {
    let status = match recovery.status {
        ApiRecoveryProtocolStatus::Running => "running",
        ApiRecoveryProtocolStatus::AwaitingUser => "awaiting_user",
        ApiRecoveryProtocolStatus::Recoverable => "recoverable",
        ApiRecoveryProtocolStatus::Idle => "idle",
    };
    let mut blocks = vec![StatusBlock::title(format!(
        "Recovery Protocol ({status}, actions: {}, checkpoints: {})",
        recovery.actions.len(),
        recovery.checkpoints.len()
    ))];

    if let Some(summary) = recovery.summary.as_deref() {
        blocks.push(match recovery.status {
            ApiRecoveryProtocolStatus::Recoverable => {
                StatusBlock::success(format!("- {}", summary))
            }
            ApiRecoveryProtocolStatus::Running | ApiRecoveryProtocolStatus::AwaitingUser => {
                StatusBlock::warning(format!("- {}", summary))
            }
            ApiRecoveryProtocolStatus::Idle => StatusBlock::muted(format!("- {}", summary)),
        });
    }

    if let Some(prompt) = recovery.last_user_prompt.as_deref() {
        blocks.push(StatusBlock::muted(format!("- Last prompt: {}", prompt)));
    }

    if recovery.actions.is_empty() {
        blocks.push(StatusBlock::muted("- No recovery actions available"));
    } else {
        for action in &recovery.actions {
            let mut line = format!("- {} · {}", action.label, action.description);
            if let Some(target_kind) = action.target_kind.as_deref() {
                line.push_str(&format!(" · target {}", target_kind));
            }
            blocks.push(StatusBlock::success(line));
        }
    }

    if !recovery.checkpoints.is_empty() {
        blocks.push(StatusBlock::muted("- Checkpoints:"));
        for checkpoint in recovery.checkpoints.iter().take(4) {
            let mut line = format!(
                "  - {} · {} · {}",
                checkpoint.kind, checkpoint.label, checkpoint.status
            );
            if let Some(summary) = checkpoint.summary.as_deref() {
                line.push_str(&format!(" · {}", summary));
            }
            blocks.push(StatusBlock::muted(line));
        }
        if recovery.checkpoints.len() > 4 {
            blocks.push(StatusBlock::muted(format!(
                "  - … {} more checkpoint(s)",
                recovery.checkpoints.len() - 4
            )));
        }
    }

    blocks
}

fn recovery_action_key(action: &crate::api::RecoveryActionInfo, index: usize) -> String {
    let base = match action.kind {
        ApiRecoveryActionKind::AbortRun => "abort-run",
        ApiRecoveryActionKind::AbortStage => "abort-stage",
        ApiRecoveryActionKind::Retry => "retry",
        ApiRecoveryActionKind::Resume => "resume",
        ApiRecoveryActionKind::PartialReplay => "partial-replay",
        ApiRecoveryActionKind::RestartStage => "restart-stage",
        ApiRecoveryActionKind::RestartSubtask => "restart-subtask",
    };
    action
        .target_id
        .as_ref()
        .map(|target_id| format!("{}:{}", base, target_id))
        .unwrap_or_else(|| format!("{}:{}", index + 1, base))
}

pub(super) fn recovery_action_items(recovery: &SessionRecoveryProtocol) -> Vec<RecoveryActionItem> {
    recovery
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| RecoveryActionItem {
            key: recovery_action_key(action, index),
            label: action.label.clone(),
            description: action.description.clone(),
        })
        .collect()
}

pub(super) fn resolve_recovery_action_selection<'a>(
    recovery: &'a SessionRecoveryProtocol,
    selector: &str,
) -> Option<&'a crate::api::RecoveryActionInfo> {
    let normalized = selector.trim().to_ascii_lowercase().replace('_', "-");
    if let Ok(index) = normalized.parse::<usize>() {
        return recovery.actions.get(index.saturating_sub(1));
    }

    recovery
        .actions
        .iter()
        .enumerate()
        .find_map(|(index, action)| {
            let key = recovery_action_key(action, index);
            let base = key.split(':').next().unwrap_or_default().to_string();
            if key == normalized || base == normalized {
                Some(action)
            } else {
                None
            }
        })
}

pub(super) fn parse_model_ref_selection(
    model_ref: &str,
    available_models: &HashSet<String>,
    model_variants: &HashMap<String, Vec<String>>,
) -> (String, Option<String>) {
    let trimmed = model_ref.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }
    if available_models.contains(trimmed) {
        return (trimmed.to_string(), None);
    }

    let Some((candidate_base, candidate_variant)) = trimmed.rsplit_once('/') else {
        return (trimmed.to_string(), None);
    };
    if candidate_variant.is_empty() || !available_models.contains(candidate_base) {
        return (trimmed.to_string(), None);
    }
    let Some(known_variants) = model_variants.get(candidate_base) else {
        return (trimmed.to_string(), None);
    };
    if !known_variants
        .iter()
        .any(|value| value == candidate_variant)
    {
        return (trimmed.to_string(), None);
    }
    (
        candidate_base.to_string(),
        Some(candidate_variant.to_string()),
    )
}

pub(super) fn resolve_command_execution_mode(
    context: &Arc<AppContext>,
    input: &str,
    selected: SelectedExecutionMode,
) -> SelectedExecutionMode {
    let registry = CommandRegistry::new();
    let Some((command, _)) = registry.parse(input) else {
        return selected;
    };

    let Some(profile) = command
        .scheduler_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return selected;
    };

    context.set_scheduler_profile(Some(profile.to_string()));
    SelectedExecutionMode {
        agent: None,
        scheduler_profile: Some(profile.to_string()),
        display_mode: Some(profile.to_string()),
    }
}

pub(super) fn selected_execution_mode(context: &Arc<AppContext>) -> SelectedExecutionMode {
    let scheduler_profile = context.current_scheduler_profile();
    if let Some(profile) = scheduler_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return SelectedExecutionMode {
            agent: None,
            scheduler_profile: Some(profile.to_string()),
            display_mode: Some(profile.to_string()),
        };
    }

    let agent = context.current_agent();
    if agent.trim().is_empty() {
        return SelectedExecutionMode {
            agent: None,
            scheduler_profile: Some("auto".to_string()),
            display_mode: Some("auto".to_string()),
        };
    }

    SelectedExecutionMode {
        agent: Some(agent.clone()),
        scheduler_profile: None,
        display_mode: Some(agent),
    }
}

pub(super) fn current_mode_label(context: &Arc<AppContext>) -> Option<String> {
    selected_execution_mode(context).display_mode
}

pub(super) fn apply_selected_mode(context: &Arc<AppContext>, mode: &Agent) {
    match mode.kind {
        ModeKind::Agent => context.set_agent(mode.name.clone()),
        ModeKind::Preset | ModeKind::Profile => {
            context.set_scheduler_profile(Some(mode.name.clone()))
        }
    }
}

pub(super) fn map_execution_mode_to_dialog_option(
    theme: &crate::theme::Theme,
    idx: usize,
    mode: ExecutionModeInfo,
) -> Agent {
    let kind = match mode.kind.as_str() {
        "preset" => ModeKind::Preset,
        "profile" => ModeKind::Profile,
        _ => ModeKind::Agent,
    };
    let description = match kind {
        ModeKind::Agent => mode
            .description
            .unwrap_or_else(|| "No description".to_string()),
        ModeKind::Preset => match mode.orchestrator.as_deref() {
            Some(orchestrator) => format!(
                "{} ({})",
                mode.description
                    .unwrap_or_else(|| "Built-in orchestration preset".to_string()),
                orchestrator
            ),
            None => mode
                .description
                .unwrap_or_else(|| "Built-in orchestration preset".to_string()),
        },
        ModeKind::Profile => match mode.orchestrator.as_deref() {
            Some(orchestrator) => format!(
                "{} ({})",
                mode.description
                    .unwrap_or_else(|| "External scheduler profile".to_string()),
                orchestrator
            ),
            None => mode
                .description
                .unwrap_or_else(|| "External scheduler profile".to_string()),
        },
    };

    Agent {
        name: mode.id,
        description,
        color: theme.agent_color(idx),
        kind,
        orchestrator: mode.orchestrator,
    }
}

pub(super) fn default_export_filename(title: &str, session_id: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in title.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
            '-'
        } else {
            continue;
        };
        if normalized == '-' {
            if prev_dash || slug.is_empty() {
                continue;
            }
            prev_dash = true;
            slug.push('-');
        } else {
            prev_dash = false;
            slug.push(normalized);
        }
    }
    if slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        let short_id = session_id.chars().take(8).collect::<String>();
        slug = format!("session-{}", short_id);
    }
    format!("{slug}.md")
}
