use super::{
    CliEventsBrowserState, CliFrontendPhase, McpStatusInfo, SessionExecutionTopology,
    SessionRuntimeState,
};
use rocode_command::output_blocks::SchedulerStageBlock;
use rocode_util::util::color::strip_ansi;

const CLI_TRANSCRIPT_MAX_LINES: usize = 1200;

#[derive(Debug, Clone, Default)]
pub(super) struct CliRetainedTranscript {
    pub(super) committed_lines: Vec<String>,
    pub(super) open_line: String,
}

impl CliRetainedTranscript {
    pub(super) fn append_rendered(&mut self, rendered: &str) {
        let normalized = strip_ansi(rendered).replace('\r', "");
        for chunk in normalized.split_inclusive('\n') {
            if let Some(content) = chunk.strip_suffix('\n') {
                self.open_line.push_str(content);
                self.committed_lines
                    .push(std::mem::take(&mut self.open_line));
                self.trim_to_budget();
            } else {
                self.open_line.push_str(chunk);
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.committed_lines.clear();
        self.open_line.clear();
    }

    pub(super) fn rendered_text(&self) -> String {
        let mut out = String::new();
        for line in &self.committed_lines {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&self.open_line);
        out
    }

    pub(super) fn line_count(&self) -> usize {
        self.committed_lines.len() + usize::from(!self.open_line.is_empty())
    }

    pub(super) fn last_line(&self) -> Option<&str> {
        if !self.open_line.is_empty() {
            Some(self.open_line.as_str())
        } else {
            self.committed_lines.last().map(String::as_str)
        }
    }

    #[cfg(test)]
    pub(super) fn viewport_lines(
        &self,
        width: usize,
        max_rows: usize,
        scroll_offset: usize,
    ) -> Vec<String> {
        let mut rows = Vec::new();
        for line in &self.committed_lines {
            super::extend_wrapped_lines(&mut rows, line, width);
        }
        if !self.open_line.is_empty() || rows.is_empty() {
            super::extend_wrapped_lines(&mut rows, &self.open_line, width);
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
        let mut count = 0usize;
        for line in &self.committed_lines {
            count += rocode_command::cli_panel::wrap_display_text(line, width.max(1)).len();
        }
        if !self.open_line.is_empty() {
            count +=
                rocode_command::cli_panel::wrap_display_text(&self.open_line, width.max(1)).len();
        }
        count.max(1)
    }

    fn trim_to_budget(&mut self) {
        if self.committed_lines.len() > CLI_TRANSCRIPT_MAX_LINES {
            let overflow = self.committed_lines.len() - CLI_TRANSCRIPT_MAX_LINES;
            self.committed_lines.drain(0..overflow);
        }
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
    pub(super) transcript: CliRetainedTranscript,
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
            transcript: CliRetainedTranscript::default(),
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
            model_catalog: std::collections::HashMap::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
        }
    }
}
