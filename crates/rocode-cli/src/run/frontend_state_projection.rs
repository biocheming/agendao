use super::{format_token_count, CliFrontendProjection};
use rocode_command::cli_style::CliStyle;
use rocode_command::run_status_labels::{canonical_run_status_labels, canonical_run_status_title};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CliFrontendPhase {
    #[default]
    Idle,
    Busy,
    Waiting,
    Cancelling,
    Failed,
}

impl CliFrontendProjection {
    pub(super) fn footer_should_animate(&self) -> bool {
        if self.pending_permission_count > 0 || self.submitting_permission_count > 0 {
            return true;
        }

        if let Some(run_tail) = self.run_tail.as_ref() {
            let slug = canonical_run_status_labels(&run_tail.status).slug;
            if matches!(
                slug,
                "running"
                    | "awaiting_permission"
                    | "awaiting_user"
                    | "cancelling"
                    | "retrying"
                    | "reconnecting"
                    | "compacting"
            ) {
                return true;
            }
        }

        matches!(
            self.phase,
            CliFrontendPhase::Busy | CliFrontendPhase::Waiting | CliFrontendPhase::Cancelling
        )
    }

    fn is_ready_state(&self) -> bool {
        matches!(self.phase, CliFrontendPhase::Idle)
            && self.run_tail.is_none()
            && self.queue_len == 0
            && self.pending_permission_count == 0
            && self.submitting_permission_count == 0
            && self.active_stage.is_none()
            && self
                .active_label
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && self.token_stats.total_tokens == 0
            && self.token_stats.input_tokens == 0
            && self.token_stats.output_tokens == 0
            && self.token_stats.reasoning_tokens == 0
            && self.transcript.rendered_text().trim().is_empty()
    }

    pub(super) fn current_context_tokens(&self) -> Option<u64> {
        let usage_context_tokens =
            (self.token_stats.context_tokens > 0).then_some(self.token_stats.context_tokens);
        let active_stage_id = self
            .session_runtime
            .as_ref()
            .and_then(|runtime| runtime.active_stage_id.as_deref());
        let active_stage_context_tokens = active_stage_id.and_then(|active_stage_id| {
            self.stage_summaries
                .iter()
                .find(|stage| stage.stage_id == active_stage_id)
                .and_then(|stage| stage.estimated_context_tokens)
        });

        rocode_types::current_context_tokens_from_sources(
            usage_context_tokens,
            active_stage_context_tokens,
        )
    }

    pub(super) fn footer_text(&self) -> String {
        let suppress_runtime_summary = self.pending_permission_count > 0
            || self.submitting_permission_count > 0
            || self.run_tail.is_some()
            || matches!(
                self.phase,
                CliFrontendPhase::Busy | CliFrontendPhase::Waiting | CliFrontendPhase::Cancelling
            );
        let state = if self.pending_permission_count > 0 || self.submitting_permission_count > 0 {
            cli_footer_state_label("awaiting_permission")
        } else if matches!(self.phase, CliFrontendPhase::Waiting) {
            cli_footer_state_label("awaiting_user")
        } else if matches!(self.phase, CliFrontendPhase::Cancelling) {
            cli_footer_state_label("cancelling")
        } else if let Some(run_tail) = self.run_tail.as_ref() {
            cli_footer_state_label(&run_tail.status)
        } else {
            match self.phase {
                CliFrontendPhase::Idle if self.is_ready_state() => cli_footer_state_label("ready"),
                CliFrontendPhase::Idle => cli_footer_state_label("idle"),
                CliFrontendPhase::Busy => cli_footer_state_label("running"),
                CliFrontendPhase::Waiting => cli_footer_state_label("awaiting_user"),
                CliFrontendPhase::Cancelling => cli_footer_state_label("cancelling"),
                CliFrontendPhase::Failed => cli_footer_state_label("error"),
            }
        };
        let mut parts = Vec::new();
        if !suppress_runtime_summary {
            parts.push(format!(" {} ", state));
            if let Some(elapsed_seconds) = self.activity_elapsed_seconds() {
                parts.push(cli_footer_meta_part(&cli_footer_elapsed_label(
                    elapsed_seconds,
                )));
            }
            if let Some(detail) = self
                .run_tail
                .as_ref()
                .and_then(|run_tail| run_tail.detail.as_deref())
                .filter(|value| !value.trim().is_empty())
            {
                parts.push(cli_footer_detail_part(detail));
            }
            if let Some(active) = self
                .active_label
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                parts.push(cli_footer_activity_part(active));
            }
        }
        if let Some(view) = self.view_label.as_deref().filter(|value| !value.is_empty()) {
            parts.push(cli_footer_activity_part(view));
        }
        if self.queue_len > 0 {
            parts.push(cli_footer_meta_part(&format!("queue {}", self.queue_len)));
        }
        if self.pending_permission_count > 0 || self.submitting_permission_count > 0 {
            parts.push(cli_footer_warning_part(&format!(
                "perm p/s {}/{}",
                self.pending_permission_count, self.submitting_permission_count
            )));
        }
        if let Some(error) = self.last_permission_submit_error.as_deref() {
            parts.push(cli_footer_error_part(&format!("perm-error {}", error)));
        }
        if let Some(current_tokens) = self.current_context_tokens() {
            let context_window = self
                .current_model_label
                .as_deref()
                .and_then(|label| self.model_catalog.get(label))
                .and_then(|entry| entry.context_window)
                .filter(|value| *value > 0);
            if let Some(limit) = context_window {
                let percent =
                    (((current_tokens as f64 / limit as f64) * 100.0).round() as u64).max(1);
                let mut label = format!(
                    "ctx {}/{} {}%",
                    format_token_count(current_tokens),
                    format_token_count(limit),
                    percent
                );
                if let Some(note) = rocode_types::context_pressure_label(Some(percent)) {
                    label.push(' ');
                    label.push_str(note);
                }
                parts.push(cli_footer_usage_part(&label));
            } else {
                parts.push(cli_footer_usage_part(&format!(
                    "ctx {}",
                    format_token_count(current_tokens)
                )));
            }
        } else if self.token_stats.total_tokens > 0 {
            parts.push(cli_footer_usage_part(&format!(
                "workflow {}",
                format_token_count(self.token_stats.total_tokens)
            )));
        }
        if self.token_stats.cache_read_tokens > 0
            || self.token_stats.cache_miss_tokens > 0
            || self.token_stats.cache_write_tokens > 0
        {
            parts.push(cli_footer_cache_part(&format!(
                "cache H/M/W {}/{}/{}",
                format_token_count(self.token_stats.cache_read_tokens),
                format_token_count(self.token_stats.cache_miss_tokens),
                format_token_count(self.token_stats.cache_write_tokens)
            )));
        }
        if let Some(cache_diagnostic) = self.cache_diagnostic.as_deref() {
            parts.push(cli_footer_cache_part(&format!(
                "cache {}",
                cache_diagnostic
            )));
        }
        if let Some(ingress_diagnostic) = self.ingress_diagnostic.as_deref() {
            parts.push(cli_footer_meta_part(&format!(
                "ingress {}",
                ingress_diagnostic
            )));
        }
        if let Some(provider_diagnostic) = self.provider_diagnostic.as_deref() {
            parts.push(cli_footer_meta_part(&format!(
                "provider {}",
                provider_diagnostic
            )));
        }
        if let Some(browser) = self.events_browser.as_ref() {
            let page = (browser.offset / browser.filter.limit.unwrap_or(24).max(1)) + 1;
            parts.push(cli_footer_meta_part(&format!("events p{}", page)));
        }
        parts.push(cli_footer_hint_part("Alt+Enter/Ctrl+J newline"));
        parts.push(cli_footer_hint_part("/help"));
        parts.push(cli_footer_hint_part("/runtime"));
        parts.push(cli_footer_hint_part("/insights"));
        parts.push(cli_footer_hint_part("/validation"));
        parts.push(cli_footer_hint_part("/attached"));
        if !matches!(self.phase, CliFrontendPhase::Idle) {
            parts.push(cli_footer_hint_part("/abort"));
        }
        parts.push(cli_footer_hint_part("Ctrl+D exit"));
        format!(" {} ", parts.join("  •  "))
    }
}

fn cli_footer_state_label(status: &str) -> String {
    let style = CliStyle::detect();
    let title = canonical_run_status_title(status);
    let spinner = cli_footer_spinner_frame();
    match canonical_run_status_labels(status).slug {
        "running" => format!("{} {}", style.bold_cyan(spinner), style.bold_cyan(title)),
        "awaiting_permission" => {
            format!(
                "{} {}",
                style.bold_yellow(spinner),
                style.bold_yellow(title)
            )
        }
        "awaiting_user" => format!("{} {}", style.bold_yellow("…"), style.bold_yellow(title)),
        "complete" => format!(
            "{} {}",
            style.bold_green(style.check()),
            style.bold_green(title)
        ),
        "error" => format!(
            "{} {}",
            style.bold_red(style.cross()),
            style.bold_red(title)
        ),
        "idle" | "ready" => format!("{} {}", style.dim("◇"), style.bold(title)),
        "cancelling" | "retrying" | "reconnecting" | "compacting" => {
            format!("{} {}", style.bold_cyan(spinner), style.bold_cyan(title))
        }
        _ => style.bold(title),
    }
}

fn cli_footer_spinner_frame() -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let elapsed_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as usize)
        .unwrap_or(0);
    FRAMES[(elapsed_ms / 80) % FRAMES.len()]
}

fn cli_footer_elapsed_label(elapsed_seconds: u64) -> String {
    if elapsed_seconds < 60 {
        return format!("{elapsed_seconds}s");
    }
    if elapsed_seconds < 3600 {
        return format!("{}m {:02}s", elapsed_seconds / 60, elapsed_seconds % 60);
    }
    format!(
        "{}h {:02}m {:02}s",
        elapsed_seconds / 3600,
        (elapsed_seconds % 3600) / 60,
        elapsed_seconds % 60
    )
}

fn cli_footer_detail_part(text: &str) -> String {
    CliStyle::detect().dim(text)
}

fn cli_footer_activity_part(text: &str) -> String {
    CliStyle::detect().bold_rgb(text, 120, 210, 255)
}

fn cli_footer_usage_part(text: &str) -> String {
    CliStyle::detect().cyan(text)
}

fn cli_footer_cache_part(text: &str) -> String {
    CliStyle::detect().rgb(text, 180, 160, 255)
}

fn cli_footer_warning_part(text: &str) -> String {
    CliStyle::detect().yellow(text)
}

fn cli_footer_error_part(text: &str) -> String {
    CliStyle::detect().red(text)
}

fn cli_footer_meta_part(text: &str) -> String {
    CliStyle::detect().dim(text)
}

fn cli_footer_hint_part(text: &str) -> String {
    CliStyle::detect().rgb(text, 140, 150, 170)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::frontend_state_types::CliRunTailState;

    #[test]
    fn footer_text_surfaces_permission_interaction_state() {
        let mut projection = CliFrontendProjection::default();
        projection.pending_permission_count = 1;
        projection.submitting_permission_count = 1;
        projection.last_permission_submit_error = Some("network down".to_string());

        let footer = projection.footer_text();
        assert!(footer.contains("perm p/s 1/1"));
        assert!(footer.contains("perm-error network down"));
        assert!(!footer.contains("Waiting for permission"));
    }

    #[test]
    fn footer_text_prefers_completed_run_tail_over_idle() {
        let projection = CliFrontendProjection {
            run_tail: Some(CliRunTailState {
                status: "completed".to_string(),
                detail: Some("input 12 · output 34".to_string()),
            }),
            ..CliFrontendProjection::default()
        };

        let footer = projection.footer_text();
        assert!(!footer.contains("Run complete"), "{footer}");
        assert!(!footer.contains("input 12 · output 34"), "{footer}");
        assert!(!footer.contains("Session idle"), "{footer}");
        assert!(footer.contains("/help"), "{footer}");
    }

    #[test]
    fn footer_text_uses_ready_for_empty_idle_projection() {
        let projection = CliFrontendProjection::default();

        let footer = projection.footer_text();

        assert!(footer.contains("Session ready"), "{footer}");
        assert!(!footer.contains("Session idle"), "{footer}");
    }

    #[test]
    fn footer_text_omits_live_runtime_summary_when_prompt_lane_owns_it() {
        let projection = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            active_label: Some("Thinking".to_string()),
            view_label: Some("view attached abcd1234".to_string()),
            ..CliFrontendProjection::default()
        };

        let footer = projection.footer_text();

        assert!(!footer.contains("Running"), "{footer}");
        assert!(!footer.contains("Thinking"), "{footer}");
        assert!(footer.contains("view attached abcd1234"), "{footer}");
        assert!(footer.contains("/abort"), "{footer}");
    }

    #[test]
    fn footer_should_animate_for_busy_and_permission_states() {
        let busy = CliFrontendProjection {
            phase: CliFrontendPhase::Busy,
            ..CliFrontendProjection::default()
        };
        assert!(busy.footer_should_animate());

        let permission = CliFrontendProjection {
            pending_permission_count: 1,
            ..CliFrontendProjection::default()
        };
        assert!(permission.footer_should_animate());

        let idle = CliFrontendProjection::default();
        assert!(!idle.footer_should_animate());
    }
}
