//! 土 — Per-session state authority.
//!
//! Every Signal has exactly one writer and one primary consumer.
//! SessionStore is Clone (all Signals are Copy).

use revue::prelude::*;
use crate::store::types::*;

/// Per-session state — all fields are Signals for reactive rendering.
#[derive(Clone)]
pub struct SessionStore {
    // ── 土：会话标识 ──
    pub session_id: Signal<Option<String>>,
    pub title: Signal<String>,
    pub run_status: Signal<RunStatus>,
    pub working_dir: Signal<String>,

    // ── 金：消息流（TranscriptFeed 唯一消费者）──
    pub messages: Signal<Vec<TranscriptBlock>>,

    // ── 金：对话栈（DialogLayer 唯一消费者）──
    pub dialog_stack: Signal<Vec<DialogKind>>,

    // ── 水：遥测（Sidebar 各面板独立消费）──
    pub token_usage: Signal<TokenUsage>,
    pub cache_stats: Signal<CacheStats>,
    pub pricing: Signal<Pricing>,
    pub context_pct: Signal<u8>,
    pub sidebar_trees: Signal<SidebarTrees>,
    pub mcp_lsp: Signal<McpLspInfo>,

    // ── 火：运行时 ──
    pub active_tools: Signal<Vec<ActiveTool>>,

    // ── 木：输入（ComposerPanel 唯一权威）──
    pub prompt_text: Signal<String>,
    pub prompt_mode: Signal<PromptMode>,
    pub attachments: Signal<Vec<Attachment>>,
    pub history_idx: Signal<Option<usize>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            session_id: signal(None),
            title: signal(String::from("New Session")),
            run_status: signal(RunStatus::Idle),
            working_dir: signal(String::new()),
            messages: signal(Vec::new()),
            dialog_stack: signal(Vec::new()),
            token_usage: signal(TokenUsage::default()),
            cache_stats: signal(CacheStats::default()),
            pricing: signal(Pricing::default()),
            context_pct: signal(0),
            sidebar_trees: signal(SidebarTrees::default()),
            mcp_lsp: signal(McpLspInfo::default()),
            active_tools: signal(Vec::new()),
            prompt_text: signal(String::new()),
            prompt_mode: signal(PromptMode::Normal),
            attachments: signal(Vec::new()),
            history_idx: signal(None),
        }
    }

    // ── 消息追加（金：EventBus → messages）──

    /// Append a user message block.
    pub fn push_user_message(&self, id: &str, content: &str) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::UserPrompt {
            id: id.into(), content: content.into(), folded: false,
        }));
    }

    /// Append or stream-append an assistant message.
    pub fn push_assistant_delta(&self, block_id: &str, text: &str) {
        self.messages.update(|msgs| {
            match msgs.last_mut() {
                Some(TranscriptBlock::AssistantMsg { id, content }) if id == block_id => {
                    content.push_str(text);
                }
                _ => msgs.push(TranscriptBlock::AssistantMsg {
                    id: block_id.into(), content: text.into(),
                }),
            }
        });
    }

    /// Append a thinking block.
    pub fn push_thinking(&self, id: &str, text: &str) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::Thinking {
            id: id.into(), content: text.into(), folded: true, duration_ms: 0,
        }));
    }

    /// Append or update a tool call.
    pub fn upsert_tool_call(&self, id: &str, name: &str, params: &str, phase: ToolPhase) {
        self.messages.update(|msgs| {
            for block in msgs.iter_mut() {
                if let TranscriptBlock::ToolCall { id: bid, ref mut phase, .. } = block {
                    if bid == id { *phase = phase.clone(); return; }
                }
            }
            msgs.push(TranscriptBlock::ToolCall {
                id: id.into(), name: name.into(), params: params.into(), phase,
            });
        });
    }

    /// Append a tool result.
    pub fn push_tool_result(&self, id: &str, name: &str, result: &str, is_error: bool) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::ToolResult {
            id: id.into(), name: name.into(), result: result.into(), is_error, folded: false,
        }));
    }

    /// Append a stage update.
    pub fn push_stage(&self, id: &str, name: &str, status: &str) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::StageUpdate {
            id: id.into(), name: name.into(), status: status.into(),
        }));
    }

    /// Append a skill activation notice.
    pub fn push_skill(&self, id: &str, name: &str) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::SkillActivated {
            id: id.into(), name: name.into(),
        }));
    }

    /// Append a compaction hint.
    pub fn push_compaction(&self, id: &str, before: u64, after: u64) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::CompactionHint {
            id: id.into(), before_tokens: before, after_tokens: after,
        }));
    }

    /// Append a system notice.
    pub fn push_notice(&self, id: &str, text: &str) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::SystemNotice {
            id: id.into(), text: text.into(),
        }));
    }

    // ── 消息折叠 ──

    pub fn toggle_fold(&self, block_idx: usize) {
        self.messages.update(|msgs| {
            if let Some(block) = msgs.get_mut(block_idx) {
                match block {
                    TranscriptBlock::UserPrompt { ref mut folded, .. }
                    | TranscriptBlock::Thinking { ref mut folded, .. }
                    | TranscriptBlock::ToolResult { ref mut folded, .. } => *folded = !*folded,
                    _ => {}
                }
            }
        });
    }

    // ── 水：遥测更新（EventBus → Signals）──

    pub fn set_token_usage(&self, input: u64, output: u64,
                           cache_read: u64, cache_miss: u64, cache_write: u64) {
        self.token_usage.set(TokenUsage {
            input, output, total: input + output,
            cache_read, cache_miss, cache_write,
        });
    }

    pub fn set_cache_stats(&self, hits: u64, misses: u64, writes: u64) {
        self.cache_stats.set(CacheStats { hits, misses, writes });
    }

    pub fn set_pricing(&self, input_per_1k: f64, output_per_1k: f64) {
        self.pricing.set(Pricing { input_per_1k, output_per_1k, total: 0.0 });
    }

    pub fn set_context_pct(&self, pct: u8) {
        self.context_pct.set(pct.min(100));
    }

    pub fn set_mcp_lsp(&self, mcp_connected: usize, mcp_total: usize, lsp_active: Vec<String>) {
        self.mcp_lsp.set(McpLspInfo { mcp_connected, mcp_total, lsp_active });
    }

    // ── 火：运行时 ──

    pub fn set_active_tool(&self, id: &str, name: &str, phase: ToolPhase) {
        self.active_tools.update(|tools| {
            tools.retain(|t| t.id != id);
            tools.push(ActiveTool { id: id.into(), name: name.into(), phase });
        });
    }

    pub fn remove_active_tool(&self, id: &str) {
        self.active_tools.update(|tools| tools.retain(|t| t.id != id));
    }

    // ── 木：输入 ──

    pub fn set_prompt_text(&self, text: &str) {
        self.prompt_text.set(text.to_string());
    }

    pub fn clear_prompt(&self) {
        self.prompt_text.set(String::new());
        self.attachments.set(Vec::new());
        self.prompt_mode.set(PromptMode::Normal);
    }

    pub fn set_prompt_mode(&self, mode: PromptMode) {
        self.prompt_mode.set(mode);
    }

    pub fn add_attachment(&self, attachment: Attachment) {
        self.attachments.update(|a| a.push(attachment));
    }

    pub fn clear_attachments(&self) {
        self.attachments.set(Vec::new());
    }

    // ── Session ID ──

    pub fn set_session_id(&self, id: &str) {
        self.session_id.set(Some(id.to_string()));
    }

    pub fn get_session_id(&self) -> Option<String> {
        self.session_id.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_store_is_idle_empty() {
        let s = SessionStore::new();
        assert_eq!(s.run_status.get(), RunStatus::Idle);
        assert!(s.messages.get().is_empty());
    }

    #[test]
    fn push_user_message() {
        let s = SessionStore::new();
        s.push_user_message("u1", "hello");
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::UserPrompt { content, .. } => assert_eq!(content, "hello"),
            _ => panic!("expected UserPrompt"),
        }
    }

    #[test]
    fn assistant_delta_accumulates() {
        let s = SessionStore::new();
        s.push_assistant_delta("b1", "a");
        s.push_assistant_delta("b1", "b");
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            TranscriptBlock::AssistantMsg { content, .. } => assert_eq!(content, "ab"),
            _ => panic!("expected AssistantMsg"),
        }
    }

    #[test]
    fn assistant_delta_new_block_on_different_id() {
        let s = SessionStore::new();
        s.push_assistant_delta("b1", "a");
        s.push_assistant_delta("b2", "b");
        assert_eq!(s.messages.get().len(), 2);
    }

    #[test]
    fn upsert_tool_call_updates_phase() {
        let s = SessionStore::new();
        s.upsert_tool_call("t1", "bash", "ls", ToolPhase::Starting);
        s.upsert_tool_call("t1", "bash", "ls", ToolPhase::Running);
        s.upsert_tool_call("t1", "bash", "ls", ToolPhase::Done);
        assert_eq!(s.messages.get().len(), 1); // same tool, no new block
    }

    #[test]
    fn toggle_fold() {
        let s = SessionStore::new();
        s.push_user_message("u1", "long content");
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::UserPrompt { folded, .. } => assert!(*folded),
            _ => panic!(),
        }
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::UserPrompt { folded, .. } => assert!(!*folded),
            _ => panic!(),
        }
    }

    #[test]
    fn token_usage_update() {
        let s = SessionStore::new();
        s.set_token_usage(100, 50, 10, 5, 3);
        let usage = s.token_usage.get();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 50);
        assert_eq!(usage.cache_read, 10);
    }

    #[test]
    fn clear_prompt_resets_input_state() {
        let s = SessionStore::new();
        s.set_prompt_text("hello");
        s.add_attachment(Attachment { name: "f".into(), kind: AttachmentKind::File { path: "p".into(), lines: 10 } });
        s.set_prompt_mode(PromptMode::Shell);
        s.clear_prompt();
        assert!(s.prompt_text.get().is_empty());
        assert!(s.attachments.get().is_empty());
        assert_eq!(s.prompt_mode.get(), PromptMode::Normal);
    }

    #[test]
    fn context_pct_clamps_at_100() {
        let s = SessionStore::new();
        s.set_context_pct(150);
        assert_eq!(s.context_pct.get(), 100);
    }
}
