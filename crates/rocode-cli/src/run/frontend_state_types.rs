use super::{
    CliEventsBrowserState, CliFrontendPhase, CliPromptAuxLane, McpStatusInfo,
    SessionExecutionTopology, SessionRuntimeState,
};
use crate::run::session_projection_usage::{cli_format_context_meter, format_token_count};
use rocode_command::cli_style::CliStyle;
use rocode_command::output_blocks::SchedulerStageBlock;
use rocode_command::run_status_labels::{canonical_run_status_labels, canonical_run_status_title};
use std::time::{SystemTime, UNIX_EPOCH};

const CLI_TRANSCRIPT_MAX_LINES: usize = 1200;
const CLI_PROMPT_AUX_MAX_LINES: usize = 3;

// ── Live Slot Transcript Model ───────────────────────────────────────────

/// One entry in the ordered visible transcript timeline.
///
/// LiveSlot entries can be replaced in-place (same slot_key) without changing
/// the timeline order — this is what enables the "replace same identity"
/// contract at the visible output layer.
#[derive(Debug, Clone)]
pub(crate) enum TranscriptEntry {
    /// Immutable committed content (user messages, closed assistant text, etc.).
    Committed { rendered_ansi: String },
    /// Live streaming content that can be updated by subsequent events.
    LiveSlot {
        slot_key: String,
        /// ANSI-rendered text (preserves color, bold, bullet styles).
        rendered_ansi: String,
        /// Plain text for strip_ansi() / width calculation.
        rendered_plain: String,
    },
}

/// Ordered timeline transcript with identity-keyed live slots.
///
/// Identity-keyed visible transcript for CLI live rendering.
///
/// When a full snapshot arrives for the same `{message_id, part_key}`, the old
/// rendered text is replaced in place and the visible output is rebuilt from
/// the timeline — no duplicate headers, no snapshot replay.
#[derive(Debug, Clone)]
pub(crate) struct CliVisibleTranscript {
    entries: Vec<TranscriptEntry>,
    max_lines: usize,
    /// Whether ANSI escape codes are preserved in visible output.
    ansi_capable: bool,
}

impl CliVisibleTranscript {
    pub(crate) fn new(ansi_capable: bool) -> Self {
        Self {
            entries: Vec::new(),
            max_lines: CLI_TRANSCRIPT_MAX_LINES,
            ansi_capable,
        }
    }

    /// Append immutable committed text from finalized history or explicit
    /// compatibility-only paths such as user commands.
    pub(crate) fn append_committed(&mut self, rendered_ansi: &str) {
        // Split into lines for budget trimming.
        for line in rendered_ansi.split_inclusive('\n') {
            self.entries.push(TranscriptEntry::Committed {
                rendered_ansi: line.to_string(),
            });
        }
        self.trim_to_budget();
    }

    /// Upsert a live slot. If a slot with the same key already exists, its
    /// rendered content is replaced in-place (order preserved). Otherwise a
    /// new entry is appended.
    pub(crate) fn upsert_live_slot(
        &mut self,
        slot_key: &str,
        rendered_ansi: String,
        rendered_plain: String,
    ) {
        // Find existing slot by key — replace in place.
        for entry in &mut self.entries {
            if let TranscriptEntry::LiveSlot {
                slot_key: ref existing_key,
                ..
            } = entry
            {
                if existing_key == slot_key {
                    *entry = TranscriptEntry::LiveSlot {
                        slot_key: slot_key.to_string(),
                        rendered_ansi,
                        rendered_plain,
                    };
                    return;
                }
            }
        }
        // New slot — append.
        self.entries.push(TranscriptEntry::LiveSlot {
            slot_key: slot_key.to_string(),
            rendered_ansi,
            rendered_plain,
        });
    }

    /// Commit a live slot — convert it to Committed, preserving position
    /// and rendered_ansi. If the slot doesn't exist, this is a no-op.
    pub(crate) fn commit_slot(&mut self, slot_key: &str) {
        for entry in &mut self.entries {
            if let TranscriptEntry::LiveSlot {
                slot_key: ref existing_key,
                rendered_ansi,
                ..
            } = entry
            {
                if existing_key == slot_key {
                    *entry = TranscriptEntry::Committed {
                        rendered_ansi: rendered_ansi.clone(),
                    };
                    return;
                }
            }
        }
    }

    /// Finalize a live slot by appending a terminal suffix exactly once
    /// and converting it to committed content in place.
    pub(crate) fn finalize_live_slot(
        &mut self,
        slot_key: &str,
        suffix_ansi: String,
        suffix_plain: String,
    ) {
        for entry in &mut self.entries {
            if let TranscriptEntry::LiveSlot {
                slot_key: ref existing_key,
                rendered_ansi,
                rendered_plain,
            } = entry
            {
                if existing_key == slot_key {
                    if !suffix_ansi.is_empty() {
                        rendered_ansi.push_str(&suffix_ansi);
                    }
                    if !suffix_plain.is_empty() {
                        rendered_plain.push_str(&suffix_plain);
                    }
                    *entry = TranscriptEntry::Committed {
                        rendered_ansi: rendered_ansi.clone(),
                    };
                    return;
                }
            }
        }
    }

    /// Rebuild the full visible ANSI text from all entries in timeline order.
    pub(crate) fn visible_ansi(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                TranscriptEntry::Committed { rendered_ansi } => {
                    out.push_str(rendered_ansi);
                }
                TranscriptEntry::LiveSlot { rendered_ansi, .. } => {
                    out.push_str(rendered_ansi);
                }
            }
        }
        out
    }

    /// Plain text version for strip_ansi() consumers and line counting.
    pub(crate) fn visible_plain(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                TranscriptEntry::Committed { rendered_ansi } => {
                    // Use strip_ansi for committed entries.
                    out.push_str(&rocode_util::util::color::strip_ansi(rendered_ansi));
                }
                TranscriptEntry::LiveSlot { rendered_plain, .. } => {
                    out.push_str(rendered_plain);
                }
            }
        }
        out
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Backward-compat: append plain text as committed lines.
    pub(crate) fn append_rendered(&mut self, rendered: &str) {
        self.append_committed(rendered);
    }

    /// Backward-compat: rebuild visible text (ANSI if capable, plain otherwise).
    pub(crate) fn rendered_text(&self) -> String {
        if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.rendered_text().lines().count().max(1)
    }

    pub(crate) fn last_line(&self) -> Option<String> {
        self.rendered_text().lines().last().map(str::to_string)
    }

    fn wrapped_rows(&self, width: usize) -> Vec<String> {
        let text = if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        };
        let mut rows = Vec::new();
        for line in text.lines() {
            let wrapped = rocode_command::cli_panel::wrap_display_text(line, width.max(1));
            if wrapped.is_empty() {
                rows.push(String::new());
            } else {
                rows.extend(wrapped);
            }
        }
        rows
    }

    pub(super) fn viewport_lines(
        &self,
        width: usize,
        max_rows: usize,
        scroll_offset: usize,
    ) -> Vec<String> {
        let mut rows = self.wrapped_rows(width);
        if rows.is_empty() {
            rows.push("No messages yet. Send a prompt to start.".to_string());
        }
        if rows.len() <= max_rows {
            return rows;
        }
        let tail_start = rows.len().saturating_sub(max_rows);
        let start = tail_start.saturating_sub(scroll_offset);
        let end = (start + max_rows).min(rows.len());
        rows[start..end].to_vec()
    }

    #[cfg(test)]
    pub(super) fn total_rows(&self, width: usize) -> usize {
        self.wrapped_rows(width).len().max(1)
    }

    fn trim_to_budget(&mut self) {
        let line_count = self.entries.len();
        if line_count > self.max_lines {
            let overflow = line_count - self.max_lines;
            self.entries.drain(0..overflow);
        }
    }
}

impl Default for CliVisibleTranscript {
    fn default() -> Self {
        Self::new(false)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliSessionTokenStats {
    pub(super) total_tokens: u64,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_miss_tokens: u64,
    pub(super) cache_write_tokens: u64,
    pub(super) context_tokens: u64,
    pub(super) total_cost: f64,
}

impl CliSessionTokenStats {
    pub(super) fn sync_from_snapshot(
        &mut self,
        usage: &rocode_session::SessionUsage,
        usage_books: Option<&rocode_types::SessionUsageBooks>,
    ) {
        let workflow = usage_books
            .map(|books| books.workflow_cumulative.clone())
            .unwrap_or_else(|| usage.workflow_usage_summary());

        self.input_tokens = workflow.input_tokens;
        self.output_tokens = workflow.output_tokens;
        self.reasoning_tokens = workflow.reasoning_tokens;
        self.cache_read_tokens = workflow.cache_read_tokens;
        self.cache_miss_tokens = workflow.cache_miss_tokens;
        self.cache_write_tokens = workflow.cache_write_tokens;
        self.context_tokens = usage_books
            .and_then(|books| books.live_context_tokens)
            .or_else(|| usage.live_context_tokens())
            .or_else(|| usage_books.and_then(|books| books.request_context_tokens))
            .unwrap_or(0);
        self.total_tokens = workflow.total_tokens();
        self.total_cost = workflow.total_cost;
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliLastTurnTokenStats {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_miss_tokens: u64,
    pub(super) cache_write_tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliModelCatalogEntry {
    pub(super) context_window: Option<u64>,
    #[cfg(test)]
    pub(super) cost_per_million_input: Option<f64>,
    #[cfg(test)]
    pub(super) cost_per_million_output: Option<f64>,
}

impl CliModelCatalogEntry {
    pub(super) fn from_provider_model(
        context_window: Option<u64>,
        #[cfg(test)] cost_per_million_input: Option<f64>,
        #[cfg(test)] cost_per_million_output: Option<f64>,
    ) -> Self {
        Self {
            context_window,
            #[cfg(test)]
            cost_per_million_input,
            #[cfg(test)]
            cost_per_million_output,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliPromptLanes {
    pub(super) info_lines: Vec<String>,
    pub(super) warning_lines: Vec<String>,
    pub(super) error_lines: Vec<String>,
}

impl CliPromptLanes {
    pub(super) fn clear(&mut self) {
        self.info_lines.clear();
        self.warning_lines.clear();
        self.error_lines.clear();
    }

    pub(super) fn clear_non_error(&mut self) {
        self.info_lines.clear();
        self.warning_lines.clear();
    }

    pub(super) fn push_aux_line(&mut self, lane: CliPromptAuxLane, rendered: &str) {
        let target = match lane {
            CliPromptAuxLane::Info => &mut self.info_lines,
            CliPromptAuxLane::Warning => &mut self.warning_lines,
            CliPromptAuxLane::Error => &mut self.error_lines,
        };
        target.clear();
        for line in rendered.lines().map(str::to_string) {
            if target.last().is_some_and(|existing| existing == &line) {
                continue;
            }
            target.push(line);
        }
        if target.len() > CLI_PROMPT_AUX_MAX_LINES {
            let overflow = target.len() - CLI_PROMPT_AUX_MAX_LINES;
            target.drain(0..overflow);
        }
    }
}

fn trim_prompt_lane_tail(mut lines: Vec<String>, max_rows: usize) -> Vec<String> {
    if max_rows == 0 {
        return Vec::new();
    }
    if lines.len() > max_rows {
        lines = lines.split_off(lines.len().saturating_sub(max_rows));
    }
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

#[derive(Debug, Clone, Default)]
pub(super) struct CliRunTailState {
    pub(super) status: String,
    pub(super) detail: Option<String>,
}

impl CliRunTailState {
    pub(super) fn line(&self) -> Option<String> {
        if self.status.trim().is_empty() {
            return None;
        }
        let slug = canonical_run_status_labels(&self.status).slug;
        let title = canonical_run_status_title(&self.status);
        if slug == "complete" {
            return match self
                .detail
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Some(detail) => Some(format!("Done: {detail}")),
                None => Some("Done".to_string()),
            };
        }
        if slug == "error" {
            return match self
                .detail
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Some(detail) => Some(format!("Error: {detail}")),
                None => Some("Error".to_string()),
            };
        }
        match self
            .detail
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            Some(detail) => Some(format!("{title}: {detail}")),
            None if self.status != "idle" => Some(title.to_string()),
            None => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CliMcpServerStatus {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) tools: usize,
    pub(super) error: Option<String>,
}

impl From<McpStatusInfo> for CliMcpServerStatus {
    fn from(info: McpStatusInfo) -> Self {
        Self {
            name: info.name,
            status: info.status,
            tools: info.tools,
            error: info.error,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CliFrontendProjection {
    pub(super) phase: CliFrontendPhase,
    pub(super) active_label: Option<String>,
    pub(super) activity_started_at: Option<i64>,
    pub(super) view_label: Option<String>,
    pub(super) queue_len: usize,
    pub(super) prompt_lanes: CliPromptLanes,
    pub(super) run_tail: Option<CliRunTailState>,
    pub(super) active_stage: Option<SchedulerStageBlock>,
    pub(super) session_runtime: Option<SessionRuntimeState>,
    pub(super) stage_summaries: Vec<rocode_command::stage_protocol::StageSummary>,
    pub(super) telemetry_topology: Option<SessionExecutionTopology>,
    pub(super) events_browser: Option<CliEventsBrowserState>,
    pub(super) transcript: CliVisibleTranscript,
    #[cfg(test)]
    pub(super) sidebar_collapsed: bool,
    pub(super) active_collapsed: bool,
    pub(super) session_title: Option<String>,
    pub(super) current_model_label: Option<String>,
    pub(super) scroll_offset: usize,
    pub(super) token_stats: CliSessionTokenStats,
    pub(super) last_turn_tokens: CliLastTurnTokenStats,
    pub(super) cache_diagnostic: Option<String>,
    pub(super) ingress_diagnostic: Option<String>,
    pub(super) provider_diagnostic: Option<String>,
    pub(super) pending_permission_count: usize,
    pub(super) submitting_permission_count: usize,
    pub(super) last_permission_submit_error: Option<String>,
    pub(super) permission_submit_started_at: Option<String>,
    pub(super) permission_submit_completed_at: Option<String>,
    pub(super) model_catalog: std::collections::HashMap<String, CliModelCatalogEntry>,
    pub(super) mcp_servers: Vec<CliMcpServerStatus>,
    pub(super) lsp_servers: Vec<String>,
}

impl Default for CliFrontendProjection {
    fn default() -> Self {
        Self {
            phase: CliFrontendPhase::default(),
            active_label: None,
            activity_started_at: None,
            view_label: None,
            queue_len: 0,
            prompt_lanes: CliPromptLanes::default(),
            run_tail: None,
            active_stage: None,
            session_runtime: None,
            stage_summaries: Vec::new(),
            telemetry_topology: None,
            events_browser: None,
            transcript: CliVisibleTranscript::default(),
            #[cfg(test)]
            sidebar_collapsed: true,
            active_collapsed: true,
            session_title: None,
            current_model_label: None,
            scroll_offset: 0,
            token_stats: CliSessionTokenStats::default(),
            last_turn_tokens: CliLastTurnTokenStats::default(),
            cache_diagnostic: None,
            ingress_diagnostic: None,
            provider_diagnostic: None,
            pending_permission_count: 0,
            submitting_permission_count: 0,
            last_permission_submit_error: None,
            permission_submit_started_at: None,
            permission_submit_completed_at: None,
            model_catalog: std::collections::HashMap::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
        }
    }
}

impl CliFrontendProjection {
    pub(super) fn sync_usage_from_snapshot(
        &mut self,
        usage: &rocode_session::SessionUsage,
        usage_books: Option<&rocode_types::SessionUsageBooks>,
    ) {
        let previous_reasoning_tokens = self.token_stats.reasoning_tokens;
        let previous_cache_read_tokens = self.token_stats.cache_read_tokens;
        let previous_cache_miss_tokens = self.token_stats.cache_miss_tokens;
        let previous_cache_write_tokens = self.token_stats.cache_write_tokens;
        let should_finalize_last_turn =
            self.last_turn_tokens.input_tokens > 0 || self.last_turn_tokens.output_tokens > 0;

        self.token_stats.sync_from_snapshot(usage, usage_books);

        if should_finalize_last_turn {
            self.last_turn_tokens.reasoning_tokens = self
                .token_stats
                .reasoning_tokens
                .saturating_sub(previous_reasoning_tokens);
            self.last_turn_tokens.cache_read_tokens = self
                .token_stats
                .cache_read_tokens
                .saturating_sub(previous_cache_read_tokens);
            self.last_turn_tokens.cache_miss_tokens = self
                .token_stats
                .cache_miss_tokens
                .saturating_sub(previous_cache_miss_tokens);
            self.last_turn_tokens.cache_write_tokens = self
                .token_stats
                .cache_write_tokens
                .saturating_sub(previous_cache_write_tokens);
        }
    }

    pub(super) fn set_runtime_activity(
        &mut self,
        phase: CliFrontendPhase,
        active_label: Option<String>,
    ) {
        let was_active = matches!(
            self.phase,
            CliFrontendPhase::Busy | CliFrontendPhase::Waiting | CliFrontendPhase::Cancelling
        );
        let will_be_active = matches!(
            phase,
            CliFrontendPhase::Busy | CliFrontendPhase::Waiting | CliFrontendPhase::Cancelling
        );
        self.phase = phase;
        if let Some(active_label) = active_label {
            self.active_label = Some(active_label);
        }
        if will_be_active {
            self.run_tail = None;
            if !was_active || self.activity_started_at.is_none() {
                self.activity_started_at = Some(cli_unix_timestamp_now());
                self.prompt_lanes.clear();
            }
        } else {
            self.activity_started_at = None;
            self.prompt_lanes.clear_non_error();
        }
    }

    pub(super) fn clear_runtime_activity(&mut self) {
        self.phase = CliFrontendPhase::Idle;
        self.active_label = None;
        self.activity_started_at = None;
        self.prompt_lanes.clear_non_error();
    }

    pub(super) fn activity_elapsed_seconds(&self) -> Option<u64> {
        let started_at = self.activity_started_at?;
        let now = cli_unix_timestamp_now();
        Some(now.saturating_sub(started_at) as u64)
    }

    fn prompt_progress_lines(&self) -> Vec<String> {
        let queue_label = cli_style_lane_label("Queue", LaneTone::Info);
        let permission_label = cli_style_lane_label("Permission", LaneTone::Warning);
        let mut lines = Vec::new();

        if let Some(stage) = self.active_stage.as_ref() {
            let title = if let (Some(index), Some(total)) = (stage.stage_index, stage.stage_total) {
                format!("stage {index}/{total} · {}", stage.title)
            } else {
                format!("stage · {}", stage.title)
            };
            lines.push(cli_style_live_activity_line(
                &title,
                self.activity_elapsed_seconds(),
                true,
            ));

            let mut summary = Vec::new();
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
            if let Some(focus) = stage.focus.as_deref().filter(|value| !value.is_empty()) {
                summary.push(focus.to_string());
            }
            if !summary.is_empty() {
                lines.push(cli_style_secondary_activity_line(&summary.join(" · ")));
            }
        } else if let Some(active) = self
            .active_label
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(cli_style_live_activity_line(
                active,
                self.activity_elapsed_seconds(),
                true,
            ));
        }

        if self.queue_len > 0 {
            lines.push(format!("{queue_label}: {}", self.queue_len));
        }
        if self.pending_permission_count > 0 || self.submitting_permission_count > 0 {
            lines.push(format!(
                "{permission_label}: pending {} · submitting {}",
                self.pending_permission_count, self.submitting_permission_count
            ));
        }

        lines
    }

    fn prompt_status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(run_tail) = self.run_tail.as_ref().and_then(CliRunTailState::line) {
            lines.push(run_tail);
        }
        lines.extend(self.prompt_lanes.error_lines.iter().cloned());
        lines.extend(self.prompt_lanes.warning_lines.iter().cloned());
        lines.extend(self.prompt_lanes.info_lines.iter().cloned());
        lines
            .into_iter()
            .map(|line| cli_style_prompt_status_line(&line))
            .collect()
    }

    fn prompt_usage_lines(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(current_tokens) = self.current_context_tokens() {
            let context_window = self
                .current_model_label
                .as_deref()
                .and_then(|label| self.model_catalog.get(label))
                .and_then(|entry| entry.context_window)
                .filter(|value| *value > 0);
            parts.push(format!(
                "ctx {}",
                cli_format_context_meter(current_tokens, context_window)
            ));
        }

        if self.token_stats.input_tokens > 0 {
            parts.push(format!(
                "in {}",
                format_token_count(self.token_stats.input_tokens)
            ));
        }
        if self.token_stats.output_tokens > 0 {
            parts.push(format!(
                "out {}",
                format_token_count(self.token_stats.output_tokens)
            ));
        }
        if self.token_stats.reasoning_tokens > 0 {
            parts.push(format!(
                "reason {}",
                format_token_count(self.token_stats.reasoning_tokens)
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

        if parts.is_empty() {
            Vec::new()
        } else {
            vec![cli_style_usage_line(&parts)]
        }
    }

    #[cfg(test)]
    pub(super) fn prompt_lane_lines(&self) -> Vec<String> {
        let progress_lines = self.prompt_progress_lines();
        let mut lines = progress_lines.clone();
        let status_lines = self.prompt_status_lines();
        if !progress_lines.is_empty() && !status_lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(status_lines);
        lines
    }

    pub(super) fn prompt_lane_lines_stable(&self, stable_rows: usize) -> Vec<String> {
        if stable_rows == 0 {
            return Vec::new();
        }
        let progress_lines = self
            .prompt_progress_lines()
            .into_iter()
            .take(2)
            .collect::<Vec<_>>();
        let status_lines = self
            .prompt_status_lines()
            .into_iter()
            .take(3)
            .collect::<Vec<_>>();
        let usage_lines = self
            .prompt_usage_lines()
            .into_iter()
            .take(1)
            .collect::<Vec<_>>();

        let mut lines = Vec::new();
        if !progress_lines.is_empty() {
            lines.extend(progress_lines);
        }
        if !status_lines.is_empty() {
            lines.extend(status_lines);
        }
        if !usage_lines.is_empty() {
            lines.extend(usage_lines);
        }

        trim_prompt_lane_tail(lines, stable_rows)
    }
}

#[derive(Clone, Copy)]
enum LaneTone {
    Info,
    Warning,
}

fn cli_style_lane_label(label: &str, tone: LaneTone) -> String {
    let style = CliStyle::detect();
    match tone {
        LaneTone::Info => style.bold_rgb(label, 120, 210, 255),
        LaneTone::Warning => style.bold_yellow(label),
    }
}

fn cli_spinner_frame_plain() -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let elapsed_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as usize)
        .unwrap_or(0);
    FRAMES[(elapsed_ms / 80) % FRAMES.len()]
}

fn cli_unix_timestamp_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn cli_fmt_elapsed_compact(elapsed_secs: u64) -> String {
    if elapsed_secs < 60 {
        return format!("{elapsed_secs}s");
    }
    if elapsed_secs < 3600 {
        return format!("{}m {:02}s", elapsed_secs / 60, elapsed_secs % 60);
    }
    format!(
        "{}h {:02}m {:02}s",
        elapsed_secs / 3600,
        (elapsed_secs % 3600) / 60,
        elapsed_secs % 60
    )
}

fn cli_style_live_activity_line(
    activity: &str,
    elapsed_seconds: Option<u64>,
    interruptible: bool,
) -> String {
    let style = CliStyle::detect();
    let spinner = if style.color {
        style.bold_cyan(cli_spinner_frame_plain())
    } else {
        "*".to_string()
    };
    let title = if style.color {
        style.bold_cyan(activity)
    } else {
        activity.to_string()
    };
    let mut suffix = Vec::new();
    if let Some(elapsed_seconds) = elapsed_seconds {
        suffix.push(cli_fmt_elapsed_compact(elapsed_seconds));
    }
    if interruptible {
        suffix.push("Esc/Ctrl+C to interrupt".to_string());
    }
    if suffix.is_empty() {
        format!("{spinner} {title}")
    } else {
        format!(
            "{spinner} {title} {}",
            style.dim(&format!("· {}", suffix.join(" · ")))
        )
    }
}

fn cli_style_secondary_activity_line(activity: &str) -> String {
    let style = CliStyle::detect();
    if style.color {
        format!("{} {}", style.dim("·"), style.dim(activity))
    } else {
        format!("· {activity}")
    }
}

fn cli_style_prompt_status_line(line: &str) -> String {
    let style = CliStyle::detect();
    if let Some(rest) = line.strip_prefix("Done:") {
        return cli_style_completion_line(rest.trim());
    }
    if line.trim() == "Done" {
        return cli_style_completion_line("");
    }
    if let Some(rest) = line.strip_prefix("Error:") {
        return cli_style_error_line(rest.trim());
    }
    if line.trim() == "Error" {
        return cli_style_error_line("");
    }
    if let Some(rest) = line.strip_prefix("Warning:") {
        return format!("{}:{}", style.bold_yellow("Warning"), style.yellow(rest));
    }
    if let Some(rest) = line.strip_prefix("Info:") {
        return format!("{}:{}", style.bold_cyan("Info"), style.dim(rest));
    }
    line.to_string()
}

fn cli_style_usage_line(parts: &[String]) -> String {
    let style = CliStyle::detect();
    let usage_label = style.bold_rgb("Usage", 120, 210, 255);
    let styled_parts = parts
        .iter()
        .map(|part| {
            if part.starts_with("ctx ") {
                style.cyan(part)
            } else if part.starts_with("in ")
                || part.starts_with("out ")
                || part.starts_with("reason ")
            {
                style.rgb(part, 190, 220, 255)
            } else {
                style.dim(part)
            }
        })
        .collect::<Vec<_>>();
    format!("{usage_label}: {}", styled_parts.join(&style.dim(" · ")))
}

fn cli_style_status_chip(label: &str, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> String {
    let style = CliStyle::detect();
    if style.color {
        format!(
            "\x1b[1;38;2;{};{};{};48;2;{};{};{}m {} \x1b[0m",
            fg.0, fg.1, fg.2, bg.0, bg.1, bg.2, label
        )
    } else {
        format!("[{label}]")
    }
}

fn cli_style_completion_line(detail: &str) -> String {
    let style = CliStyle::detect();
    let chip = cli_style_status_chip("Done", (230, 255, 236), (24, 110, 61));
    if detail.is_empty() {
        chip
    } else {
        format!("{chip} {}", style.dim(detail))
    }
}

fn cli_style_error_line(detail: &str) -> String {
    let style = CliStyle::detect();
    let chip = cli_style_status_chip("Error", (255, 236, 236), (140, 32, 32));
    if detail.is_empty() {
        chip
    } else {
        format!("{chip} {}", style.red(detail))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliFrontendProjection, CliPromptAuxLane, CliPromptLanes, CliRunTailState,
        CliSessionTokenStats, CliVisibleTranscript,
    };
    use crate::run::{cli_apply_live_slot_update, CliFrontendPhase};
    use rocode_command::cli_style::CliStyle;
    use rocode_command::output_blocks::{OutputBlock, SchedulerStageBlock, ToolBlock};
    use rocode_session::SessionUsage;
    use rocode_types::{
        LiveMessagePartIdentity, LiveMessagePartKind, LivePartPhase, SessionUsageBooks,
        WorkflowUsageSummary,
    };
    use rocode_util::util::color::strip_ansi;

    #[test]
    fn sync_from_snapshot_uses_request_context_when_live_is_missing() {
        let usage = SessionUsage {
            context_tokens: 0,
            ..SessionUsage::default()
        };
        let usage_books = SessionUsageBooks {
            request_context_tokens: Some(48_000),
            live_context_tokens: None,
            workflow_cumulative: WorkflowUsageSummary::default(),
        };
        let mut stats = CliSessionTokenStats::default();

        stats.sync_from_snapshot(&usage, Some(&usage_books));

        assert_eq!(stats.context_tokens, 48_000);
    }

    #[test]
    fn prompt_lanes_keep_progress_and_status_separate() {
        let mut projection = CliFrontendProjection {
            active_label: Some("Skill SkillsList".to_string()),
            prompt_lanes: CliPromptLanes {
                error_lines: vec!["Error: boom".to_string()],
                ..CliPromptLanes::default()
            },
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert!(
            plain_lines[0].contains("Skill SkillsList"),
            "{plain_lines:?}"
        );
        assert!(plain_lines[0].contains("interrupt"), "{plain_lines:?}");
        assert_eq!(plain_lines[1], "");
        assert_eq!(plain_lines[2], "[Error] boom");

        projection
            .prompt_lanes
            .push_aux_line(CliPromptAuxLane::Info, "Done: tokens ready");
        let done_lines = projection
            .prompt_lane_lines()
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();
        assert!(done_lines.iter().any(|line| line == "[Done] tokens ready"));
    }

    #[test]
    fn prompt_lanes_surface_run_tail_ahead_of_aux_status() {
        let projection = CliFrontendProjection {
            run_tail: Some(CliRunTailState {
                status: "complete".to_string(),
                detail: Some("input 12 · output 34".to_string()),
            }),
            prompt_lanes: CliPromptLanes {
                info_lines: vec!["Info: background sync".to_string()],
                ..CliPromptLanes::default()
            },
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(
            plain_lines.first().map(String::as_str),
            Some("[Done] input 12 · output 34")
        );
        assert_eq!(
            plain_lines.get(1).map(String::as_str),
            Some("Info: background sync")
        );
    }

    #[test]
    fn prompt_lanes_prioritize_error_then_warning_then_info() {
        let projection = CliFrontendProjection {
            prompt_lanes: CliPromptLanes {
                info_lines: vec!["Info: background sync".to_string()],
                warning_lines: vec!["Warning: retry scheduled".to_string()],
                error_lines: vec!["Error: boom".to_string()],
            },
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(
            plain_lines,
            vec![
                "[Error] boom".to_string(),
                "Warning: retry scheduled".to_string(),
                "Info: background sync".to_string(),
            ]
        );
    }

    #[test]
    fn prompt_lanes_dedupe_repeated_aux_lines_per_lane() {
        let mut lanes = CliPromptLanes::default();

        lanes.push_aux_line(CliPromptAuxLane::Warning, "Warning: retry scheduled");
        lanes.push_aux_line(CliPromptAuxLane::Warning, "Warning: retry scheduled");
        lanes.push_aux_line(CliPromptAuxLane::Error, "Error: boom");
        lanes.push_aux_line(CliPromptAuxLane::Error, "Error: boom");

        assert_eq!(
            lanes.warning_lines,
            vec!["Warning: retry scheduled".to_string()]
        );
        assert_eq!(lanes.error_lines, vec!["Error: boom".to_string()]);
    }

    #[test]
    fn prompt_lanes_keep_only_latest_aux_block_per_lane() {
        let mut lanes = CliPromptLanes::default();

        lanes.push_aux_line(CliPromptAuxLane::Info, "Info: Using Skill SkillsList");
        lanes.push_aux_line(CliPromptAuxLane::Info, "Info: Using Skill SkillView");
        lanes.push_aux_line(
            CliPromptAuxLane::Warning,
            "Warning: Awaiting permission · Using Bash",
        );
        lanes.push_aux_line(
            CliPromptAuxLane::Warning,
            "Warning: Awaiting permission · Using ExternalDirectory",
        );

        assert_eq!(
            lanes.info_lines,
            vec!["Info: Using Skill SkillView".to_string()]
        );
        assert_eq!(
            lanes.warning_lines,
            vec!["Warning: Awaiting permission · Using ExternalDirectory".to_string()]
        );
    }

    #[test]
    fn leaving_active_phase_clears_non_error_aux_lines() {
        let mut projection = CliFrontendProjection::default();
        projection
            .prompt_lanes
            .push_aux_line(CliPromptAuxLane::Info, "Info: Using Skill SkillsList");
        projection.prompt_lanes.push_aux_line(
            CliPromptAuxLane::Warning,
            "Warning: Awaiting permission · Using Bash",
        );
        projection
            .prompt_lanes
            .push_aux_line(CliPromptAuxLane::Error, "Error: boom");

        projection.set_runtime_activity(CliFrontendPhase::Busy, Some("Thinking".to_string()));
        projection.set_runtime_activity(CliFrontendPhase::Idle, None);

        assert!(projection.prompt_lanes.info_lines.is_empty());
        assert!(projection.prompt_lanes.warning_lines.is_empty());
        assert!(projection.prompt_lanes.error_lines.is_empty());
    }

    #[test]
    fn prompt_usage_line_surfaces_context_and_token_flow() {
        let mut projection = CliFrontendProjection::default();
        projection.current_model_label = Some("openai/gpt-5".to_string());
        projection.model_catalog.insert(
            "openai/gpt-5".to_string(),
            super::CliModelCatalogEntry::from_provider_model(Some(200_000), None, None),
        );
        projection.token_stats.context_tokens = 52_830;
        projection.token_stats.input_tokens = 12_000;
        projection.token_stats.output_tokens = 4_000;
        projection.token_stats.reasoning_tokens = 1_500;
        projection.token_stats.cache_read_tokens = 40_000;
        projection.token_stats.cache_miss_tokens = 6_000;
        projection.token_stats.cache_write_tokens = 2_000;

        let lines = projection.prompt_usage_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(
            plain_lines,
            vec![
                "Usage: ctx 52.8K/200K [███░░░░░░░] 26% · in 12K · out 4K · reason 1.5K · cache H/M/W 40K/6K/2K".to_string()
            ]
        );
    }

    #[test]
    fn sync_usage_from_snapshot_computes_last_turn_reasoning_and_cache_delta() {
        let mut projection = CliFrontendProjection::default();
        projection.token_stats.input_tokens = 120_000;
        projection.token_stats.output_tokens = 18_000;
        projection.token_stats.reasoning_tokens = 5_000;
        projection.token_stats.cache_read_tokens = 34_000;
        projection.token_stats.cache_miss_tokens = 7_000;
        projection.token_stats.cache_write_tokens = 2_000;
        projection.last_turn_tokens.input_tokens = 12_000;
        projection.last_turn_tokens.output_tokens = 4_000;

        let usage = SessionUsage {
            input_tokens: 90_000,
            output_tokens: 10_000,
            reasoning_tokens: 6_500,
            cache_write_tokens: 2_700,
            cache_read_tokens: 42_000,
            cache_miss_tokens: 8_500,
            context_tokens: 82_000,
            total_cost: 1.25,
        };
        let usage_books = SessionUsageBooks {
            request_context_tokens: Some(88_000),
            live_context_tokens: Some(82_000),
            workflow_cumulative: WorkflowUsageSummary {
                input_tokens: 132_000,
                output_tokens: 22_000,
                reasoning_tokens: 6_500,
                cache_write_tokens: 2_700,
                cache_read_tokens: 42_000,
                cache_miss_tokens: 8_500,
                total_cost: 1.60,
            },
        };

        projection.sync_usage_from_snapshot(&usage, Some(&usage_books));

        assert_eq!(projection.last_turn_tokens.input_tokens, 12_000);
        assert_eq!(projection.last_turn_tokens.output_tokens, 4_000);
        assert_eq!(projection.last_turn_tokens.reasoning_tokens, 1_500);
        assert_eq!(projection.last_turn_tokens.cache_read_tokens, 8_000);
        assert_eq!(projection.last_turn_tokens.cache_miss_tokens, 1_500);
        assert_eq!(projection.last_turn_tokens.cache_write_tokens, 700);
    }

    #[test]
    fn prompt_lane_lines_stable_keeps_only_visible_lines() {
        let projection = CliFrontendProjection {
            active_label: Some("Skill SkillsList".to_string()),
            run_tail: Some(CliRunTailState {
                status: "running".to_string(),
                detail: Some("Current stage: Research".to_string()),
            }),
            token_stats: CliSessionTokenStats {
                input_tokens: 12_000,
                ..CliSessionTokenStats::default()
            },
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines_stable(7);
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(plain_lines.len(), 3, "{plain_lines:?}");
        assert!(
            plain_lines
                .first()
                .is_some_and(|line| line.contains("Skill SkillsList")),
            "{plain_lines:?}"
        );
        assert!(
            plain_lines
                .iter()
                .any(|line| line == "Running: Current stage: Research"),
            "{plain_lines:?}"
        );
        assert!(
            plain_lines.iter().any(|line| line == "Usage: in 12K"),
            "{plain_lines:?}"
        );
    }

    #[test]
    fn prompt_lanes_prefer_active_stage_summary() {
        let projection = CliFrontendProjection {
            active_label: Some("assistant response".to_string()),
            active_stage: Some(SchedulerStageBlock {
                stage_id: Some("stage-1".to_string()),
                profile: None,
                stage: "research".to_string(),
                title: "Research".to_string(),
                text: "planning".to_string(),
                stage_index: Some(1),
                stage_total: Some(3),
                step: None,
                status: Some("running".to_string()),
                focus: Some("SkillsList".to_string()),
                last_event: None,
                waiting_on: Some("tool".to_string()),
                estimated_context_tokens: None,
                skill_tree_budget: None,
                skill_tree_truncation_strategy: None,
                skill_tree_truncated: None,
                retry_attempt: None,
                activity: None,
                loop_budget: None,
                available_skill_count: None,
                available_agent_count: None,
                available_category_count: None,
                active_skills: Vec::new(),
                active_agents: Vec::new(),
                active_categories: Vec::new(),
                done_agent_count: 0,
                total_agent_count: 0,
                prompt_tokens: None,
                context_tokens: None,
                completion_tokens: None,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_miss_tokens: None,
                cache_write_tokens: None,
                attached_session_id: None,
                decision: None,
            }),
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();
        assert!(plain_lines[0].contains("Research"), "{plain_lines:?}");
        assert!(plain_lines[1].contains("running"), "{plain_lines:?}");
        assert!(
            plain_lines[1].contains("waiting on tool"),
            "{plain_lines:?}"
        );
    }

    // ── P3-I: Live slot replace contract tests ─────────────────────────

    #[test]
    fn visible_transcript_upsert_replaces_same_slot_in_place() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.upsert_live_slot("msg-1:text/main", "hello".to_string(), "hello".to_string());
        assert_eq!(transcript.rendered_text(), "hello");

        // Same slot_key, different content → replace, not append.
        transcript.upsert_live_slot(
            "msg-1:text/main",
            "hello world".to_string(),
            "hello world".to_string(),
        );
        let visible = transcript.rendered_text();
        assert_eq!(
            visible, "hello world",
            "same slot_key must replace, not append: {visible}"
        );
        assert!(
            !visible.contains("hellohello"),
            "duplicate text detected — slot was appended instead of replaced: {visible}"
        );
    }

    #[test]
    fn visible_transcript_commit_preserves_content_in_order() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.upsert_live_slot(
            "msg-1:reasoning/main",
            "thinking".to_string(),
            "thinking".to_string(),
        );
        transcript.upsert_live_slot(
            "msg-1:text/main",
            "answer".to_string(),
            "answer".to_string(),
        );
        // Commit the reasoning slot — content preserved, order preserved.
        transcript.commit_slot("msg-1:reasoning/main");
        let visible = transcript.rendered_text();
        let reasoning_pos = visible.find("thinking").unwrap_or(usize::MAX);
        let answer_pos = visible.find("answer").unwrap_or(usize::MAX);
        assert!(
            reasoning_pos < answer_pos,
            "committed reasoning must appear before live text in timeline order"
        );
    }

    #[test]
    fn scheduler_stage_identity_is_committed_not_live_slot() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::scheduler_stage_part_key("stage-1"),
            part_kind: LiveMessagePartKind::SchedulerStage,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: None,
        };
        let block = OutputBlock::SchedulerStage(Box::new(SchedulerStageBlock {
            stage_id: Some("stage-1".to_string()),
            profile: Some("default".to_string()),
            stage: "research".to_string(),
            title: "Research".to_string(),
            text: "planning".to_string(),
            stage_index: Some(1),
            stage_total: Some(3),
            step: Some(1),
            status: Some("running".to_string()),
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            activity: None,
            loop_budget: None,
            available_skill_count: None,
            available_agent_count: None,
            available_category_count: None,
            active_skills: Vec::new(),
            active_agents: Vec::new(),
            active_categories: Vec::new(),
            done_agent_count: 0,
            total_agent_count: 0,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_miss_tokens: None,
            cache_write_tokens: None,
            attached_session_id: None,
            decision: None,
        }));

        cli_apply_live_slot_update(&mut transcript, &block, &identity, &style);

        assert_eq!(transcript.rendered_text(), "");
    }

    #[test]
    fn assistant_start_and_end_do_not_materialize_blank_visible_lines() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let start_identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Start,
            legacy_block_id: Some("msg-1".to_string()),
        };
        let end_identity = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..start_identity.clone()
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(rocode_command::output_blocks::MessageBlock::start(
                rocode_command::output_blocks::MessageRole::Assistant,
            )),
            &start_identity,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(rocode_command::output_blocks::MessageBlock::end(
                rocode_command::output_blocks::MessageRole::Assistant,
            )),
            &end_identity,
            &style,
        );

        assert_eq!(transcript.rendered_text(), "");
    }

    #[test]
    fn empty_reasoning_start_end_do_not_materialize_thinking_shell() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let start_identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Start,
            legacy_block_id: Some("msg-1".to_string()),
        };
        let end_identity = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..start_identity.clone()
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::start()),
            &start_identity,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::end()),
            &end_identity,
            &style,
        );

        let rendered = transcript.rendered_text();
        assert_eq!(rendered, "", "{rendered}");
        assert!(!rendered.contains("Thinking"), "{rendered}");
    }

    #[test]
    fn empty_assistant_and_reasoning_boundaries_leave_transcript_empty() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let assistant_start = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::ASSISTANT_TEXT_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantText,
            phase: LivePartPhase::Start,
            legacy_block_id: Some("msg-1".to_string()),
        };
        let assistant_end = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..assistant_start.clone()
        };
        let reasoning_start = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Start,
            legacy_block_id: Some("msg-1".to_string()),
        };
        let reasoning_end = LiveMessagePartIdentity {
            phase: LivePartPhase::End,
            ..reasoning_start.clone()
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(rocode_command::output_blocks::MessageBlock::start(
                rocode_command::output_blocks::MessageRole::Assistant,
            )),
            &assistant_start,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::start()),
            &reasoning_start,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::end()),
            &reasoning_end,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Message(rocode_command::output_blocks::MessageBlock::end(
                rocode_command::output_blocks::MessageRole::Assistant,
            )),
            &assistant_end,
            &style,
        );

        assert_eq!(transcript.rendered_text(), "");
    }

    #[test]
    fn tool_running_progress_identity_bypasses_live_slot_and_stays_descriptive() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::tool_call_part_key("call-1"),
            part_kind: LiveMessagePartKind::ToolCall,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("call-1".to_string()),
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Tool(ToolBlock::running("SkillsList".to_string(), "".to_string())),
            &identity,
            &style,
        );

        assert_eq!(transcript.rendered_text(), "");
    }

    #[test]
    fn reasoning_full_snapshots_replace_same_slot_without_replaying_thinking_header() {
        let mut transcript = CliVisibleTranscript::new(false);
        let style = CliStyle::plain();
        let identity = LiveMessagePartIdentity {
            message_id: "msg-1".to_string(),
            part_key: rocode_types::ASSISTANT_REASONING_MAIN_PART_KEY.to_string(),
            part_kind: LiveMessagePartKind::AssistantReasoning,
            phase: LivePartPhase::Snapshot,
            legacy_block_id: Some("msg-1".to_string()),
        };

        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking first".to_string(),
            )),
            &identity,
            &style,
        );
        cli_apply_live_slot_update(
            &mut transcript,
            &OutputBlock::Reasoning(rocode_command::output_blocks::ReasoningBlock::full(
                "Thinking first second".to_string(),
            )),
            &identity,
            &style,
        );

        let rendered = transcript.rendered_text();
        assert_eq!(rendered.matches("[thinking]").count(), 1, "{rendered}");
        assert!(rendered.contains("Thinking first second"), "{rendered}");
        assert!(
            !rendered.contains("Thinking first\n[thinking]"),
            "{rendered}"
        );
    }

    #[test]
    fn prompt_lanes_surface_skill_aware_progress_label() {
        let projection = CliFrontendProjection {
            active_label: Some("Skill SkillsList".to_string()),
            ..CliFrontendProjection::default()
        };

        let lines = projection.prompt_lane_lines();
        let plain_lines = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();
        assert!(
            plain_lines
                .first()
                .is_some_and(|line| line.contains("Skill SkillsList")),
            "{plain_lines:?}"
        );
        assert!(
            plain_lines
                .first()
                .is_some_and(|line| line.contains("interrupt")),
            "{plain_lines:?}"
        );
    }
}
