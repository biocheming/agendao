use super::{
    CliEventsBrowserState, CliFrontendPhase, McpStatusInfo, SessionExecutionTopology,
    SessionRuntimeState,
};
use rocode_command::output_blocks::SchedulerStageBlock;

const CLI_TRANSCRIPT_MAX_LINES: usize = 1200;

// ── Live Slot Transcript Model ───────────────────────────────────────────

/// One entry in the ordered visible transcript timeline.
///
/// LiveSlot entries can be replaced in-place (same slot_key) without changing
/// the timeline order — this is what enables the "replace same identity"
/// contract at the visible output layer.
#[derive(Debug, Clone)]
pub(super) enum TranscriptEntry {
    /// Immutable committed content (user messages, closed assistant text, etc.).
    Committed {
        rendered_ansi: String,
    },
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
pub(super) struct CliVisibleTranscript {
    entries: Vec<TranscriptEntry>,
    max_lines: usize,
    /// Whether ANSI escape codes are preserved in visible output.
    ansi_capable: bool,
}

impl CliVisibleTranscript {
    pub(super) fn new(ansi_capable: bool) -> Self {
        Self {
            entries: Vec::new(),
            max_lines: CLI_TRANSCRIPT_MAX_LINES,
            ansi_capable,
        }
    }

    /// Append immutable committed text (legacy path, user commands, etc.).
    pub(super) fn append_committed(&mut self, rendered_ansi: &str) {
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
    pub(super) fn upsert_live_slot(
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
    pub(super) fn commit_slot(&mut self, slot_key: &str) {
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

    /// Rebuild the full visible ANSI text from all entries in timeline order.
    pub(super) fn visible_ansi(&self) -> String {
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
    pub(super) fn visible_plain(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                TranscriptEntry::Committed { rendered_ansi } => {
                    // Use strip_ansi for committed entries.
                    out.push_str(&rocode_util::util::color::strip_ansi(rendered_ansi));
                }
                TranscriptEntry::LiveSlot {
                    rendered_plain, ..
                } => {
                    out.push_str(rendered_plain);
                }
            }
        }
        out
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Backward-compat: append plain text as committed lines.
    pub(super) fn append_rendered(&mut self, rendered: &str) {
        self.append_committed(rendered);
    }

    /// Backward-compat: rebuild visible text (ANSI if capable, plain otherwise).
    pub(super) fn rendered_text(&self) -> String {
        if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        }
    }

    pub(super) fn line_count(&self) -> usize {
        self.rendered_text().lines().count().max(1)
    }

    pub(super) fn last_line(&self) -> Option<String> {
        self.rendered_text().lines().last().map(str::to_string)
    }

    #[cfg(test)]
    pub(super) fn last_rendered_line(&self) -> Option<String> {
        let text = if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        };
        text.lines().last().map(str::to_string)
    }

    #[cfg(test)]
    pub(super) fn viewport_lines(
        &self,
        width: usize,
        max_rows: usize,
        scroll_offset: usize,
    ) -> Vec<String> {
        let text = if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        };
        let mut rows = Vec::new();
        for line in text.lines() {
            super::extend_wrapped_lines(&mut rows, line, width);
        }
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
        let text = if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        };
        let mut count = 0usize;
        for line in text.lines() {
            count += rocode_command::cli_panel::wrap_display_text(line, width.max(1)).len();
        }
        count.max(1)
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
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Default)]
pub(super) struct CliModelCatalogEntry {
    pub(super) context_window: Option<u64>,
    pub(super) cost_per_million_input: Option<f64>,
    pub(super) cost_per_million_output: Option<f64>,
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
    pub(super) view_label: Option<String>,
    pub(super) queue_len: usize,
    pub(super) active_stage: Option<SchedulerStageBlock>,
    pub(super) session_runtime: Option<SessionRuntimeState>,
    pub(super) stage_summaries: Vec<rocode_command::stage_protocol::StageSummary>,
    pub(super) telemetry_topology: Option<SessionExecutionTopology>,
    pub(super) events_browser: Option<CliEventsBrowserState>,
    pub(super) transcript: CliVisibleTranscript,
    #[cfg_attr(not(test), allow(dead_code))]
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
            view_label: None,
            queue_len: 0,
            active_stage: None,
            session_runtime: None,
            stage_summaries: Vec::new(),
            telemetry_topology: None,
            events_browser: None,
            transcript: CliVisibleTranscript::default(),
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

#[cfg(test)]
mod tests {
    use super::{CliSessionTokenStats, CliVisibleTranscript};
    use rocode_session::SessionUsage;
    use rocode_types::{SessionUsageBooks, WorkflowUsageSummary};

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

    // ── P3-I: Live slot replace contract tests ─────────────────────────

    #[test]
    fn visible_transcript_upsert_replaces_same_slot_in_place() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.upsert_live_slot(
            "msg-1:text/main",
            "hello".to_string(),
            "hello".to_string(),
        );
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
    fn visible_transcript_isolates_different_slot_keys() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.upsert_live_slot(
            "msg-1:text/main",
            "assistant text".to_string(),
            "assistant text".to_string(),
        );
        transcript.upsert_live_slot(
            "msg-1:reasoning/main",
            "thinking...".to_string(),
            "thinking...".to_string(),
        );
        let visible = transcript.rendered_text();
        assert!(visible.contains("assistant text"), "missing assistant text: {visible}");
        assert!(visible.contains("thinking..."), "missing reasoning: {visible}");
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
}
