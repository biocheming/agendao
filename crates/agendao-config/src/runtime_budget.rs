//! Runtime budget authority (§5 single state ownership).
//!
//! Every numerical runtime limit is sourced from this authority.
//!
//! Override these defaults via the `runtimeBudget` section in `agendao.json`
//! or `agendao.jsonc`.

use serde::{Deserialize, Serialize};

/// Canonical budget authority for all runtime resource limits.
///
/// Constitution §5: a single struct owns every budget field. No semantic
/// duplicates in server/session/frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeBudgetConfig {
    // ── Scheduler execution ──
    pub scheduler_max_model_calls: u32,
    pub scheduler_max_tool_calls: u32,
    pub scheduler_max_total_tokens: u64,
    pub scheduler_max_wall_time_ms: u64,
    pub scheduler_max_parallelism: u32,
    pub scheduler_max_graph_nodes: u32,
    pub scheduler_max_graph_depth: u32,
    pub scheduler_max_loop_iterations: u32,
    pub scheduler_max_agent_steps: u32,

    // ── Scheduler workspace capabilities ──
    pub scheduler_workspace_max_files: u32,
    pub scheduler_workspace_max_total_bytes: u64,
    pub scheduler_workspace_min_free_disk_bytes: u64,
    pub scheduler_workspace_operation_timeout_ms: u64,

    // ── Tool result governance ──
    /// Max chars in a single tool result before artifact offload.
    pub tool_result_max_chars: usize,
    /// Preview chars kept when a single tool result exceeds the budget.
    pub tool_result_preview_chars: usize,
    /// Max aggregate chars across all tool results in a batch.
    pub tool_batch_aggregate_max_chars: usize,

    // ── Transcript / context ──
    /// Max chars of a single tool result kept in the session transcript.
    pub max_tool_result_transcript_chars: usize,
    /// Max attachment bytes when reading files for the model.
    pub max_attachment_bytes: usize,
    /// Max MCP resource chars included in system prompt.
    pub max_mcp_resource_chars: usize,

    // ── Streaming / connection ──
    /// Max events buffered per SSE/event-bridge connection before dropping.
    pub stream_connection_queue_size: usize,
    /// Max UI bridge events queued in TUI frontend.
    pub max_ui_bridge_queue: usize,
    /// Max events processed per render frame in TUI.
    pub max_events_per_frame: usize,

    // ── Frontend local caches ──
    /// Max entries in TUI prompt history.
    pub frontend_max_prompt_history_entries: usize,
    /// Max stash entries in TUI prompt.
    pub frontend_max_stash_entries: usize,
    /// Max frecency entries in TUI prompt.
    pub frontend_max_frecency_entries: usize,
    /// Max lines retained in CLI transcript.
    pub frontend_max_transcript_lines: usize,
    /// Max chars for semantic highlighting in TUI.
    pub frontend_semantic_highlight_max_chars: usize,
    // ── P2-3 / P2-4: frontend display budgets ──
    /// Max chars of a single tool result displayed inline in the frontend.
    /// Larger results are artifact-backed: the UI shows a preview + link,
    /// and the full content is fetched on demand.
    pub frontend_max_tool_result_display_chars: usize,
    /// Max messages rendered in TUI viewport (non-virtualized safety cap).
    pub tui_max_viewport_messages: usize,
    /// Max entries in TUI message render output cache.
    pub tui_message_output_cache_entries: usize,
    /// TUI render ops per frame before batching.
    pub tui_max_render_ops_per_frame: usize,
    /// Max depth of Web pending output-block queue.
    pub web_max_pending_output_blocks: usize,
    /// Max entries in Web side-panel selector cache.
    pub web_side_panel_cache_entries: usize,

    // ── PTY ──
    /// Max bytes buffered per PTY session.
    pub pty_buffer_limit: usize,
}

impl Default for RuntimeBudgetConfig {
    fn default() -> Self {
        Self {
            // Scheduler execution
            scheduler_max_model_calls: 32,
            scheduler_max_tool_calls: 96,
            scheduler_max_total_tokens: 1_048_576,
            scheduler_max_wall_time_ms: 1_800_000,
            scheduler_max_parallelism: 4,
            scheduler_max_graph_nodes: 48,
            scheduler_max_graph_depth: 16,
            scheduler_max_loop_iterations: 6,
            scheduler_max_agent_steps: 16,

            // Scheduler workspace capabilities
            scheduler_workspace_max_files: 10_000,
            scheduler_workspace_max_total_bytes: 1_073_741_824,
            scheduler_workspace_min_free_disk_bytes: 536_870_912,
            scheduler_workspace_operation_timeout_ms: 30_000,

            // Tool result governance
            tool_result_max_chars: 32_000,
            tool_result_preview_chars: 8_000,
            tool_batch_aggregate_max_chars: 120_000,

            // Transcript / context
            max_tool_result_transcript_chars: 32_000,
            max_attachment_bytes: 120_000,
            max_mcp_resource_chars: 12_000,

            // Streaming / connection
            stream_connection_queue_size: 128,
            max_ui_bridge_queue: 4096,
            max_events_per_frame: 256,

            // Frontend local caches
            frontend_max_prompt_history_entries: 200,
            frontend_max_stash_entries: 50,
            frontend_max_frecency_entries: 1000,
            frontend_max_transcript_lines: 1200,
            frontend_semantic_highlight_max_chars: 8_000,

            // P2-3 / P2-4: frontend display budgets
            frontend_max_tool_result_display_chars: 8_000,
            tui_max_viewport_messages: 200,
            tui_message_output_cache_entries: 300,
            tui_max_render_ops_per_frame: 256,
            web_max_pending_output_blocks: 256,
            web_side_panel_cache_entries: 100,

            // PTY
            pty_buffer_limit: 2 * 1024 * 1024,
        }
    }
}

impl RuntimeBudgetConfig {
    /// Read the budget from an optional Config.  Falls back to defaults when
    /// the config stanza is absent — this is the single authoritative read path.
    pub fn from_config(config: Option<&crate::Config>) -> Self {
        config
            .and_then(|c| c.runtime_budget.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_is_reasonable() {
        let b = RuntimeBudgetConfig::default();
        assert!(b.tool_result_max_chars > 0);
        assert!(b.tool_result_preview_chars < b.tool_result_max_chars);
        assert!(b.tool_batch_aggregate_max_chars > b.tool_result_max_chars);
        assert!(b.stream_connection_queue_size > 0);
    }

    #[test]
    fn from_config_falls_back_to_defaults_when_missing() {
        let c = crate::Config::default();
        let b = RuntimeBudgetConfig::from_config(Some(&c));
        // Should equal default since the config has no runtime_budget set.
        let d = RuntimeBudgetConfig::default();
        assert_eq!(b.tool_result_max_chars, d.tool_result_max_chars);
    }
}
