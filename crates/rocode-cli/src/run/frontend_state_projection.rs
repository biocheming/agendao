use super::{format_token_count, CliFrontendProjection};

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
        let state = match self.phase {
            CliFrontendPhase::Idle => "Ready".to_string(),
            CliFrontendPhase::Busy => "Busy".to_string(),
            CliFrontendPhase::Waiting => "Waiting".to_string(),
            CliFrontendPhase::Cancelling => "Cancelling".to_string(),
            CliFrontendPhase::Failed => "Error".to_string(),
        };
        let mut parts = vec![format!(" {} ", state)];
        if let Some(active) = self
            .active_label
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            parts.push(active.to_string());
        }
        if let Some(view) = self.view_label.as_deref().filter(|value| !value.is_empty()) {
            parts.push(view.to_string());
        }
        if self.queue_len > 0 {
            parts.push(format!("queue {}", self.queue_len));
        }
        if self.pending_permission_count > 0 || self.submitting_permission_count > 0 {
            parts.push(format!(
                "perm p/s {}/{}",
                self.pending_permission_count, self.submitting_permission_count
            ));
        }
        if let Some(error) = self.last_permission_submit_error.as_deref() {
            parts.push(format!("perm-error {}", error));
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
                parts.push(label);
            } else {
                parts.push(format!("ctx {}", format_token_count(current_tokens)));
            }
        } else if self.token_stats.total_tokens > 0 {
            parts.push(format!(
                "workflow {}",
                format_token_count(self.token_stats.total_tokens)
            ));
        }
        if self.token_stats.cache_read_tokens > 0
            || self.token_stats.cache_miss_tokens > 0
            || self.token_stats.cache_write_tokens > 0
        {
            parts.push(format!(
                "cache H/M/W {}/{}/{}",
                format_token_count(self.token_stats.cache_read_tokens),
                format_token_count(self.token_stats.cache_miss_tokens),
                format_token_count(self.token_stats.cache_write_tokens)
            ));
        }
        if let Some(cache_diagnostic) = self.cache_diagnostic.as_deref() {
            parts.push(format!("cache {}", cache_diagnostic));
        }
        if let Some(ingress_diagnostic) = self.ingress_diagnostic.as_deref() {
            parts.push(format!("ingress {}", ingress_diagnostic));
        }
        if let Some(provider_diagnostic) = self.provider_diagnostic.as_deref() {
            parts.push(format!("provider {}", provider_diagnostic));
        }
        if let Some(browser) = self.events_browser.as_ref() {
            let page = (browser.offset / browser.filter.limit.unwrap_or(24).max(1)) + 1;
            parts.push(format!("events p{}", page));
        }
        parts.push("Alt+Enter/Ctrl+J newline".to_string());
        parts.push("/help".to_string());
        parts.push("/runtime".to_string());
        parts.push("/insights".to_string());
        parts.push("/validation".to_string());
        parts.push("/attached".to_string());
        if !matches!(self.phase, CliFrontendPhase::Idle) {
            parts.push("/abort".to_string());
        }
        parts.push("Ctrl+D exit".to_string());
        format!(" {} ", parts.join("  •  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_text_surfaces_permission_interaction_state() {
        let mut projection = CliFrontendProjection::default();
        projection.pending_permission_count = 1;
        projection.submitting_permission_count = 1;
        projection.last_permission_submit_error = Some("network down".to_string());

        let footer = projection.footer_text();
        assert!(footer.contains("perm p/s 1/1"));
        assert!(footer.contains("perm-error network down"));
    }
}
