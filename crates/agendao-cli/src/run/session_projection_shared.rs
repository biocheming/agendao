#[cfg(test)]
use super::{CliStyle, SchedulerStageBlock};

pub(in crate::run) trait CliStageStatusLabel {
    fn as_ref_label(&self) -> &'static str;
}

impl CliStageStatusLabel for agendao_command::stage_protocol::StageStatus {
    fn as_ref_label(&self) -> &'static str {
        match self {
            agendao_command::stage_protocol::StageStatus::Running => "running",
            agendao_command::stage_protocol::StageStatus::Waiting => "waiting",
            agendao_command::stage_protocol::StageStatus::Done => "done",
            agendao_command::stage_protocol::StageStatus::Cancelled => "cancelled",
            agendao_command::stage_protocol::StageStatus::Cancelling => "cancelling",
            agendao_command::stage_protocol::StageStatus::Blocked => "blocked",
            agendao_command::stage_protocol::StageStatus::Retrying => "retrying",
        }
    }
}

pub(in crate::run) trait CliRunStatusLabel {
    fn as_ref_label(&self) -> &'static str;
}

impl CliRunStatusLabel for crate::api_client::SessionRunStatusKind {
    fn as_ref_label(&self) -> &'static str {
        match self {
            crate::api_client::SessionRunStatusKind::Idle => "idle",
            crate::api_client::SessionRunStatusKind::Running => "running",
            crate::api_client::SessionRunStatusKind::Compacting => "compacting",
            crate::api_client::SessionRunStatusKind::WaitingOnTool => "waiting_on_tool",
            crate::api_client::SessionRunStatusKind::WaitingOnUser => "waiting_on_user",
            crate::api_client::SessionRunStatusKind::Cancelling => "cancelling",
            crate::api_client::SessionRunStatusKind::Blocked => "blocked",
            crate::api_client::SessionRunStatusKind::Sleeping => "sleeping",
        }
    }
}

pub(in crate::run) fn cli_session_context_kind_label(
    kind: crate::api_client::SessionContextKind,
) -> &'static str {
    kind.label()
}

pub(in crate::run) fn cli_session_handoff_mode_label(
    mode: agendao_types::SessionHandoffMode,
) -> &'static str {
    match mode {
        agendao_types::SessionHandoffMode::SelfContinuity => "self continuity",
        agendao_types::SessionHandoffMode::BoundedHandoff => "bounded handoff",
        agendao_types::SessionHandoffMode::StageOutputSink => "stage output sink",
        agendao_types::SessionHandoffMode::FullHistoryFork => "full-history fork",
    }
}

pub(in crate::run) fn cli_context_closure_prefix_status_label(
    prefix: &agendao_types::SessionPrefixStabilityContract,
) -> &'static str {
    prefix.status_label()
}

pub(in crate::run) fn cli_context_closure_boundary_status_label(
    boundary: &agendao_types::SessionCompactionBoundaryContract,
) -> &'static str {
    boundary.status_label()
}

pub(in crate::run) fn cli_context_closure_cache_status_label(
    cache: &agendao_types::SessionCacheExplainabilityContract,
) -> &'static str {
    cache.status_label()
}

pub(crate) fn cli_context_closure_cache_diagnostic_label(
    contract: Option<&agendao_types::SessionContextClosureContract>,
) -> Option<String> {
    contract?.coarse_diagnostic_label()
}

pub(in crate::run) fn cli_context_closure_isolation_status_label(
    isolation: &agendao_types::SessionChildHistoryIsolationContract,
) -> &'static str {
    isolation.status_label()
}

pub(crate) fn cli_cache_evidence_status_label(
    summary: &agendao_provider::cache::CacheEvidenceSummary,
) -> Option<String> {
    if !summary.should_surface() {
        return None;
    }

    let has_cause = summary
        .primary_cause
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    Some(
        if has_cause {
            agendao_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: true,
                source: agendao_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        } else {
            agendao_types::SessionCacheExplainabilityContract {
                issue_present: true,
                explained: false,
                source: agendao_types::SessionCacheExplainabilitySource::None,
                severity: None,
                explanation: None,
            }
            .status_label()
        }
        .to_string(),
    )
}

pub(in crate::run) fn cli_context_closure_evidence_impact_label(
    severity: agendao_types::SessionCacheSeverity,
) -> &'static str {
    severity.label()
}

pub(in crate::run) fn cli_context_closure_evidence_source_label(
    source: agendao_types::SessionCacheExplainabilitySource,
) -> &'static str {
    source.label()
}

pub(in crate::run) fn cli_context_closure_evidence_detail_label(detail: &str) -> String {
    let normalized = detail.trim();
    if normalized.is_empty() {
        return "--".to_string();
    }

    if normalized.contains("boundary recorded · prefix changed") {
        return "boundary recorded · prefix changed".to_string();
    }
    if normalized.contains("prefix changed before the stable boundary") {
        return "prefix changed before the stable boundary".to_string();
    }
    if normalized.contains("tool surface changed") {
        return "tool surface changed".to_string();
    }
    if normalized.contains("request shape changed") {
        return "request shape changed".to_string();
    }
    if normalized.contains("systemHash changed: system prompt changed") {
        return "system prompt changed".to_string();
    }
    if normalized.contains("family changed: protocol family changed") {
        return "request family changed".to_string();
    }
    if normalized.contains("surface changed:") {
        let field = normalized
            .split_once(':')
            .map(|(_, suffix)| suffix.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("runtime fields");
        return format!("surface changed · {}", field);
    }

    normalized.to_string()
}

pub(in crate::run) fn cli_prompt_surface_evidence_label(fields: &[String]) -> String {
    if fields.is_empty() {
        "surface changed".to_string()
    } else {
        format!("surface {}", fields.join(", "))
    }
}

pub(in crate::run) fn cli_yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

pub(in crate::run) fn cli_optional_generated_at(ts: Option<i64>) -> String {
    ts.and_then(|value| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(value))
        .map(|value| value.with_timezone(&chrono::Local))
        .map(|value| format!(" @ {}", value.format("%Y-%m-%d %H:%M:%S")))
        .unwrap_or_default()
}

pub(in crate::run) fn cli_stage_usage_line(
    stage: &agendao_command::stage_protocol::StageSummary,
) -> String {
    let mut parts = vec![format!(
        "{} [{}]",
        stage.stage_name,
        stage.status.as_ref_label()
    )];
    if let Some(prompt_tokens) = stage.prompt_tokens {
        parts.push(format!(
            "in {}",
            crate::run::session_projection_usage::format_token_count(prompt_tokens)
        ));
    }
    if let Some(completion_tokens) = stage.completion_tokens {
        parts.push(format!(
            "out {}",
            crate::run::session_projection_usage::format_token_count(completion_tokens)
        ));
    }
    if let Some(reasoning_tokens) = stage.reasoning_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "reason {}",
            crate::run::session_projection_usage::format_token_count(reasoning_tokens)
        ));
    }
    if let Some(cache_read_tokens) = stage.cache_read_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-r {}",
            crate::run::session_projection_usage::format_token_count(cache_read_tokens)
        ));
    }
    if let Some(cache_miss_tokens) = stage.cache_miss_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-m {}",
            crate::run::session_projection_usage::format_token_count(cache_miss_tokens)
        ));
    }
    if let Some(cache_write_tokens) = stage.cache_write_tokens.filter(|value| *value > 0) {
        parts.push(format!(
            "cache-w {}",
            crate::run::session_projection_usage::format_token_count(cache_write_tokens)
        ));
    }
    if let Some(budget) = stage.skill_tree_budget {
        let truncated = if stage.skill_tree_truncated.unwrap_or(false) {
            " truncated"
        } else {
            ""
        };
        parts.push(format!(
            "budget {}{}",
            crate::run::session_projection_usage::format_token_count(budget),
            truncated
        ));
    }
    if let Some(waiting_on) = stage.waiting_on.as_deref() {
        parts.push(format!("waiting {}", waiting_on));
    }
    if let Some(retry_attempt) = stage.retry_attempt {
        parts.push(format!("retry {}", retry_attempt));
    }
    parts.join(" · ")
}

#[cfg(test)]
pub(in crate::run) fn extend_wrapped_lines(out: &mut Vec<String>, text: &str, width: usize) {
    if text.is_empty() {
        out.push(String::new());
        return;
    }
    let wrapped = agendao_command::cli_panel::wrap_display_text(text, width.max(1));
    if wrapped.is_empty() {
        out.push(String::new());
    } else {
        out.extend(wrapped);
    }
}

#[cfg(test)]
pub(in crate::run) fn cli_active_stage_context_lines(
    stage: Option<&SchedulerStageBlock>,
    style: &CliStyle,
) -> Vec<String> {
    let Some(stage) = stage else {
        return Vec::new();
    };

    let max_width = usize::from(style.width).saturating_sub(8).clamp(24, 96);
    let header = if let (Some(index), Some(total)) = (stage.stage_index, stage.stage_total) {
        format!("Stage: {} [{}/{}]", stage.title, index, total)
    } else {
        format!("Stage: {}", stage.title)
    };

    let mut summary = Vec::new();
    if let Some(step) = stage.step {
        summary.push(format!("step {step}"));
    }
    if let Some(status) = stage.status.as_deref().filter(|value| !value.is_empty()) {
        summary.push(status.to_string());
    }
    if let Some(waiting_on) = stage
        .waiting_on
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        summary.push(format!("waiting on {waiting_on}"));
    }
    summary.push(format!(
        "tokens {}/{}",
        stage
            .prompt_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string()),
        stage
            .completion_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string())
    ));

    let mut lines = vec![
        agendao_command::cli_panel::truncate_display(&header, max_width),
        agendao_command::cli_panel::truncate_display(
            &format!("Status: {}", summary.join(" · ")),
            max_width,
        ),
    ];
    if let Some(focus) = stage.focus.as_deref().filter(|value| !value.is_empty()) {
        lines.push(agendao_command::cli_panel::truncate_display(
            &format!("Focus: {focus}"),
            max_width,
        ));
    }
    if let Some(last_event) = stage
        .last_event
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(agendao_command::cli_panel::truncate_display(
            &format!("Last: {last_event}"),
            max_width,
        ));
    }
    if let Some(ref attached_id) = stage.attached_session_id {
        lines.push(agendao_command::cli_panel::truncate_display(
            &format!("Child: {attached_id}"),
            max_width,
        ));
    }
    lines
}
