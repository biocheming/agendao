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

    /// Number of rendered rows to scroll back from the latest. 0 = pinned
    /// to the bottom (default — newest content visible). Higher values
    /// shift the visible window earlier in the transcript so users can
    /// re-read history. Updated by mouse wheel and PageUp/PageDown.
    pub scroll_offset: Signal<u16>,

    /// Index of the transcript block currently under the keyboard
    /// cursor. The cursor moves with j/k (vim) and is the target of
    /// Space (toggle fold). When `None`, no block is selected — typical
    /// when the user is composing in the prompt and hasn't tabbed into
    /// the transcript yet. Rendering can paint a left-bar accent on
    /// the cursor block to indicate focus.
    pub transcript_cursor: Signal<Option<usize>>,

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
            scroll_offset: signal(0),
            transcript_cursor: signal(None),
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
            id: id.into(), content: content.into(), fold: FoldState::Truncated,
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

    /// Append a thinking block, or extend the most-recent reasoning block
    /// when the id matches. Without this delta-aware merge, every
    /// reasoning chunk from the LLM stream appended a NEW thinking row,
    /// turning a single chain-of-thought into dozens of single-character
    /// blocks in the transcript.
    pub fn push_thinking(&self, id: &str, text: &str) {
        self.messages.update(|msgs| {
            if let Some(TranscriptBlock::Thinking { id: bid, content, .. }) = msgs.last_mut() {
                if bid == id {
                    content.push_str(text);
                    return;
                }
            }
            msgs.push(TranscriptBlock::Thinking {
                id: id.into(), content: text.into(), fold: FoldState::Truncated, duration_ms: 0,
            });
        });
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
    ///
    /// Defaults to `fold: FoldState::Truncated` because tool outputs are usually long
    /// (a websearch dump can be thousands of characters) and would push
    /// every other transcript block off the screen. Users can expand
    /// individual results when they need full detail.
    pub fn push_tool_result(&self, id: &str, name: &str, result: &str, is_error: bool) {
        self.messages.update(|msgs| {
            let block = TranscriptBlock::ToolResult {
                id: id.into(), name: name.into(), result: result.into(), is_error, fold: FoldState::Truncated,
            };
            // 插到对应 ToolCall 之后（同 tool_call_id），让调用与结果紧邻配对显示，
            // 而非 append 末尾——避免 LLM 并行发起多个 tool 时调用与结果割裂
            // （先一串 call、很久后一串 result）。找不到对应 ToolCall（事件乱序
            // 等异常）时 fallback append 末尾，保证结果不丢。ToolCall 与 ToolResult
            // 同 id 共存不冲突：fold/phase 查找均按 block 类型过滤。
            let pos = msgs.iter().rposition(|b| {
                matches!(b, TranscriptBlock::ToolCall { id: bid, .. } if bid == id)
            });
            match pos {
                Some(i) => msgs.insert(i + 1, block),
                None => msgs.push(block),
            }
        });
    }

    /// Append a stage update with optional JSON metadata.
    pub fn push_stage(&self, id: &str, name: &str, status: &str, metadata: Option<String>) {
        self.messages.update(|msgs| msgs.push(TranscriptBlock::StageUpdate {
            id: id.into(), name: name.into(), status: status.into(), metadata,
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

    /// Push or update a todo list block.  Deduplicates by `block_id`:
    /// replaces the last TodoList with the same id, otherwise appends.
    pub fn push_todo_list(
        &self,
        block_id: &str,
        items: Vec<crate::store::types::TodoItem>,
        summary: Option<crate::store::types::TodoSummary>,
    ) {
        self.messages.update(|msgs| {
            // Replace existing TodoList with same id, or append
            if let Some(TranscriptBlock::TodoList { id, .. }) = msgs.last() {
                if id == block_id {
                    if let Some(TranscriptBlock::TodoList { items: ref mut old, .. }) = msgs.last_mut() {
                        *old = items;
                        return;
                    }
                }
            }
            msgs.push(TranscriptBlock::TodoList {
                id: block_id.into(),
                items,
                fold: FoldState::Truncated,
                summary,
            });
        });
    }

    // ── 消息折叠 ──

    pub fn toggle_fold(&self, block_idx: usize) {
        // Cycle through FoldState: Folded → Truncated → Expanded → Folded.
        let mut new_msgs: Vec<TranscriptBlock> = self.messages.get();
        if let Some(block) = new_msgs.get_mut(block_idx) {
            match block {
                TranscriptBlock::UserPrompt { ref mut fold, .. }
                | TranscriptBlock::Thinking { ref mut fold, .. }
                | TranscriptBlock::ToolResult { ref mut fold, .. }
                | TranscriptBlock::TodoList { ref mut fold, .. } => *fold = fold.next(),
                _ => {}
            }
        }
        self.messages.set(new_msgs);
    }

    // ── 水：遥测更新（EventBus → Signals）──

    pub fn set_token_usage(&self, input: u64, output: u64, reasoning: u64,
                           cache_read: u64, cache_miss: u64, cache_write: u64,
                           context_tokens: u64, total_cost: f64) {
        self.token_usage.set(TokenUsage {
            input, output, reasoning, total: input + output + reasoning,
            cache_read, cache_miss, cache_write,
            context_tokens, total_cost,
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

    // ── Scroll: anchored to the latest by default (offset = 0). ──
    //
    // Larger offset shifts the visible window UP into older messages,
    // so `scroll_up` (mouse wheel up / PageUp) increases offset, and
    // `scroll_down` decreases it. Newly arrived messages auto-pin to
    // the bottom only when offset is 0 — once the user scrolled up to
    // read history, incoming events should not yank them back to the
    // bottom mid-read. The renderer caps offset at total transcript
    // height so we don't slide past the start.

    pub fn scroll_up(&self) {
        self.scroll_offset.update(|o| *o = o.saturating_add(3));
    }

    pub fn scroll_down(&self) {
        self.scroll_offset.update(|o| *o = o.saturating_sub(3));
    }

    pub fn scroll_page_up(&self, page: u16) {
        self.scroll_offset.update(|o| *o = o.saturating_add(page));
    }

    pub fn scroll_page_down(&self, page: u16) {
        self.scroll_offset.update(|o| *o = o.saturating_sub(page));
    }

    pub fn scroll_to_bottom(&self) {
        self.scroll_offset.set(0);
    }

    // ── Transcript cursor & fold ──
    //
    // The cursor is what `Space` / `Enter` operate on inside the
    // transcript. Moving the cursor with j/k auto-scrolls so the
    // cursor row stays in view (mirroring how vim handles long files).

    /// Move cursor to the previous foldable block, wrapping at the top.
    /// "Foldable" today means UserPrompt / Thinking / ToolResult — the
    /// only blocks whose `toggle_fold` actually flips state. Cursor
    /// skips assistant text and tool-call rows since fold is a no-op
    /// for those.
    pub fn cursor_prev_foldable(&self) {
        let msgs = self.messages.get();
        let mut idx = self.transcript_cursor.get().unwrap_or(msgs.len());
        loop {
            if idx == 0 { idx = msgs.len(); }
            if idx == 0 { return; }
            idx -= 1;
            if Self::is_foldable(&msgs[idx]) { break; }
        }
        self.transcript_cursor.set(Some(idx));
    }

    pub fn cursor_next_foldable(&self) {
        let msgs = self.messages.get();
        if msgs.is_empty() { return; }
        let mut idx = self.transcript_cursor.get().map(|i| i + 1).unwrap_or(0);
        let start = idx;
        loop {
            if idx >= msgs.len() { idx = 0; }
            if Self::is_foldable(&msgs[idx]) {
                self.transcript_cursor.set(Some(idx));
                return;
            }
            idx += 1;
            // Loop guard: if we walked the whole list back to the start.
            if idx == start { return; }
        }
    }

    fn is_foldable(block: &TranscriptBlock) -> bool {
        matches!(
            block,
            TranscriptBlock::UserPrompt { .. }
            | TranscriptBlock::Thinking { .. }
            | TranscriptBlock::ToolResult { .. }
        )
    }

    /// Top row (from the beginning of the content) where the block
    /// under the cursor lives. Each block occupies its `height()` rows
    /// plus a 1-row gap (matching the `vstack().gap(1)` the renderer
    /// uses). Returns 0 if no cursor is set.
    pub fn cursor_top_row(&self) -> u16 {
        let Some(cursor) = self.transcript_cursor.get() else { return 0 };
        let msgs = self.messages.get();
        msgs.iter()
            .take(cursor)
            .map(|b| b.height().saturating_add(1))
            .sum()
    }

    /// Total content height (Σ block heights + gaps + trailing newline).
    /// Matches the `total_h` formula in `RootView::render` so the
    /// cursor math and the renderer math agree.
    pub fn total_transcript_height(&self) -> u16 {
        let msgs = self.messages.get();
        msgs.iter()
            .map(|b| b.height().saturating_add(1))
            .sum::<u16>()
            .saturating_add(1)
    }

    /// Adjust `scroll_offset` so the cursor block sits inside the
    /// visible viewport. No-op if the cursor is already in view.
    ///
    /// `viewport_h` is the height of the transcript area in rows.
    /// The store's `scroll_offset` counts "rows back from the
    /// bottom" — 0 = pinned to the newest message, growing = earlier
    /// content. We compute where the cursor's top row is in the
    /// renderer's coordinate space (`scroll_top = max_offset - offset`)
    /// and shift the offset so the cursor lands somewhere in the
    /// upper third of the viewport (mirroring how vim's `zz` recenter
    /// works after a jump).
    pub fn ensure_cursor_visible(&self, viewport_h: u16) {
        let Some(cursor) = self.transcript_cursor.get() else { return };
        if cursor == 0 { return; } // first block always visible at scroll_top=0
        let total = self.total_transcript_height();
        if total <= viewport_h { return; } // everything fits, nothing to scroll
        let max_offset = total.saturating_sub(viewport_h);
        let user_offset = self.scroll_offset.get().min(max_offset);
        let scroll_top = max_offset.saturating_sub(user_offset);
        let cursor_top = self.cursor_top_row();
        let cursor_bottom = cursor_top.saturating_add(self.messages.get()[cursor].height());
        // Pad so the cursor doesn't sit on the very top or bottom edge.
        let pad: u16 = 2;
        let view_top = scroll_top;
        let view_bottom = scroll_top.saturating_add(viewport_h);
        if cursor_top >= view_top.saturating_add(pad) && cursor_bottom + pad <= view_bottom {
            return; // already in view
        }
        // Target: place the cursor's TOP at scroll_top + pad.
        // Convert to user_offset = max_offset - scroll_top.
        let new_scroll_top = cursor_top.saturating_sub(pad);
        let new_user_offset = max_offset.saturating_sub(new_scroll_top);
        self.scroll_offset.set(new_user_offset);
    }

    /// Toggle fold on the block under the cursor (or the latest
    /// foldable block when no cursor is set yet — matches the user's
    /// "I just want to expand the last result" mental model).
    pub fn toggle_fold_at_cursor(&self) {
        let mut idx = self.transcript_cursor.get();
        if idx.is_none() {
            // Find the most recent foldable block.
            let msgs = self.messages.get();
            for i in (0..msgs.len()).rev() {
                if Self::is_foldable(&msgs[i]) { idx = Some(i); break; }
            }
        }
        if let Some(i) = idx {
            self.toggle_fold(i);
            self.transcript_cursor.set(Some(i));
        }
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
    fn tool_result_inserts_right_after_its_call() {
        // 并行 5 个 read：5 个 ToolCall 先入列，done 后每个 ToolResult 应紧跟
        // 各自的 ToolCall（配对相邻），而非全 append 末尾造成调用与结果割裂。
        let s = SessionStore::new();
        for i in 1..=5 {
            s.upsert_tool_call(&format!("t{i}"), "read", &format!("f{i}"), ToolPhase::Starting);
        }
        for i in 1..=5 {
            s.upsert_tool_call(&format!("t{i}"), "read", "", ToolPhase::Done);
            s.push_tool_result(&format!("t{i}"), "read", &format!("out{i}"), false);
        }
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 10);
        // 期望顺序：TC1, TR1, TC2, TR2, …, TC5, TR5
        for i in 0..5 {
            match (&msgs[i * 2], &msgs[i * 2 + 1]) {
                (TranscriptBlock::ToolCall { id: cid, .. },
                 TranscriptBlock::ToolResult { id: rid, result, .. }) => {
                    assert_eq!(cid, rid, "pair {} id mismatch", i);
                    assert_eq!(result, &format!("out{}", i + 1));
                }
                other => panic!("expected (ToolCall, ToolResult) at pair {i}, got {other:?}"),
            }
        }
    }

    #[test]
    fn tool_result_without_call_appends() {
        // 找不到对应 ToolCall（事件乱序）时 fallback append 末尾，不丢结果。
        let s = SessionStore::new();
        s.push_tool_result("orphan", "read", "out", false);
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0], TranscriptBlock::ToolResult { .. }));
    }

    /// 统计 messages 中 Thinking block 的数量。
    fn count_thinking(msgs: &[TranscriptBlock]) -> usize {
        msgs.iter()
            .filter(|b| matches!(b, TranscriptBlock::Thinking { .. }))
            .count()
    }

    #[test]
    fn thinking_accumulates_consecutive_same_id() {
        // 同 id 的连续 reasoning delta 应累积成单个 Thinking block
        // —— push_thinking 的 last_mut + bid==id merge 正是为此而设计。
        let s = SessionStore::new();
        s.push_thinking("m1", "step 1 ");
        s.push_thinking("m1", "step 2 ");
        s.push_thinking("m1", "step 3");
        let msgs = s.messages.get();
        assert_eq!(count_thinking(&msgs), 1, "consecutive same-id reasoning must merge into one Thinking");
        match &msgs[0] {
            TranscriptBlock::Thinking { content, .. } => assert_eq!(content, "step 1 step 2 step 3"),
            _ => panic!("expected Thinking"),
        }
    }

    #[test]
    fn reasoning_after_assistant_keeps_separate_thinking() {
        // 有意设计（保留分段）：reasoning 与 assistant text 在同一 message 内
        // 交替（reasoning → text → reasoning，同 id）时，push_thinking 只检查
        // last_mut，中间插入 AssistantMsg 后下一次 reasoning 新建独立 Thinking——
        // 保留「哪段思考夹在哪段输出之间」的时序对应。视觉上的连续感由渲染层
        // 的 ┆ 续接符（layout_block_ctx）处理，数据层不合并。
        let s = SessionStore::new();
        s.push_thinking("m1", "先思考");
        s.push_assistant_delta("m1", "先输出一部分");
        s.push_thinking("m1", "再继续思考");
        let msgs = s.messages.get();
        assert_eq!(count_thinking(&msgs), 2,
            "interleaved assistant delta keeps reasoning as 2 separate Thinking blocks");
    }

    #[test]
    fn reasoning_after_tool_keeps_separate_thinking() {
        // 有意设计（保留分段）：reasoning → tool → reasoning（同 id）也因 last_mut
        // 是 ToolCall 而保持独立 Thinking——保留思考与工具的时序对应。reasoning
        // model（思考→工具→再思考）频繁触发，渲染层用 ┆ 续接符表明连续。
        let s = SessionStore::new();
        s.push_thinking("m1", "思考阶段一");
        s.upsert_tool_call("t1", "read", "f.txt", ToolPhase::Done);
        s.push_thinking("m1", "思考阶段二");
        let msgs = s.messages.get();
        assert_eq!(count_thinking(&msgs), 2,
            "interleaved tool call keeps reasoning as 2 separate Thinking blocks");
    }

    #[test]
    fn thinking_different_id_creates_separate() {
        // 不同 id 的 reasoning 天然是独立 Thinking（多 message / 多 reasoning 周期）。
        let s = SessionStore::new();
        s.push_thinking("m1", "第一轮思考");
        s.push_thinking("m2", "第二轮思考");
        let msgs = s.messages.get();
        assert_eq!(count_thinking(&msgs), 2, "different-id reasoning yields separate Thinking blocks");
    }

    #[test]
    fn toggle_fold() {
        let s = SessionStore::new();
        s.push_user_message("u1", "long content");
        // Default is Truncated → toggle to Expanded
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::UserPrompt { fold, .. } => assert_eq!(*fold, FoldState::Expanded),
            _ => panic!(),
        }
        // Expanded → toggle to Folded
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::UserPrompt { fold, .. } => assert_eq!(*fold, FoldState::Folded),
            _ => panic!(),
        }
        // Folded → toggle to Truncated
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::UserPrompt { fold, .. } => assert_eq!(*fold, FoldState::Truncated),
            _ => panic!(),
        }
    }

    #[test]
    fn token_usage_update() {
        let s = SessionStore::new();
        s.set_token_usage(100, 50, 20, 10, 5, 3, 2000, 0.015);
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
