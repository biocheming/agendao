//! 土 — Per-session state authority.
//!
//! Every Signal has exactly one writer and one primary consumer.
//! SessionStore is Clone (all Signals are Copy).

use crate::store::types::*;
use revue::prelude::*;

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

    /// U11①：上一帧渲染的 transcript 总高（render 单点回写）。流式
    /// chunk/新块把内容底部长高 Δ 时，若用户翻上去阅读（offset>0），
    /// offset 同步 +Δ——锚的是内容坐标，不是"距底行数"，正在读的行
    /// 视觉位置不动。
    pub scroll_anchor_last_h: Signal<u16>,

    /// U11④：未读记账——offset==0（在底）时对齐为当前块数；翻上去
    /// 期间 messages 长尾多出的块数即 status bar "↓ N new"。
    pub unread_seen_len: Signal<usize>,

    /// Index of the transcript block currently under the keyboard
    /// cursor. The cursor moves with j/k (vim) and is the target of
    /// Space (toggle fold). When `None`, no block is selected — typical
    /// when the user is composing in the prompt and hasn't tabbed into
    /// the transcript yet. Rendering can paint a left-bar accent on
    /// the cursor block to indicate focus.
    pub transcript_cursor: Signal<Option<usize>>,

    // ── 水：遥测（Sidebar 各面板独立消费）──
    pub token_usage: Signal<TokenUsage>,
    pub context_pct: Signal<u8>,
    /// 上下文窗口分母（token）。两条写入路径同口径：投影事件
    /// `ContextCompactionSummary.limit_tokens`（权威）与 `apply_session_open`
    /// 从 provider 模型元数据 `context_window` 播种。None = 分母未知
    /// （sidebar Window 显示 `-`，info strip 不拼 `/limit` 尾）。
    pub context_limit: Signal<Option<u64>>,
    /// 会话实际使用的模型（优先来自 session metadata，旧会话回退到最后一条
    /// assistant 消息的 `model` 字段，通常为 `provider/model` 形式）。
    pub session_model: Signal<Option<String>>,
    /// 会话实际使用的 agent；来源与 `session_model` 相同。
    pub session_agent: Signal<Option<String>>,
    pub sidebar_trees: Signal<SidebarTrees>,
    pub mcp_lsp: Signal<McpLspInfo>,
    /// Task-governance ledger snapshot (Phase 5 renders it; the event
    /// handler is the only writer).
    pub task_ledger:
        Signal<Option<std::sync::Arc<agendao_types::task_ledger::SessionTaskLedgerView>>>,

    /// 流式分段的待决集合：`kind:id` 在 `start` 时登记，`full` 时取出。
    /// 见 `apply_assistant_snapshot` 的两种线上 `full` 形态说明。不参与渲染，
    /// 仅作事件流状态（writer = telemetry::event_handler）。
    pub stream_new_segments: Signal<std::collections::HashSet<String>>,
    /// M8 segment-level lifecycle fence. Wire `end` can terminate a chunk
    /// segment (not necessarily a logical turn), so a later `start` reopens
    /// this identity. It prevents duplicate/out-of-order deltas between end
    /// and the next start; it does not claim logical-message finality.
    pub finalized_stream_blocks: Signal<std::collections::HashSet<String>>,

    /// 会话级 diff 汇总（`FrontendEvent::DiffReplaced`，replace 语义——每轮
    /// 结束下发的全量集合直接替换，不累加）。空 Vec = 无未决改动（角标隐藏）。
    pub diff_summary: Signal<Vec<DiffStat>>,
    /// diff 角标逐文件明细展开态（点击角标 toggle；writer = keymap 鼠标）。
    pub diff_detail_open: Signal<bool>,
    /// M7 per-session details policy; never shared across session switches.
    pub details_policy: Signal<crate::details_policy::DetailsPolicy>,
    /// Last authoritative topology summary. `None` means no snapshot seen.
    pub topology_summary: Signal<Option<TopologySummary>>,
    /// Authoritative M9 subagent projection; replaced by topology snapshots.
    pub subagent_projection: Signal<Option<crate::subagent_panel::SubagentPanelProjection>>,

    // ── 火：运行时 ──
    pub active_tools: Signal<Vec<ActiveTool>>,
    pub active_turn_id: Signal<Option<String>>,
    /// 本次运行的起点（Reasonix "Working Ns" 模式）：进入运行态置位、
    /// 回 Idle/WaitingUser/Error 清零。None = 未在运行。
    pub running_since: Signal<Option<std::time::Instant>>,

    // ── §8 可观测性：活跃 sandbox 执行集合 ──
    /// 唯一权威来源是 `sandbox.execution.upsert/removed` 事件与
    /// runtime 快照（replaced 全量替换、事件增量维护）。任何
    /// "sandboxed" 展示只能以此为据——没有条目的执行不得渲染成
    /// 已沙箱（writer = telemetry::event_handler）。
    pub active_sandboxes: Signal<Vec<agendao_client::SandboxExecutionSummary>>,

    // ── 木：输入附面（文本/history 的唯一权威是 `input::PromptInput`；
    //    此处只承载待发附件，由 keymap 写入、RootView 附件条渲染消费）──
    pub attachments: Signal<Vec<Attachment>>,

    // ── M2.5：只读 Shadow Projection 协调器（不驱动 UI，仅用于协议与一致性比对）──
    pub shadow_coordinator:
        std::sync::Arc<crate::store::projection_coordinator::ProjectionCoordinator>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
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
            scroll_anchor_last_h: signal(0),
            unread_seen_len: signal(0),
            transcript_cursor: signal(None),
            token_usage: signal(TokenUsage::default()),
            context_pct: signal(0),
            context_limit: signal(None),
            session_model: signal(None),
            session_agent: signal(None),
            sidebar_trees: signal(SidebarTrees::default()),
            mcp_lsp: signal(McpLspInfo::default()),
            task_ledger: Signal::new(None),
            active_tools: signal(Vec::new()),
            active_sandboxes: signal(Vec::new()),
            active_turn_id: signal(None),
            running_since: signal(None),
            attachments: signal(Vec::new()),
            shadow_coordinator: std::sync::Arc::new(
                crate::store::projection_coordinator::ProjectionCoordinator::new(),
            ),
            stream_new_segments: signal(std::collections::HashSet::new()),
            finalized_stream_blocks: signal(std::collections::HashSet::new()),
            diff_summary: signal(Vec::new()),
            diff_detail_open: signal(false),
            details_policy: signal(crate::details_policy::DetailsPolicy::default()),
            topology_summary: signal(None),
            subagent_projection: signal(None),
        }
    }

    /// 重置到新会话初始态:清空当前 session 的消息/状态/遥测/工具,保留
    /// working_dir(创建新 session 需要)与 mcp_lsp/sidebar_trees(环境信息)。
    /// 用于 /new 和 dispatch 在 Home 路由创建新 session——若不重置,
    /// push_user_message 会追加到上一个 session 残留的 messages 上,
    /// 造成"新 session 接着旧 session 显示"的数据错位(土:状态唯一所有权,
    /// 新会话不能携带旧会话状态)。
    pub fn reset_for_new_session(&self) {
        self.task_ledger.set(None);
        self.session_id.set(None);
        self.active_turn_id.set(None);
        self.title.set(String::from("New Session"));
        self.set_run_status(RunStatus::Idle);
        self.messages.update(|m| m.clear());
        self.scroll_offset.set(0);
        self.scroll_anchor_last_h.set(0);
        self.unread_seen_len.set(0);
        self.transcript_cursor.set(None);
        self.token_usage.set(TokenUsage::default());
        self.context_pct.set(0);
        self.context_limit.set(None);
        self.session_model.set(None);
        self.session_agent.set(None);
        self.active_tools.update(|t| t.clear());
        self.stream_new_segments.update(|s| s.clear());
        self.finalized_stream_blocks.update(|s| s.clear());
        self.diff_summary.update(|d| d.clear());
        self.diff_detail_open.set(false);
        self.details_policy
            .set(crate::details_policy::DetailsPolicy::default());
        self.topology_summary.set(None);
        self.subagent_projection.set(None);
        // 新会话重置 shadow 协调器，防止旧会话数据污染
        self.shadow_coordinator.reset_session();
    }

    pub fn clear_details_session_overrides(&self) {
        self.details_policy
            .update(|policy| policy.instance_overrides.clear());
    }

    pub fn set_topology_summary(&self, summary: TopologySummary) {
        self.topology_summary.set(Some(summary));
    }

    pub fn clear_topology_summary(&self) {
        self.topology_summary.set(None);
    }

    /// M9 replacement fence. Topology is a server snapshot, never a local
    /// counter: a different session, an older revision, or a second
    /// unversioned update is rejected rather than guessed about.
    pub fn replace_subagent_projection(
        &self,
        topology: &agendao_api::SessionExecutionTopology,
    ) -> bool {
        if self.session_id.get().as_deref() != Some(topology.session_id.as_str()) {
            return false;
        }
        let next = crate::subagent_panel::SubagentPanelProjection::from_topology(topology);
        if let Some(current) = self.subagent_projection.get() {
            match (current.topology_updated_at, next.topology_updated_at) {
                (Some(current_ts), Some(next_ts)) if next_ts < current_ts => return false,
                (Some(current_ts), Some(next_ts))
                    if next_ts == current_ts
                        && current.topology_fingerprint != next.topology_fingerprint =>
                {
                    return false
                }
                (Some(_), None) => return false,
                (None, None) if current.topology_fingerprint != next.topology_fingerprint => {
                    return false
                }
                _ => {}
            }
        }
        self.subagent_projection.set(Some(next));
        true
    }

    pub fn subagent_count(&self) -> Option<usize> {
        self.topology_summary.get().and_then(|s| s.subagents)
    }

    pub fn running_tool_count(&self) -> usize {
        self.topology_summary
            .get()
            .map(|s| s.running_tools)
            .unwrap_or(0)
    }

    /// Apply a server-authoritative ledger snapshot for the active session.
    /// All event and reconciliation paths use this one revision guard so a
    /// delayed fetch or out-of-order event cannot roll the UI backward.
    pub fn apply_task_ledger_snapshot(
        &self,
        ledger: agendao_types::task_ledger::SessionTaskLedgerView,
    ) -> bool {
        if self.session_id.get().as_deref() != Some(ledger.session_id.as_str())
            || ledger.revision == 0
        {
            return false;
        }
        let newer = self
            .task_ledger
            .get()
            .as_ref()
            .map(|current| current.revision < ledger.revision)
            .unwrap_or(true);
        if newer {
            self.task_ledger.set(Some(std::sync::Arc::new(ledger)));
        }
        newer
    }

    // ── 消息追加（金：EventBus → messages）──

    /// Append a user message block.
    pub fn push_user_message(&self, id: &str, content: &str) {
        self.messages.update(|msgs| {
            close_trailing_thinking(msgs);
            msgs.push(TranscriptBlock::UserPrompt {
                id: id.into(),
                content: content.into(),
                fold: FoldState::Truncated,
                failed: false,
            })
        });
    }

    /// 回收一条乐观 push 的 user message（发送失败时由 `Event::Tick` drain 调用）。
    ///
    /// 按 id 精确匹配；未命中（已被事件流覆盖等）则 no-op，幂等。与
    /// `push_user_message` 对称，闭合"push 即承诺 remove"的生命周期 —— 不留
    /// 一条"幽灵 user prompt"误导用户以为已发送。
    /// 发送失败打标（Reasonix msg--user-failed）：保留原文 + failed 标记。
    pub fn mark_user_message_failed(&self, id: &str) {
        self.messages.update(|msgs| {
            for block in msgs.iter_mut() {
                if let TranscriptBlock::UserPrompt {
                    id: bid, failed, ..
                } = block
                {
                    if bid == id {
                        *failed = true;
                        return;
                    }
                }
            }
        });
    }

    /// Append or stream-append an assistant message.
    pub fn push_assistant_delta(&self, block_id: &str, text: &str) {
        if self
            .finalized_stream_blocks
            .get()
            .contains(&format!("m:{block_id}"))
        {
            return;
        }
        self.messages.update(|msgs| match msgs.last_mut() {
            Some(TranscriptBlock::AssistantMsg { id, content, .. }) if id == block_id => {
                content.push_str(text);
            }
            _ => {
                close_trailing_thinking(msgs);
                msgs.push(TranscriptBlock::AssistantMsg {
                    id: block_id.into(),
                    content: text.into(),
                    lifecycle: StreamBlockLifecycle::Streaming,
                    fold: FoldState::Truncated,
                })
            }
        });
    }

    /// phase="start"：登记"下一条同 key 的 full 起新段"（追加而非合并）。
    /// kind 用短前缀区分流：m = assistant message，r = reasoning。
    pub fn mark_stream_segment_start(&self, kind: &str, block_id: &str) {
        let key = format!("{kind}:{block_id}");
        // A new wire segment may legally reuse the id after a prior
        // start/full/end chunk lifecycle; reopen the id before accepting it.
        self.finalized_stream_blocks.update(|s| {
            s.remove(&key);
        });
        self.messages.update(|msgs| {
            for block in msgs.iter_mut() {
                match (kind, block) {
                    ("m", TranscriptBlock::AssistantMsg { id, lifecycle, .. })
                        if id == block_id =>
                    {
                        *lifecycle = StreamBlockLifecycle::Streaming
                    }
                    ("r", TranscriptBlock::Thinking { id, lifecycle, .. }) if id == block_id => {
                        *lifecycle = StreamBlockLifecycle::Streaming
                    }
                    _ => {}
                }
            }
        });
        self.stream_new_segments.update(|s| {
            s.insert(key);
        });
    }

    /// 取出并清除分段标记（一次性）。
    fn take_stream_segment(&self, kind: &str, block_id: &str) -> bool {
        let key = format!("{kind}:{block_id}");
        let marked = self.stream_new_segments.get().contains(&key);
        if marked {
            self.stream_new_segments.update(|s| {
                s.remove(&key);
            });
        }
        marked
    }

    pub fn finalize_stream_block(&self, kind: &str, block_id: &str) {
        let key = format!("{kind}:{block_id}");
        self.finalized_stream_blocks.update(|s| {
            s.insert(key.clone());
        });
        self.stream_new_segments.update(|s| {
            s.remove(&key);
        });
        self.messages.update(|msgs| {
            for block in msgs.iter_mut() {
                match (kind, block) {
                    ("m", TranscriptBlock::AssistantMsg { id, lifecycle, .. })
                        if id == block_id =>
                    {
                        *lifecycle = StreamBlockLifecycle::Finalized
                    }
                    ("r", TranscriptBlock::Thinking { id, lifecycle, .. }) if id == block_id => {
                        *lifecycle = StreamBlockLifecycle::Finalized
                    }
                    _ => {}
                }
            }
        });
    }

    pub fn stream_block_lifecycle(&self, kind: &str, block_id: &str) -> StreamBlockLifecycle {
        if self.stream_block_is_closed(kind, block_id) {
            StreamBlockLifecycle::Finalized
        } else {
            StreamBlockLifecycle::Streaming
        }
    }

    /// Explicit lifecycle vocabulary for consumers/tests. This reports the
    /// stream segment state only; no terminal turn authority is inferred.
    pub fn stream_block_is_closed(&self, kind: &str, block_id: &str) -> bool {
        self.finalized_stream_blocks
            .get()
            .contains(&format!("{kind}:{block_id}"))
    }

    /// Merge a `full`-phase snapshot into the running assistant block.
    ///
    /// 线上 `full` 块有两种真实形态（实测 wire 取证）：
    ///   1. **单生命周期累积流**：一次 `start` 后逐帧 `full`（累积全文，经
    ///      local frontend receiver coalesce 归并 delta 而来）——段内后续 `full` 必须
    ///      按快照合并（前缀替换），否则追加出 "TheThe answer to..."（Bug B）。
    ///   2. **逐 chunk 生命周期**（deepseek-v4-flash / qwen 实测）：每个 chunk
    ///      都是独立 `start`/`full`/`end`，`full` 只携带该 chunk 的**片段**
    ///      （coalesce 的累积器被逐 chunk 的 End 清零）——`start` 后的首个
    ///      `full` 必须**追加**为新段，否则替换到只剩最后一截。
    ///
    /// `start` 由 `mark_stream_segment_start` 记录；无 `start` 的 `full`
    /// （一次性错误文本、历史回填、turn-final 完成帧）按 merge 口径处理：
    /// 累积→前缀替换、重复→去重、多 part→拼接。
    pub fn apply_assistant_snapshot(&self, block_id: &str, text: &str) {
        if self
            .finalized_stream_blocks
            .get()
            .contains(&format!("m:{block_id}"))
        {
            return;
        }
        let new_segment = self.take_stream_segment("m", block_id);
        self.messages.update(|msgs| match msgs.last_mut() {
            Some(TranscriptBlock::AssistantMsg { id, content, .. }) if id == block_id => {
                if new_segment {
                    content.push_str(text);
                } else {
                    merge_snapshot_text_in_place(content, text);
                }
            }
            _ => msgs.push(TranscriptBlock::AssistantMsg {
                id: block_id.into(),
                content: text.into(),
                lifecycle: StreamBlockLifecycle::Streaming,
                fold: FoldState::Truncated,
            }),
        });
    }

    /// Append a thinking block, or extend the most-recent reasoning block
    /// when the id matches. Without this delta-aware merge, every
    /// reasoning chunk from the LLM stream appended a NEW thinking row,
    /// turning a single chain-of-thought into dozens of single-character
    /// blocks in the transcript.
    pub fn push_thinking(&self, id: &str, text: &str) {
        if self
            .finalized_stream_blocks
            .get()
            .contains(&format!("r:{id}"))
        {
            return;
        }
        self.messages.update(|msgs| {
            if let Some(TranscriptBlock::Thinking {
                id: bid, content, ..
            }) = msgs.last_mut()
            {
                if bid == id {
                    content.push_str(text);
                    return;
                }
            }
            close_trailing_thinking(msgs);
            msgs.push(TranscriptBlock::Thinking {
                id: id.into(),
                content: text.into(),
                lifecycle: StreamBlockLifecycle::Streaming,
                fold: FoldState::Truncated,
                duration_ms: 0,
                user_overridden: false,
            });
        });
    }

    /// Merge a `full`-phase reasoning snapshot into the running thinking block.
    /// 与 `apply_assistant_snapshot` 同理（`start` 新段追加，否则 merge）。
    pub fn apply_thinking_snapshot(&self, id: &str, text: &str) {
        if self
            .finalized_stream_blocks
            .get()
            .contains(&format!("r:{id}"))
        {
            return;
        }
        let new_segment = self.take_stream_segment("r", id);
        self.messages.update(|msgs| {
            if let Some(TranscriptBlock::Thinking {
                id: bid, content, ..
            }) = msgs.last_mut()
            {
                if bid == id {
                    if new_segment {
                        content.push_str(text);
                    } else {
                        merge_snapshot_text_in_place(content, text);
                    }
                    return;
                }
            }
            close_trailing_thinking(msgs);
            msgs.push(TranscriptBlock::Thinking {
                id: id.into(),
                content: text.into(),
                lifecycle: StreamBlockLifecycle::Streaming,
                fold: FoldState::Truncated,
                duration_ms: 0,
                user_overridden: false,
            });
        });
    }

    /// Append or update a tool call.
    ///
    /// 计时口径（Reasonix ToolCard elapsed/duration 模式）：首次插入记
    /// `started_at`；进入终态（Done）的那一刻固化 `duration`——之后如果
    /// 再收到同 id 的事件（重放/补发）不重算。
    pub fn upsert_tool_call(&self, id: &str, name: &str, params: &str, phase: ToolPhase) {
        let now = std::time::Instant::now();
        self.messages.update(|msgs| {
            for block in msgs.iter_mut() {
                if let TranscriptBlock::ToolCall {
                    id: bid,
                    name: block_name,
                    params: block_params,
                    phase: block_phase,
                    started_at,
                    duration,
                } = block
                {
                    if bid == id {
                        if !name.is_empty() {
                            *block_name = name.into();
                        }
                        if !params.is_empty() {
                            *block_params = params.into();
                        }
                        if phase == ToolPhase::Done && *block_phase != ToolPhase::Done {
                            *duration = Some(now - *started_at);
                        }
                        *block_phase = phase;
                        return;
                    }
                }
            }
            close_trailing_thinking(msgs);
            msgs.push(TranscriptBlock::ToolCall {
                id: id.into(),
                name: name.into(),
                params: params.into(),
                phase,
                started_at: now,
                duration: None,
            });
        });
    }

    /// Append a tool result.
    ///
    /// Defaults to `fold: FoldState::Folded`（单行 chip）：tool outputs 通常很长
    /// （一次 websearch dump 可达数千字符），默认展开会霸屏。Truncated（3 行
    /// 预览）仍挤占空间，故默认折叠成单行 chip（显示 name · N lines · 状态），
    /// 用户按 Space 展开看详情。兑现「工具是背景动作、默认不霸屏」
    /// （kimi/方案1/现代 AI TUI 共识）——原注释承诺「避免挤出屏幕」，但
    /// Truncated 并未真正兑现，Folded 才是。
    ///
    /// 例外：`diff` 预览（edit/write/apply_patch）默认 Truncated——diff 就是
    /// 用户要审阅的本体（3 行预览 + hint），不是背景噪音。
    pub fn push_tool_result(
        &self,
        id: &str,
        name: &str,
        result: &str,
        is_error: bool,
        diff: Option<DiffPreview>,
    ) {
        self.messages.update(|msgs| {
            let fold = if diff.is_some() {
                FoldState::Truncated
            } else {
                FoldState::Folded
            };
            let block = TranscriptBlock::ToolResult {
                id: id.into(),
                name: name.into(),
                result: result.into(),
                is_error,
                fold,
                diff,
            };
            // 插到对应 ToolCall 之后（同 tool_call_id），让调用与结果紧邻配对显示，
            // 而非 append 末尾——避免 LLM 并行发起多个 tool 时调用与结果割裂
            // （先一串 call、很久后一串 result）。找不到对应 ToolCall（事件乱序
            // 等异常）时 fallback append 末尾，保证结果不丢。ToolCall 与 ToolResult
            // 同 id 共存不冲突：fold/phase 查找均按 block 类型过滤。
            let pos = msgs
                .iter()
                .rposition(|b| matches!(b, TranscriptBlock::ToolCall { id: bid, .. } if bid == id));
            match pos {
                Some(i) => msgs.insert(i + 1, block),
                None => {
                    close_trailing_thinking(msgs);
                    msgs.push(block);
                }
            }
        });
    }

    /// Insert or update one stage card by stable identity.
    ///
    /// Scheduler progress repeatedly uses the same `id`, so step/status/message
    /// changes replace the existing transcript row in place. The user's fold
    /// choice survives updates.
    pub fn push_stage(
        &self,
        id: &str,
        name: &str,
        status: &str,
        message: &str,
        fields: Vec<StageField>,
    ) {
        self.messages.update(|msgs| {
            if let Some(TranscriptBlock::StageUpdate {
                name: current_name,
                status: current_status,
                message: current_message,
                fields: current_fields,
                ..
            }) = msgs.iter_mut().find(|block| {
                matches!(block, TranscriptBlock::StageUpdate { id: current_id, .. } if current_id == id)
            }) {
                current_name.clear();
                current_name.push_str(name);
                current_status.clear();
                current_status.push_str(status);
                current_message.clear();
                current_message.push_str(message);
                *current_fields = fields;
                return;
            }
            msgs.push(TranscriptBlock::StageUpdate {
                id: id.into(),
                name: name.into(),
                status: status.into(),
                message: message.into(),
                fields,
                fold: FoldState::Truncated,
            });
        });
    }

    /// Append a skill activation notice.
    pub fn push_skill(&self, id: &str, name: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptBlock::SkillActivated {
                id: id.into(),
                name: name.into(),
            })
        });
    }

    /// Append a compaction hint.
    pub fn push_compaction(&self, id: &str, before: u64, after: u64) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptBlock::CompactionHint {
                id: id.into(),
                before_tokens: before,
                after_tokens: after,
            })
        });
    }

    /// Append a system notice.
    pub fn push_notice(&self, id: &str, text: &str) {
        self.messages.update(|msgs| {
            msgs.push(TranscriptBlock::SystemNotice {
                id: id.into(),
                text: text.into(),
            })
        });
    }

    /// Push or update a todo list block.  Deduplicates by `block_id`:
    /// replaces the last TodoList with the same id, otherwise appends.
    ///
    /// Returns `true` when the visible content actually changed. When the
    /// incoming items are identical to the current block's, the Signal is
    /// left untouched (no dirty marking, no redraw) — callers key redraws
    /// off this flag.
    pub fn push_todo_list(
        &self,
        block_id: &str,
        items: Vec<crate::store::types::TodoItem>,
        summary: Option<crate::store::types::TodoSummary>,
    ) -> bool {
        // No-op fast path: identical items in the existing block — skip the
        // Signal write entirely so no redraw is triggered.
        if let Some(TranscriptBlock::TodoList { id, items: old, .. }) = self.messages.get().last() {
            if id == block_id && *old == items {
                return false;
            }
        }
        self.messages.update(|msgs| {
            // Replace existing TodoList with same id, or append
            if let Some(TranscriptBlock::TodoList { id, .. }) = msgs.last() {
                if id == block_id {
                    if let Some(TranscriptBlock::TodoList {
                        items: ref mut old, ..
                    }) = msgs.last_mut()
                    {
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
        true
    }

    // ── 消息折叠 ──

    pub fn toggle_fold(&self, block_idx: usize) {
        // Cycle through FoldState: Folded → Truncated → Expanded → Folded.
        let mut new_msgs: Vec<TranscriptBlock> = self.messages.get();
        match new_msgs.get_mut(block_idx) {
            Some(TranscriptBlock::UserPrompt { fold, .. })
            | Some(TranscriptBlock::ToolResult { fold, .. })
            | Some(TranscriptBlock::TodoList { fold, .. })
            | Some(TranscriptBlock::StageUpdate { fold, .. })
            | Some(TranscriptBlock::AssistantMsg { fold, .. }) => {
                *fold = fold.next();
            }
            // 手动操作接管：此后自动跟随（流结束自动收起）不再动此块。
            Some(TranscriptBlock::Thinking {
                fold,
                user_overridden,
                ..
            }) => {
                *fold = fold.next();
                *user_overridden = true;
            }
            _ => {}
        }
        self.messages.set(new_msgs);
    }

    // ── 水：遥测更新（EventBus → Signals）──

    pub fn set_token_usage(&self, usage: TokenUsage) {
        self.token_usage.set(usage);
    }

    pub fn set_context_pct(&self, pct: u8) {
        self.context_pct.set(pct.min(100));
    }

    /// 分母写入口（与 set_context_pct 配对）：0 视作未知归 None——
    /// 投影与播种两条路径都不该把 0 当真实窗口大小展示。
    pub fn set_context_limit(&self, limit: Option<u64>) {
        self.context_limit.set(limit.filter(|l| *l > 0));
    }

    pub fn set_mcp_lsp(&self, mcp_connected: usize, mcp_total: usize, lsp_active: Vec<String>) {
        self.mcp_lsp.set(McpLspInfo {
            mcp_connected,
            mcp_total,
            lsp_active,
        });
    }

    /// `FrontendEvent::DiffReplaced` 落地（replace 语义：全量替换，不累加）。
    /// 空集合 = 本轮无未决改动——角标隐藏，顺带收起明细（无内容可展开）。
    pub fn set_diff_summary(&self, diffs: Vec<DiffStat>) {
        if diffs.is_empty() {
            self.diff_detail_open.set(false);
        }
        self.diff_summary.set(diffs);
    }

    /// diff 角标点击：toggle 逐文件明细（keymap 鼠标唯一写路径）。
    pub fn toggle_diff_detail(&self) {
        let next = !self.diff_detail_open.get();
        self.diff_detail_open.set(next);
    }

    // ── 火：运行时 ──

    /// run_status 唯一写入口（土律·单点权威）：所有状态迁移经此，
    /// 同步维护 `running_since` 计时锚点——运行态显示总耗时（Reasonix
    /// "Working Ns" 模式）依赖它；重复的运行态事件不重置计时。
    pub fn set_run_status(&self, status: RunStatus) {
        let running = matches!(
            status,
            RunStatus::Running | RunStatus::Compacting | RunStatus::Sending
        );
        if running {
            if self.running_since.get().is_none() {
                self.running_since.set(Some(std::time::Instant::now()));
            }
        } else {
            self.running_since.set(None);
        }
        self.run_status.set(status);
    }

    /// §8：sandbox 执行增量进入活跃集合。与 server 侧 store 相同的
    /// refine 语义——已存在的条目只补非空字段（started 只带 pid，
    /// 不得抹掉 prepared 写入的 profile/fingerprint）。
    pub fn sandbox_execution_upsert(&self, summary: agendao_client::SandboxExecutionSummary) {
        if summary.execution_id.is_empty() {
            return;
        }
        self.active_sandboxes.update(|set| {
            match set
                .iter_mut()
                .find(|e| e.execution_id == summary.execution_id)
            {
                Some(existing) => {
                    if !summary.backend.is_empty() {
                        existing.backend = summary.backend.clone();
                    }
                    if !summary.profile_kind.is_empty() {
                        existing.profile_kind = summary.profile_kind.clone();
                    }
                    if !summary.plan_fingerprint.is_empty() {
                        existing.plan_fingerprint = summary.plan_fingerprint.clone();
                    }
                    if summary.pid.is_some() {
                        existing.pid = summary.pid;
                    }
                }
                None => set.push(summary),
            }
        });
    }

    /// §8：sandbox 执行离开活跃集合（exited/denied/violation）。
    pub fn sandbox_execution_removed(&self, execution_id: &str) {
        self.active_sandboxes
            .update(|set| set.retain(|e| e.execution_id != execution_id));
    }

    /// runtime 快照的权威替换：replaced 语义下事件流残迹不得存活。
    pub fn set_active_sandboxes(&self, snapshot: Vec<agendao_client::SandboxExecutionSummary>) {
        self.active_sandboxes.set(snapshot);
    }

    pub fn set_active_tool(&self, id: &str, name: &str, phase: ToolPhase) {
        self.active_tools.update(|tools| {
            // 同一工具的 phase 推进保留最初起点，工具耗时才是真实执行时长。
            let started_at = tools
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.started_at)
                .unwrap_or_else(std::time::Instant::now);
            tools.retain(|t| t.id != id);
            tools.push(ActiveTool {
                id: id.into(),
                name: name.into(),
                phase,
                started_at,
            });
        });
    }

    // ── 木：输入附面（附件）──

    pub fn add_attachment(&self, attachment: Attachment) {
        self.attachments.update(|a| a.push(attachment));
    }

    pub fn clear_attachments(&self) {
        self.attachments.set(Vec::new());
    }

    // ── Session ID ──

    pub fn set_session_id(&self, id: &str) {
        if self.session_id.get().as_deref() != Some(id) {
            self.details_policy
                .set(crate::details_policy::DetailsPolicy::default());
            self.topology_summary.set(None);
            self.subagent_projection.set(None);
        }
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
        // U11④：回底即已读——未读计数立即消失（不等下一帧 render 回写）。
        self.unread_seen_len.set(self.messages.get().len());
    }

    /// U11②：滚到顶。offset 语义 = 距底行数，顶 = max_offset；u16::MAX
    /// 由渲染侧 `.min(max_offset)` 收口到真实顶，无需在此知道总高。
    pub fn scroll_to_top(&self) {
        self.scroll_offset.set(u16::MAX);
    }

    /// U11：render 每帧回写（单点，渲染算完权威 total_h 后调用）。
    /// ①内容锚定：offset>0 且总高环比增长 Δ → offset += Δ（钉内容
    /// 坐标，流式 chunk 顶不走正在读的行）；④未读记账：offset==0 →
    /// seen 对齐当前块数（在底即已读）。pinned（内联 permission/
    /// question 强制钉底）期间不锚定。last==0（reset/新会话首帧）
    /// 不锚定——防止首帧把 offset 顶飞。
    pub fn sync_scroll_frame(&self, total_h: u16, msg_len: usize, pinned: bool) {
        let last = self.scroll_anchor_last_h.get();
        self.scroll_anchor_last_h.set(total_h);
        if self.scroll_offset.get() == 0 {
            self.unread_seen_len.set(msg_len);
            return;
        }
        if pinned || last == 0 || total_h <= last {
            return;
        }
        let delta = total_h - last;
        self.scroll_offset.update(|o| *o = o.saturating_add(delta));
    }

    /// U11④：翻上去阅读期间新到底的块数（status bar "↓ N new"）。
    pub fn unread_count(&self) -> usize {
        if self.scroll_offset.get() == 0 {
            return 0;
        }
        self.messages
            .get()
            .len()
            .saturating_sub(self.unread_seen_len.get())
    }

    // ── Transcript cursor & fold ──
    //
    // The cursor is what `Space` / `Enter` operate on inside the
    // transcript. Moving the cursor with j/k auto-scrolls so the
    // cursor row stays in view (mirroring how vim handles long files).

    /// Move cursor to the previous foldable block, wrapping at the top.
    /// "Foldable" today means UserPrompt / Thinking / ToolResult / TodoList /
    /// StageUpdate / AssistantMsg — the blocks whose `toggle_fold` actually flips
    /// state. Cursor skips tool-call rows since fold is a no-op there.
    pub fn cursor_prev_foldable(&self) {
        let msgs = self.messages.get();
        let mut idx = self.transcript_cursor.get().unwrap_or(msgs.len());
        loop {
            if idx == 0 {
                idx = msgs.len();
            }
            if idx == 0 {
                return;
            }
            idx -= 1;
            if Self::is_foldable(&msgs[idx]) {
                break;
            }
        }
        self.transcript_cursor.set(Some(idx));
    }

    pub fn cursor_next_foldable(&self) {
        let msgs = self.messages.get();
        if msgs.is_empty() {
            return;
        }
        let mut idx = self.transcript_cursor.get().map(|i| i + 1).unwrap_or(0);
        let start = idx;
        loop {
            if idx >= msgs.len() {
                idx = 0;
            }
            if Self::is_foldable(&msgs[idx]) {
                self.transcript_cursor.set(Some(idx));
                return;
            }
            idx += 1;
            // Loop guard: if we walked the whole list back to the start.
            if idx == start {
                return;
            }
        }
    }

    fn is_foldable(block: &TranscriptBlock) -> bool {
        matches!(
            block,
            TranscriptBlock::UserPrompt { .. }
                | TranscriptBlock::Thinking { .. }
                | TranscriptBlock::ToolResult { .. }
                | TranscriptBlock::TodoList { .. }
                | TranscriptBlock::StageUpdate { .. }
                | TranscriptBlock::AssistantMsg { .. }
        )
    }

    /// Top row (from the beginning of the content) where the block
    /// under the cursor lives. Each block occupies its `height()` rows
    /// plus a 1-row gap (matching the `vstack().gap(1)` the renderer
    /// uses). Returns 0 if no cursor is set.
    pub fn cursor_top_row(&self) -> u16 {
        let Some(cursor) = self.transcript_cursor.get() else {
            return 0;
        };
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
        let Some(cursor) = self.transcript_cursor.get() else {
            return;
        };
        // U13⑤：原 `cursor == 0 → return` 假设"首块总在 scroll_top=0
        // 可见"——错误：钉底时 scroll_top=max_offset，首块远在视口外，
        // j/k 绕回首块时视口不跟随（块被折叠/展开却看不见）。删掉该
        // 早退，下方数学天然处理：cursor_top=0 → new_scroll_top=0 →
        // offset=max_offset（真实顶）。
        let total = self.total_transcript_height();
        if total <= viewport_h {
            return;
        } // everything fits, nothing to scroll
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

    /// Focus a typed evidence reference already present in the transcript.
    /// Stage ids/names and tool call/result ids share this single lookup so
    /// Task State navigation cannot invent a second transcript index.
    pub fn focus_transcript_reference(&self, reference: &str, viewport_h: u16) -> bool {
        let messages = self.messages.get();
        let position = messages.iter().position(|block| match block {
            TranscriptBlock::UserPrompt { id, .. }
            | TranscriptBlock::Thinking { id, .. }
            | TranscriptBlock::ToolCall { id, .. }
            | TranscriptBlock::ToolResult { id, .. }
            | TranscriptBlock::SkillActivated { id, .. }
            | TranscriptBlock::TodoList { id, .. }
            | TranscriptBlock::AssistantMsg { id, .. }
            | TranscriptBlock::ImageRef { id, .. }
            | TranscriptBlock::CompactionHint { id, .. }
            | TranscriptBlock::SystemNotice { id, .. } => id == reference,
            TranscriptBlock::StageUpdate { id, name, .. } => id == reference || name == reference,
        });
        let Some(position) = position else {
            return false;
        };
        self.transcript_cursor.set(Some(position));
        self.ensure_cursor_visible(viewport_h);
        true
    }

    /// 给定块索引 `i`，若它在连续 ToolResult 组内，返回 `(组首索引, 组长度)`。
    /// 用于「单井聚合」：判断 cursor 是否落在工具结果组内，从而决定 Space 是
    /// 切该块自己的 fold 还是展开整组。组折叠阈值与 `layout_tool_result_group`
    /// 的 `TOOL_GROUP_PREVIEW` 共享，避免渲染与交互漂移（金律：阈值单点）。
    fn tool_group_head(msgs: &[TranscriptBlock], i: usize) -> Option<(usize, usize)> {
        let b = msgs.get(i)?;
        if !matches!(b, TranscriptBlock::ToolResult { .. }) {
            return None;
        }
        let mut head = i;
        while head > 0 && matches!(msgs[head - 1], TranscriptBlock::ToolResult { .. }) {
            head -= 1;
        }
        let mut len = 0;
        let mut j = head;
        while j < msgs.len() && matches!(msgs[j], TranscriptBlock::ToolResult { .. }) {
            len += 1;
            j += 1;
        }
        Some((head, len))
    }

    /// Toggle fold on the block under the cursor (or the latest
    /// foldable block when no cursor is set yet — matches the user's
    /// "I just want to expand the last result" mental model).
    ///
    /// U26②：返回是否真切了 fold——无可折叠块、或 cursor 显式落在无
    /// fold 字段的块（ToolCall/SystemNotice 等）时 false，调用方
    ///（Space 键）据此把按键落回编辑器（空格可作消息首字符），不做
    /// 无声消费（第十条）。
    pub fn toggle_fold_at_cursor(&self) -> bool {
        let msgs = self.messages.get();
        let mut idx = self.transcript_cursor.get();
        if idx.is_none() {
            // Find the most recent foldable block.
            for i in (0..msgs.len()).rev() {
                if Self::is_foldable(&msgs[i]) {
                    idx = Some(i);
                    break;
                }
            }
        }
        let Some(i) = idx else {
            return false;
        };
        // 单井聚合：cursor 落在折叠的 ToolResult 组内（段首 fold=Folded 且项数 >
        // TOOL_GROUP_PREVIEW）时，Space 展开整组（切段首 fold）——让「[+N more]」
        // 行也能点开。组未折叠（项数 ≤ 阈值，或段首已 Expanded）则切 cursor 块自己
        // 的 fold（段首块的 fold 即组开关：展开态下 Space 折叠组）。
        let expand_group_head = Self::tool_group_head(&msgs, i).and_then(|(head, len)| {
            use crate::screen::session::TOOL_GROUP_PREVIEW;
            let collapsed = len > TOOL_GROUP_PREVIEW
                && matches!(
                    &msgs[head],
                    TranscriptBlock::ToolResult {
                        fold: FoldState::Folded,
                        ..
                    }
                );
            if collapsed {
                Some(head)
            } else {
                None
            }
        });
        drop(msgs);
        let target = expand_group_head.unwrap_or(i);
        // 目标块是否真的会被 toggle_fold 改变（toggle_fold 只认带 fold
        // 字段的五类；cursor 显式落在 ToolCall/SystemNotice 等无 fold 块
        // 上时它是无声 no-op）——不切则如实报 false，让 Space 落回编辑器。
        let toggleable = {
            let msgs = self.messages.get();
            matches!(
                msgs.get(target),
                Some(
                    TranscriptBlock::UserPrompt { .. }
                        | TranscriptBlock::Thinking { .. }
                        | TranscriptBlock::ToolResult { .. }
                        | TranscriptBlock::TodoList { .. }
                        | TranscriptBlock::StageUpdate { .. }
                        | TranscriptBlock::AssistantMsg { .. }
                )
            )
        };
        if !toggleable {
            return false;
        }
        self.toggle_fold(target);
        self.transcript_cursor.set(Some(i));
        true
    }

    /// 土律：transcript → 纯文本的**唯一**序列化权威。
    ///
    /// 既给 `/copy`（OSC52 直发）也给 `/export`（dialog c/s 共用）用，
    /// 避免两处各自实现"User: " / "Assistant: " / "Tool: name(params)" /
    /// "Result [name]: " 的格式漂移（金的成形语法在这里**只能有一份**）。
    ///
    /// U18② 起逐块成形委托 `block_to_text`（全 13 变体覆盖，含 Thinking
    /// ——重构基线的"Thinking 跳过"是有意改变：思考内容可读、可复制）。
    pub fn transcript_to_text(&self) -> String {
        let msgs = self.messages.get();
        let mut text = String::new();
        for b in msgs.iter() {
            text.push_str(&block_to_text(b));
            text.push('\n');
        }
        text
    }

    /// 取光标当前 block 若是 UserPrompt 则返回 `(id, content)`；否则 `None`。
    ///
    /// 给 `/revise` Edit & Resend 用：keymap 先调本方法拿 (id, content)，
    /// 再做 fork+set_text。光标不在 UserPrompt 上时返回 None，调用方负责
    /// toast 提示——避免无声失败（道纪第十条：唯一查询权威）。
    pub fn cursor_user_prompt(&self) -> Option<(String, String)> {
        let cursor = self.transcript_cursor.get()?;
        let msgs = self.messages.get();
        let block = msgs.get(cursor)?;
        match block {
            TranscriptBlock::UserPrompt { id, content, .. } => Some((id.clone(), content.clone())),
            _ => None,
        }
    }

    /// 取光标当前 block 的文本表示，用于 'c' 单块复制（OSC52 → 终端剪贴板）。
    ///
    /// U18② 起逐块成形委托 `block_to_text`（与全量序列化同一权威）——
    /// 全 13 变体覆盖，'c' 不再对 SkillActivated /
    /// TodoList / StageUpdate 等报 "Nothing to copy"；None 只剩"无
    /// cursor / cursor 超界"两种（调用方 toast 的语义不变）。
    pub fn cursor_block_to_text(&self) -> Option<String> {
        let cursor = self.transcript_cursor.get()?;
        let msgs = self.messages.get();
        msgs.get(cursor).map(block_to_text)
    }
}

/// 土律：单个 TranscriptBlock → 纯文本的**唯一**成形权威（U18②）。
///
/// `transcript_to_text`（/copy 全量、/export）、`cursor_block_to_text`
/// （'c' 单块）、keymap 的 visible_transcript_text（'C' 当前屏）三处
/// 共用——"User: " / "Tool: name(params)" 等成形语法只有一份（金的
/// 成形语法不可漂移）。
///
/// match 无通配臂：新增 TranscriptBlock 变体时编译错强制在这里做出
/// 序列化决定（金：新增块的成形不许逃逸审查）。
pub fn block_to_text(block: &TranscriptBlock) -> String {
    match block {
        TranscriptBlock::UserPrompt { content, .. } => format!("User: {content}"),
        TranscriptBlock::AssistantMsg { content, .. } => format!("Assistant: {content}"),
        TranscriptBlock::ToolCall { name, params, .. } => format!("Tool: {name}({params})"),
        TranscriptBlock::ToolResult { name, result, .. } => {
            format!("Result [{name}]: {result}")
        }
        TranscriptBlock::Thinking { content, .. } => format!("Thinking: {content}"),
        TranscriptBlock::SkillActivated { name, .. } => format!("Skill: {name}"),
        TranscriptBlock::TodoList { items, .. } => {
            let mut out = String::from("Todo:");
            for item in items {
                // 对齐 render 的状态记号：✓ 完成 / ▶ 进行 / ✗ 取消 / 空 待办，
                // 纯文本用 markdown task-list 方言（[x]/[~]/[-]/[ ]）。
                let mark = match item.status {
                    TodoStatus::Completed => "x",
                    TodoStatus::InProgress => "~",
                    TodoStatus::Cancelled => "-",
                    TodoStatus::Pending => " ",
                };
                out.push_str(&format!("\n- [{mark}] {}", item.content));
            }
            out
        }
        TranscriptBlock::StageUpdate {
            name,
            status,
            message,
            fields,
            ..
        } => {
            let mut out = format!("Stage: {name} — {status}");
            if !fields.is_empty() {
                out.push('\n');
                out.push_str(
                    &fields
                        .iter()
                        .map(|field| format!("{}: {}", field.label, field.value))
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
            }
            if !message.is_empty() {
                out.push('\n');
                out.push_str(message);
            }
            out
        }
        TranscriptBlock::CompactionHint {
            before_tokens,
            after_tokens,
            ..
        } => {
            format!("Compaction: {before_tokens} → {after_tokens} tokens")
        }
        TranscriptBlock::SystemNotice { text, .. } => format!("Notice: {text}"),
        TranscriptBlock::ImageRef { mime, .. } => format!("[Image: {mime}]"),
    }
}

/// 快照归并（与服务端 `merge_snapshot_text_in_place` 同口径——
/// routes/event_stream.rs 与 session_runtime/local_frontend.rs 两处副本的
/// 第三种消费方副本），原地增量更新版本。
///
/// 逐字节等价于旧的"分配新 String 返回"版本：`merged` 在所有分支下都以
/// `existing` 为前缀或与 `existing` 相同，因此只需 append 增量即可，
/// 无需每 chunk 重分配全文（旧实现 10KB 回答 × 500 chunk ≈ 5MB 重分配）。
///
/// `full` 快照的 text 在两种线上形态下都正确：
///   - 累积快照：incoming 以 existing 为前缀 → 追加增量（等价于取 incoming 替换）。
///   - 重复/陈旧快照：existing 以 incoming 为前缀 → 保留 existing（去重）。
///   - 逐 chunk 片段：无前缀关系 → 去重叠后拼接（仅追加尾部）。
///
/// Reasonix 自动跟随口径：紧跟其后的非 thinking 块落地 = 思考流结束——
/// 未被用户接管的尾部 Thinking 自动收起（Folded）。用户折叠/展开过
/// （user_overridden）则尊重用户选择不动。
fn close_trailing_thinking(msgs: &mut [TranscriptBlock]) {
    if let Some(TranscriptBlock::Thinking {
        fold,
        user_overridden,
        ..
    }) = msgs.last_mut()
    {
        if !*user_overridden {
            *fold = FoldState::Folded;
        }
    }
}

fn merge_snapshot_text_in_place(existing: &mut String, incoming: &str) {
    if incoming.is_empty() {
        return;
    }
    if existing.is_empty() {
        existing.reserve(incoming.len());
        existing.push_str(incoming);
        return;
    }
    if incoming.starts_with(existing.as_str()) {
        existing.push_str(&incoming[existing.len()..]);
        return;
    }
    if existing.starts_with(incoming) {
        return;
    }

    let overlap = suffix_prefix_overlap(existing, incoming);
    existing.reserve(incoming.len() - overlap);
    existing.push_str(&incoming[overlap..]);
}

fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
    let max = existing.len().min(incoming.len());
    for size in (1..=max).rev() {
        if existing.is_char_boundary(existing.len() - size)
            && incoming.is_char_boundary(size)
            && existing[existing.len() - size..] == incoming[..size]
        {
            return size;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧的分配式参考实现（golden，逐字节保留修复前行为），仅用于等价性断言。
    fn reference_merge_snapshot_text(existing: &str, incoming: &str) -> String {
        if existing.is_empty() {
            return incoming.to_string();
        }
        if incoming.is_empty() {
            return existing.to_string();
        }
        if incoming.starts_with(existing) {
            return incoming.to_string();
        }
        if existing.starts_with(incoming) {
            return existing.to_string();
        }
        let overlap = suffix_prefix_overlap(existing, incoming);
        if overlap > 0 {
            let mut merged = String::with_capacity(existing.len() + incoming.len() - overlap);
            merged.push_str(existing);
            merged.push_str(&incoming[overlap..]);
            return merged;
        }
        let mut merged = String::with_capacity(existing.len() + incoming.len());
        merged.push_str(existing);
        merged.push_str(incoming);
        merged
    }

    fn merge_via_in_place(existing: &str, incoming: &str) -> String {
        let mut merged = existing.to_string();
        merge_snapshot_text_in_place(&mut merged, incoming);
        merged
    }

    /// 确定性伪随机 op 流：覆盖 append 增量、累积快照、陈旧快照、重叠片段、
    /// 不相干片段、空快照全部分支，逐步断言原地实现与参考实现逐字节一致。
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
            xs[(self.next() as usize) % xs.len()]
        }
    }

    const MERGE_FRAGS: [&str; 8] = ["The", " answer", " is", " ", "ab", "你好", "，", "x"];

    #[test]
    fn reset_for_new_session_clears_task_ledger() {
        let store = SessionStore::new();
        store.task_ledger.set(Some(std::sync::Arc::new(
            agendao_types::task_ledger::SessionTaskLedgerView::from(
                agendao_types::task_ledger::SessionTaskLedger {
                    session_id: "ses_t".into(),
                    revision: 7,
                    status: agendao_types::task_ledger::TaskLedgerStatus::Active,
                    ..agendao_types::task_ledger::SessionTaskLedger::empty("ses_t")
                },
            ),
        )));
        assert!(store.task_ledger.get().is_some());
        store.reset_for_new_session();
        assert!(
            store.task_ledger.get().is_none(),
            "stale ledger must not leak across sessions"
        );
    }

    #[test]
    fn task_ledger_reconciliation_rejects_wrong_session_and_old_revision() {
        let store = SessionStore::new();
        store.set_session_id("ses_active");
        let mut wrong = agendao_types::task_ledger::SessionTaskLedger::empty("ses_other");
        wrong.revision = 3;
        assert!(!store.apply_task_ledger_snapshot(wrong.into()));

        let mut current = agendao_types::task_ledger::SessionTaskLedger::empty("ses_active");
        current.revision = 4;
        assert!(store.apply_task_ledger_snapshot(current.into()));
        let mut stale = agendao_types::task_ledger::SessionTaskLedger::empty("ses_active");
        stale.revision = 2;
        assert!(!store.apply_task_ledger_snapshot(stale.into()));
        assert_eq!(store.task_ledger.get().unwrap().revision, 4);
    }

    #[test]
    fn merge_snapshot_handles_cumulative_and_fragment_streams() {
        // 累积快照（Bug B 形态）：替换而非拼接
        let mut acc = String::new();
        for snap in ["The", "The answer to", "The answer to 1+1 is 2"] {
            merge_snapshot_text_in_place(&mut acc, snap);
        }
        assert_eq!(acc, "The answer to 1+1 is 2");

        // 逐 chunk 片段（无前缀关系时退化为拼接——段内混合内容的兜底）
        let mut acc = String::new();
        for frag in ["R", "ust is", " a systems", " language"] {
            merge_snapshot_text_in_place(&mut acc, frag);
        }
        assert_eq!(acc, "Rust is a systems language");

        // 重叠去重
        assert_eq!(merge_via_in_place("Rust is", "is a"), "Rust is a");
        // 陈旧快照去重
        assert_eq!(merge_via_in_place("The answer", "The"), "The answer");
        // 注意：逐 token 片段流（如 "1","+","1"）不靠 merge 拼接——片段携带
        // per-chunk `start`，由 apply_assistant_snapshot 的分段追加负责
        // （见 event_handler::tests::fragment_full_snapshots_append_after_each_start）。
    }

    #[test]
    fn merge_snapshot_in_place_matches_reference_byte_for_byte() {
        let mut rng = Lcg(0x5EED_5EED_5EED_5EED);
        let mut in_place = String::new();
        let mut reference = String::new();
        // truth 是"线上真实全文"，用于构造逼真的累积/陈旧/重叠 incoming。
        let mut truth = String::new();

        for step in 0..600 {
            let incoming = match rng.next() % 6 {
                // 累积快照：incoming 以现有内容为前缀。
                0 => {
                    truth.push_str(rng.pick(&MERGE_FRAGS));
                    truth.clone()
                }
                // 陈旧/重复快照：incoming 是现有内容的前缀。
                1 => {
                    let keep = (rng.next() as usize) % (truth.len() + 1);
                    let mut boundary = 0;
                    for (idx, _) in truth.char_indices() {
                        if idx <= keep {
                            boundary = idx;
                        }
                    }
                    truth[..boundary].to_string()
                }
                // 重叠片段：现有内容的最后一个片段 + 新片段。
                2 => {
                    let tail = rng.pick(&MERGE_FRAGS);
                    let head = rng.pick(&MERGE_FRAGS);
                    truth.push_str(head);
                    format!("{tail}{head}")
                }
                // 不相干片段：无前缀关系，退化为拼接。
                3 => {
                    let frag = rng.pick(&MERGE_FRAGS);
                    truth.push_str(frag);
                    frag.to_string()
                }
                // 空快照：no-op。
                4 => String::new(),
                // 完整替换（incoming == 现有内容）。
                _ => truth.clone(),
            };
            merge_snapshot_text_in_place(&mut in_place, &incoming);
            reference = reference_merge_snapshot_text(&reference, &incoming);
            assert_eq!(
                in_place, reference,
                "step {step}: in-place merge diverged from reference (incoming={incoming:?})"
            );
        }
        assert!(!in_place.is_empty(), "op stream must exercise real merges");
    }

    /// 分配量级断言：600 帧累积快照（最终 ~数 KB）下，原地归并只做
    /// O(chunk) 增量追加，总分配应限制在最终文本量的常数倍内；
    /// 旧的"每帧分配全文"实现同口径下为 Σ frame_len ≈ 百 KB 级（O(n²)）。
    #[test]
    fn merge_snapshot_in_place_allocates_linear_not_quadratic() {
        const CHUNKS: usize = 400;
        const CHUNK: &str = "abcdefghijklmnopqrstuvwxy"; // 25 bytes
                                                         // 预先构造好输入帧（不计入测量）：第 i 帧是长度 i*25 的累积快照。
        let mut frames = Vec::with_capacity(CHUNKS);
        let mut truth = String::new();
        for _ in 0..CHUNKS {
            truth.push_str(CHUNK);
            frames.push(truth.clone());
        }
        // 旧实现的分配下界：每帧分配一次合并结果，Σ i*25 = 2_005_000 bytes。
        let quadratic_reference: usize = (1..=CHUNKS).sum::<usize>() * CHUNK.len();

        let guard = crate::test_alloc::AllocGuard::start();
        let mut acc = String::new();
        for frame in &frames {
            merge_snapshot_text_in_place(&mut acc, frame);
        }
        let allocated = guard.bytes();
        drop(guard);

        assert_eq!(acc.len(), CHUNKS * CHUNK.len());
        assert!(
            allocated < quadratic_reference / 20,
            "in-place merge must be ~O(n) (allocated {allocated} bytes; \
             quadratic reference {quadratic_reference} bytes)"
        );
    }

    /// store 级 golden：full 快照流经 apply_assistant_snapshot 的最终内容与
    /// 参考归并模型逐字节一致；`start` 标记的新段仍为追加语义（分段不变）。
    #[test]
    fn apply_assistant_snapshot_matches_reference_model() {
        let mut rng = Lcg(0xC0FF_EE11_2233_4455);
        let s = SessionStore::new();
        let block_id = "msg-golden";
        let mut reference = String::new();
        // truth 是"线上真实全文"，用于构造逼真的累积/重叠 incoming。
        let mut truth = String::new();

        for step in 0..400 {
            let new_segment = rng.next().is_multiple_of(10);
            if new_segment {
                s.mark_stream_segment_start("m", block_id);
            }
            let incoming = match rng.next() % 4 {
                // 累积快照。
                0 => {
                    truth.push_str(rng.pick(&MERGE_FRAGS));
                    truth.clone()
                }
                // 不相干片段。
                1 => rng.pick(&MERGE_FRAGS).to_string(),
                // 重复快照（incoming == 现有全文）。
                2 => truth.clone(),
                // 重叠片段。
                _ => {
                    let tail = rng.pick(&MERGE_FRAGS);
                    let head = rng.pick(&MERGE_FRAGS);
                    truth.push_str(head);
                    format!("{tail}{head}")
                }
            };
            s.apply_assistant_snapshot(block_id, &incoming);
            if new_segment {
                reference.push_str(&incoming);
            } else {
                reference = reference_merge_snapshot_text(&reference, &incoming);
            }
            let msgs = s.messages.get();
            let Some(TranscriptBlock::AssistantMsg { content, .. }) = msgs.last() else {
                panic!("step {step}: expected assistant block");
            };
            assert_eq!(content, &reference, "step {step}: store content diverged");
        }
    }

    /// reset_for_new_session 必须清空对话运行态(messages/session_id/title/scroll/cursor),
    /// 否则 /new 或 dispatch 创建新 session 时携带旧 session 残留——
    /// "新 session 接着旧 session 显示"的数据错位 bug 正是漏了这一步。
    #[test]
    fn reset_for_new_session_clears_conversation_state() {
        let s = SessionStore::new();
        s.set_session_id("old-session");
        s.push_user_message("m1", "残留的旧消息");
        s.title.set("Old Title".into());
        s.scroll_offset.set(5);
        s.transcript_cursor.set(Some(2));
        s.session_model.set(Some("deepseek/deepseek-v4-pro".into()));
        s.session_agent.set(Some("build".into()));

        s.reset_for_new_session();

        assert_eq!(s.get_session_id(), None, "session_id 必须清空");
        assert!(s.messages.get().is_empty(), "messages 必须清空");
        assert_eq!(s.title.get(), "New Session".to_string(), "title 回到初始");
        assert_eq!(s.scroll_offset.get(), 0, "scroll_offset 归零");
        assert_eq!(s.transcript_cursor.get(), None, "cursor 清空");
        assert!(s.session_model.get().is_none(), "session model 必须清空");
        assert!(s.session_agent.get().is_none(), "session agent 必须清空");
    }

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
            s.upsert_tool_call(
                &format!("t{i}"),
                "read",
                &format!("f{i}"),
                ToolPhase::Starting,
            );
        }
        for i in 1..=5 {
            s.upsert_tool_call(&format!("t{i}"), "read", "", ToolPhase::Done);
            s.push_tool_result(&format!("t{i}"), "read", &format!("out{i}"), false, None);
        }
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 10);
        // 期望顺序：TC1, TR1, TC2, TR2, …, TC5, TR5
        for i in 0..5 {
            match (&msgs[i * 2], &msgs[i * 2 + 1]) {
                (
                    TranscriptBlock::ToolCall { id: cid, .. },
                    TranscriptBlock::ToolResult {
                        id: rid, result, ..
                    },
                ) => {
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
        s.push_tool_result("orphan", "read", "out", false, None);
        let msgs = s.messages.get();
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0], TranscriptBlock::ToolResult { .. }));
    }

    /// diff 预览默认 Truncated（可审阅本体），普通结果保持 Folded（不霸屏）。
    /// set_diff_summary 是 replace 语义；空集合收起明细。
    #[test]
    fn diff_result_defaults_truncated_and_summary_replaces() {
        let s = SessionStore::new();
        s.push_tool_result(
            "t1",
            "edit",
            "",
            false,
            Some(DiffPreview {
                text: "+a\n-b".into(),
                truncated: false,
            }),
        );
        s.push_tool_result("t2", "read", "out", false, None);
        let msgs = s.messages.get();
        match &msgs[0] {
            TranscriptBlock::ToolResult { fold, diff, .. } => {
                assert_eq!(*fold, FoldState::Truncated);
                assert!(diff.is_some());
            }
            _ => panic!("expected ToolResult"),
        }
        match &msgs[1] {
            TranscriptBlock::ToolResult { fold, diff, .. } => {
                assert_eq!(*fold, FoldState::Folded);
                assert!(diff.is_none());
            }
            _ => panic!("expected ToolResult"),
        }

        s.set_diff_summary(vec![DiffStat {
            path: "a.rs".into(),
            additions: 1,
            deletions: 2,
        }]);
        assert_eq!(s.diff_summary.get().len(), 1);
        s.set_diff_summary(vec![
            DiffStat {
                path: "b.rs".into(),
                additions: 3,
                deletions: 0,
            },
            DiffStat {
                path: "c.rs".into(),
                additions: 0,
                deletions: 1,
            },
        ]);
        let summary = s.diff_summary.get();
        assert_eq!(summary.len(), 2, "replace 而非累加");
        assert_eq!(summary[0].path, "b.rs");
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
        assert_eq!(
            count_thinking(&msgs),
            1,
            "consecutive same-id reasoning must merge into one Thinking"
        );
        match &msgs[0] {
            TranscriptBlock::Thinking { content, .. } => {
                assert_eq!(content, "step 1 step 2 step 3")
            }
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
        assert_eq!(
            count_thinking(&msgs),
            2,
            "interleaved assistant delta keeps reasoning as 2 separate Thinking blocks"
        );
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
        assert_eq!(
            count_thinking(&msgs),
            2,
            "interleaved tool call keeps reasoning as 2 separate Thinking blocks"
        );
    }

    #[test]
    fn thinking_different_id_creates_separate() {
        // 不同 id 的 reasoning 天然是独立 Thinking（多 message / 多 reasoning 周期）。
        let s = SessionStore::new();
        s.push_thinking("m1", "第一轮思考");
        s.push_thinking("m2", "第二轮思考");
        let msgs = s.messages.get();
        assert_eq!(
            count_thinking(&msgs),
            2,
            "different-id reasoning yields separate Thinking blocks"
        );
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

    /// U26②：toggle_fold_at_cursor 如实报告是否真切了 fold——
    /// 空 transcript / cursor 落在无 fold 字段的块（SystemNotice）
    /// 时 false（Space 据此落回编辑器）；有可折叠块时 true。
    #[test]
    fn toggle_fold_at_cursor_reports_whether_it_toggled() {
        let s = SessionStore::new();
        // 空 transcript：无可折叠块 → false。
        assert!(!s.toggle_fold_at_cursor());
        // 只有 SystemNotice（无 fold 字段）→ false。
        s.push_notice("n1", "note");
        assert!(!s.toggle_fold_at_cursor());
        // 有可折叠块（UserPrompt）→ true 且 cursor 被设置。
        s.push_user_message("u1", "hello");
        assert!(s.toggle_fold_at_cursor());
        assert_eq!(s.transcript_cursor.get(), Some(1));
        // cursor 显式落在 SystemNotice（index 0）→ false（不无声消费）。
        s.transcript_cursor.set(Some(0));
        assert!(!s.toggle_fold_at_cursor());
    }

    #[test]
    fn toggle_fold_assistant_msg_cycles_and_is_foldable() {
        let s = SessionStore::new();
        s.push_assistant_delta("a1", "l1\nl2\nl3\nl4\nl5");
        // 长回答默认 Truncated（3 行预览）；toggle → Expanded → Folded → Truncated。
        match &s.messages.get()[0] {
            TranscriptBlock::AssistantMsg { fold, .. } => assert_eq!(*fold, FoldState::Truncated),
            _ => panic!(),
        }
        assert!(SessionStore::is_foldable(&s.messages.get()[0]));
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::AssistantMsg { fold, .. } => assert_eq!(*fold, FoldState::Expanded),
            _ => panic!(),
        }
        s.toggle_fold(0);
        match &s.messages.get()[0] {
            TranscriptBlock::AssistantMsg { fold, .. } => assert_eq!(*fold, FoldState::Folded),
            _ => panic!(),
        }
    }

    #[test]
    fn token_usage_update() {
        let s = SessionStore::new();
        s.set_token_usage(TokenUsage {
            input: 100,
            output: 50,
            reasoning: 20,
            total: 170,
            cache_read: 10,
            cache_miss: 5,
            cache_write: 3,
            context_tokens: 2000,
            total_cost: 0.015,
        });
        let usage = s.token_usage.get();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 50);
        assert_eq!(usage.cache_read, 10);
    }

    #[test]
    fn attachments_add_and_clear() {
        let s = SessionStore::new();
        s.add_attachment(Attachment {
            name: "f".into(),
            kind: AttachmentKind::File {
                path: "p".into(),
                lines: 10,
            },
        });
        assert_eq!(s.attachments.get().len(), 1);
        s.clear_attachments();
        assert!(s.attachments.get().is_empty());
    }

    #[test]
    fn context_pct_clamps_at_100() {
        let s = SessionStore::new();
        s.set_context_pct(150);
        assert_eq!(s.context_pct.get(), 100);
    }

    /// 分母归一：0 与 None 同归 None（0 不是真实窗口大小）；正值原样保留。
    /// reset_for_new_session 清 None——新会话不携带旧会话的窗口口径。
    #[test]
    fn context_limit_normalizes_zero_and_resets() {
        let s = SessionStore::new();
        s.set_context_limit(Some(0));
        assert_eq!(s.context_limit.get(), None, "0 归一为未知");
        s.set_context_limit(Some(200_000));
        assert_eq!(s.context_limit.get(), Some(200_000));
        s.set_context_limit(None);
        assert_eq!(s.context_limit.get(), None);
        s.set_context_limit(Some(128_000));
        s.reset_for_new_session();
        assert_eq!(s.context_limit.get(), None, "reset 清分母");
    }

    /// `transcript_to_text` 是 export/copy 共用的唯一序列化权威——
    /// User / Assistant / Tool / Result 四种 block 都要正确序列化，
    /// 顺序与 messages 一致；未支持 block（如 Thinking）跳过不产生空行。
    #[test]
    fn transcript_to_text_serializes_all_supported_blocks_in_order() {
        let s = SessionStore::new();
        s.push_user_message("u1", "hello");
        s.push_assistant_delta("a1", "hi there");
        s.upsert_tool_call("t1", "read", "{\"path\":\"f\"}", ToolPhase::Starting);
        s.push_tool_result("t1", "read", "ok", false, None);

        let text = s.transcript_to_text();
        assert!(text.contains("User: hello\n"));
        assert!(text.contains("Assistant: hi there\n"));
        assert!(text.contains("Tool: read({\"path\":\"f\"})\n"));
        assert!(text.contains("Result [read]: ok\n"));
        // 顺序锚：User 行先于 Assistant 行先于 Tool 行。
        let u = text.find("User:").unwrap();
        let a = text.find("Assistant:").unwrap();
        let t = text.find("Tool:").unwrap();
        assert!(u < a && a < t);
    }

    /// 光标落在 UserPrompt 上时返回 `(id, content)`，
    /// 落在其他 block（如 ToolResult）或无 cursor 时返回 None。
    /// 这是 `/revise` 的关键前置——None 让调用方 toast，不静默失败。
    #[test]
    fn cursor_user_prompt_hits_only_user_blocks() {
        let s = SessionStore::new();
        s.push_user_message("u1", "edit me");
        s.push_tool_result("t1", "read", "out", false, None);

        // 无 cursor → None
        assert_eq!(s.cursor_user_prompt(), None);

        // cursor 在 UserPrompt → Some
        s.transcript_cursor.set(Some(0));
        assert_eq!(
            s.cursor_user_prompt(),
            Some(("u1".into(), "edit me".into()))
        );

        // cursor 在 ToolResult → None（光标范围里有这个 block 但不是 UserPrompt）
        s.transcript_cursor.set(Some(1));
        assert_eq!(s.cursor_user_prompt(), None);

        // cursor 超界 → None
        s.transcript_cursor.set(Some(99));
        assert_eq!(s.cursor_user_prompt(), None);
    }

    /// cursor_block_to_text 服务于 'c' 单块复制（OSC52）：逐块成形与
    /// transcript_to_text 同权威（block_to_text）。None 只剩无 cursor /
    /// 超界两种，让调用方 toast 提示。
    #[test]
    fn cursor_block_to_text_serializes_supported_blocks() {
        use crate::store::types::ToolPhase;
        let s = SessionStore::new();
        s.push_user_message("u1", "hello");
        s.push_assistant_delta("a1", "world");
        s.upsert_tool_call("t1", "Read", "{\"path\":\"x\"}", ToolPhase::Done);
        s.push_thinking("th1", "ponder");
        s.push_skill("sk1", "auto");

        // 无 cursor → None
        assert_eq!(s.cursor_block_to_text(), None);

        // UserPrompt
        s.transcript_cursor.set(Some(0));
        assert_eq!(s.cursor_block_to_text().as_deref(), Some("User: hello"));

        // AssistantMsg
        s.transcript_cursor.set(Some(1));
        assert_eq!(
            s.cursor_block_to_text().as_deref(),
            Some("Assistant: world")
        );

        // ToolCall
        s.transcript_cursor.set(Some(2));
        assert_eq!(
            s.cursor_block_to_text().as_deref(),
            Some("Tool: Read({\"path\":\"x\"})"),
        );

        // Thinking
        s.transcript_cursor.set(Some(3));
        assert_eq!(
            s.cursor_block_to_text().as_deref(),
            Some("Thinking: ponder")
        );

        // SkillActivated（U18②：原 None，现可读序列化）
        s.transcript_cursor.set(Some(4));
        assert_eq!(s.cursor_block_to_text().as_deref(), Some("Skill: auto"));

        // cursor 超界 → None
        s.transcript_cursor.set(Some(99));
        assert_eq!(s.cursor_block_to_text(), None);
    }

    /// U18②：block_to_text 覆盖全部 13 个变体（原返回 None 的 6 个
    /// 变体现在都可复制）；match 无通配臂，新增变体编译错强制决定成形。
    #[test]
    fn block_to_text_covers_all_variants() {
        use crate::store::types::{FoldState, TodoStatus, ToolPhase};
        let cases: Vec<(TranscriptBlock, &str)> = vec![
            (
                TranscriptBlock::UserPrompt {
                    id: "1".into(),
                    content: "hi".into(),
                    fold: FoldState::Expanded,
                    failed: false,
                },
                "User: hi",
            ),
            (
                TranscriptBlock::AssistantMsg {
                    id: "2".into(),
                    content: "yo".into(),
                    lifecycle: StreamBlockLifecycle::Streaming,
                    fold: FoldState::Expanded,
                },
                "Assistant: yo",
            ),
            (
                TranscriptBlock::ToolCall {
                    id: "3".into(),
                    name: "Bash".into(),
                    params: "ls".into(),
                    phase: ToolPhase::Done,
                    started_at: std::time::Instant::now(),
                    duration: None,
                },
                "Tool: Bash(ls)",
            ),
            (
                TranscriptBlock::ToolResult {
                    id: "4".into(),
                    name: "Bash".into(),
                    result: "ok".into(),
                    is_error: false,
                    fold: FoldState::Expanded,
                    diff: None,
                },
                "Result [Bash]: ok",
            ),
            (
                TranscriptBlock::Thinking {
                    id: "5".into(),
                    content: "hmm".into(),
                    lifecycle: StreamBlockLifecycle::Streaming,
                    fold: FoldState::Expanded,
                    duration_ms: 0,
                    user_overridden: false,
                },
                "Thinking: hmm",
            ),
            (
                TranscriptBlock::SkillActivated {
                    id: "6".into(),
                    name: "pdf".into(),
                },
                "Skill: pdf",
            ),
            (
                TranscriptBlock::TodoList {
                    id: "7".into(),
                    items: vec![
                        TodoItem {
                            content: "done".into(),
                            status: TodoStatus::Completed,
                        },
                        TodoItem {
                            content: "doing".into(),
                            status: TodoStatus::InProgress,
                        },
                        TodoItem {
                            content: "todo".into(),
                            status: TodoStatus::Pending,
                        },
                        TodoItem {
                            content: "dropped".into(),
                            status: TodoStatus::Cancelled,
                        },
                    ],
                    fold: FoldState::Expanded,
                    summary: None,
                },
                "Todo:\n- [x] done\n- [~] doing\n- [ ] todo\n- [-] dropped",
            ),
            (
                TranscriptBlock::StageUpdate {
                    id: "8".into(),
                    name: "plan".into(),
                    status: "running".into(),
                    message: String::new(),
                    fields: vec![],
                    fold: FoldState::Truncated,
                },
                "Stage: plan — running",
            ),
            (
                TranscriptBlock::StageUpdate {
                    id: "9".into(),
                    name: "exec".into(),
                    status: "done".into(),
                    message: "building".into(),
                    fields: vec![StageField {
                        label: "Tools".into(),
                        value: "1".into(),
                    }],
                    fold: FoldState::Truncated,
                },
                "Stage: exec — done\nTools: 1\nbuilding",
            ),
            (
                TranscriptBlock::CompactionHint {
                    id: "10".into(),
                    before_tokens: 9000,
                    after_tokens: 3000,
                },
                "Compaction: 9000 → 3000 tokens",
            ),
            (
                TranscriptBlock::SystemNotice {
                    id: "11".into(),
                    text: "note".into(),
                },
                "Notice: note",
            ),
            (
                TranscriptBlock::ImageRef {
                    id: "12".into(),
                    mime: "image/png".into(),
                },
                "[Image: image/png]",
            ),
        ];
        assert_eq!(cases.len(), 12, "13 变体减 StageUpdate 两例 = 12 条用例");
        for (block, want) in cases {
            assert_eq!(block_to_text(&block), want, "变体 {block:?}");
        }
    }

    /// U18②：全量序列化同权威——/copy 与 /export 现在也含 Thinking 等
    /// 块（原"未支持跳过"是有意改变：内容可读即应可复制）。
    #[test]
    fn transcript_to_text_includes_all_block_kinds() {
        let s = SessionStore::new();
        s.push_user_message("u1", "hello");
        s.push_thinking("th1", "ponder");
        s.push_skill("sk1", "auto");
        let text = s.transcript_to_text();
        assert!(text.contains("User: hello\n"));
        assert!(text.contains("Thinking: ponder\n"));
        assert!(text.contains("Skill: auto\n"));
    }

    // ── U11：内容锚定 + 未读计数 ──

    /// 翻上去阅读（offset>0）时内容底部长高 Δ → offset 同步 +Δ，
    /// 正在读的行视觉不动（锚内容坐标，非"距底行数"）。
    #[test]
    fn anchor_bumps_offset_on_growth_while_scrolled_up() {
        let s = SessionStore::new();
        s.push_user_message("u1", "hello");
        // 首帧：total 100，用户翻上去 10 行。
        s.sync_scroll_frame(100, 1, false);
        s.scroll_offset.set(10);
        // 次帧：流式把内容顶高 6 行。
        s.sync_scroll_frame(106, 1, false);
        assert_eq!(s.scroll_offset.get(), 16, "offset +Δ 保持阅读位置");
        // 再次长高 4 行。
        s.sync_scroll_frame(110, 1, false);
        assert_eq!(s.scroll_offset.get(), 20);
    }

    /// 在底（offset==0）不锚定——钉底语义不变。
    #[test]
    fn anchor_noop_at_bottom() {
        let s = SessionStore::new();
        s.sync_scroll_frame(100, 0, false);
        s.sync_scroll_frame(120, 0, false);
        assert_eq!(s.scroll_offset.get(), 0);
    }

    /// pinned（内联 permission/question 钉底）期间不锚定。
    #[test]
    fn anchor_noop_when_pinned() {
        let s = SessionStore::new();
        s.sync_scroll_frame(100, 0, false);
        s.scroll_offset.set(5);
        s.sync_scroll_frame(110, 0, true);
        assert_eq!(s.scroll_offset.get(), 5);
    }

    /// last==0 首帧不锚定（防 reset/新会话首帧把 offset 顶飞）。
    #[test]
    fn anchor_noop_on_first_frame() {
        let s = SessionStore::new();
        s.scroll_offset.set(7);
        s.sync_scroll_frame(100, 0, false);
        assert_eq!(s.scroll_offset.get(), 7);
    }

    /// 内容缩短（compact 替换）不锚定，offset 由渲染 min(max_offset) 收口。
    #[test]
    fn anchor_noop_on_shrink() {
        let s = SessionStore::new();
        s.sync_scroll_frame(100, 0, false);
        s.scroll_offset.set(10);
        s.sync_scroll_frame(40, 0, false);
        assert_eq!(s.scroll_offset.get(), 10);
    }

    /// 翻上去期间新块到达 → 未读计数；回底立即清零。
    #[test]
    fn unread_counts_new_blocks_until_bottom() {
        let s = SessionStore::new();
        s.push_user_message("u1", "a");
        // 在底一帧：seen 对齐 1。
        s.sync_scroll_frame(10, 1, false);
        // 翻上去后新到两块。
        s.scroll_offset.set(5);
        s.push_user_message("u2", "b");
        s.push_user_message("u3", "c");
        s.sync_scroll_frame(20, 3, false);
        assert_eq!(s.unread_count(), 2);
        // 回底 → 未读消失。
        s.scroll_to_bottom();
        assert_eq!(s.unread_count(), 0);
    }

    /// scroll_to_top 置 u16::MAX，渲染侧 min(max_offset) 收口到真实顶。
    #[test]
    fn scroll_to_top_saturates_to_max_offset() {
        let s = SessionStore::new();
        s.scroll_to_top();
        assert_eq!(s.scroll_offset.get(), u16::MAX);
    }

    /// U13⑤：cursor==0（首块）在钉底时不在视口内——ensure_cursor_visible
    /// 必须把视口拉回真实顶（原 cursor==0 早退假设"首块总可见"，错误）。
    #[test]
    fn ensure_cursor_visible_scrolls_to_first_block() {
        let s = SessionStore::new();
        for i in 0..10 {
            s.push_user_message(&format!("u{i}"), "line");
        }
        let total = s.total_transcript_height();
        let viewport: u16 = 4;
        assert!(total > viewport, "前置：内容须超出视口");
        s.transcript_cursor.set(Some(0));
        s.ensure_cursor_visible(viewport);
        // cursor_top=0 → new_scroll_top=0（pad 饱和）→ offset=max_offset。
        assert_eq!(s.scroll_offset.get(), total - viewport);
    }

    #[test]
    fn task_state_evidence_focus_uses_stage_id_or_name() {
        let s = SessionStore::new();
        s.push_user_message("u1", "before");
        s.push_stage("stage-id", "verify/tests", "completed", "", vec![]);
        s.push_user_message("u2", "after");

        assert!(s.focus_transcript_reference("verify/tests", 4));
        assert_eq!(s.transcript_cursor.get(), Some(1));
        assert!(s.focus_transcript_reference("stage-id", 4));
        assert!(!s.focus_transcript_reference("missing", 4));
    }

    #[test]
    fn stage_update_replaces_in_place_and_preserves_expansion() {
        let s = SessionStore::new();
        s.push_stage(
            "scheduler-progress:execute",
            "Scheduler step 1/16",
            "running",
            "one\ntwo\nthree\nfour",
            vec![StageField {
                label: "Agent".into(),
                value: "worker".into(),
            }],
        );
        s.transcript_cursor.set(Some(0));
        assert!(s.toggle_fold_at_cursor());

        s.push_stage(
            "scheduler-progress:execute",
            "Scheduler step 2/16",
            "running",
            "new message",
            vec![],
        );

        let messages = s.messages.get();
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            TranscriptBlock::StageUpdate {
                name,
                message,
                fold,
                ..
            } => {
                assert_eq!(name, "Scheduler step 2/16");
                assert_eq!(message, "new message");
                assert_eq!(*fold, FoldState::Expanded);
            }
            other => panic!("expected StageUpdate, got {other:?}"),
        }
    }

    #[test]
    fn run_status_transitions_maintain_running_since_anchor() {
        let s = SessionStore::new();
        assert!(s.running_since.get().is_none(), "Idle 起点为空");
        s.set_run_status(RunStatus::Running);
        let anchor = s.running_since.get().expect("Running 置位计时锚点");
        // 重复运行态事件（Question/Permission 恢复等）不得重置计时。
        s.set_run_status(RunStatus::Running);
        s.set_run_status(RunStatus::Compacting);
        assert_eq!(
            s.running_since.get(),
            Some(anchor),
            "运行态之间的迁移共享同一起点"
        );
        s.set_run_status(RunStatus::WaitingUser);
        assert!(s.running_since.get().is_none(), "等待用户清零");
        s.set_run_status(RunStatus::Running);
        assert!(
            s.running_since.get().is_some_and(|t| t >= anchor),
            "重新运行重新起表"
        );
    }

    #[test]
    fn active_tool_phase_updates_preserve_started_at() {
        let s = SessionStore::new();
        s.set_active_tool("t1", "bash", ToolPhase::Starting);
        let started = s
            .active_tools
            .get()
            .iter()
            .find(|t| t.id == "t1")
            .unwrap()
            .started_at;
        s.set_active_tool("t1", "bash", ToolPhase::Running);
        let tools = s.active_tools.get();
        let after = tools.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(after.started_at, started, "phase 推进不重置工具起点");
        assert_eq!(after.phase, ToolPhase::Running);
    }

    #[test]
    fn failed_user_message_stays_marked_not_removed() {
        // ⑤ Reasonix msg--user-failed：失败打标保留，不回收。
        let s = SessionStore::new();
        s.push_user_message("m1", "hello");
        s.mark_user_message_failed("m1");
        let msgs = s.messages.get();
        let Some(TranscriptBlock::UserPrompt {
            failed, content, ..
        }) = msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::UserPrompt { id, .. } if id == "m1"))
        else {
            panic!("失败消息必须保留在 transcript");
        };
        assert!(*failed, "失败标记置位");
        assert_eq!(content, "hello", "原文不丢");
    }

    #[test]
    fn thinking_auto_follows_until_user_overrides() {
        // Reasonix 自动跟随：思考流结束（后续非 thinking 块落地）自动收起；
        // 用户折叠/展开过则尊重用户，不再自动。
        let s = SessionStore::new();
        s.push_thinking("r1", "step one...");
        assert_eq!(
            fold_of_thinking(&s, "r1"),
            Some(FoldState::Truncated),
            "流式思考默认 3 行预览（自动跟随展开态）"
        );
        // assistant 消息落地 = 思考结束 → 自动收起
        s.push_assistant_delta("m1", "answer");
        assert_eq!(
            fold_of_thinking(&s, "r1"),
            Some(FoldState::Folded),
            "未接管时思考流结束自动收起"
        );
        // 用户接管：重新展开后再来新思考流，用户块不再被动
        s.toggle_fold(idx_of_thinking(&s, "r1")); // Folded→Truncated（user_overridden=true）
        s.push_thinking("r2", "second thought");
        s.push_assistant_delta("m2", "answer2");
        assert_eq!(
            fold_of_thinking(&s, "r1"),
            Some(FoldState::Truncated),
            "用户展开过的块不被自动收起"
        );
        assert_eq!(
            fold_of_thinking(&s, "r2"),
            Some(FoldState::Folded),
            "未接管的 r2 仍自动收起"
        );
    }

    fn fold_of_thinking(s: &SessionStore, id: &str) -> Option<FoldState> {
        s.messages.get().iter().find_map(|b| {
            if let TranscriptBlock::Thinking { id: bid, fold, .. } = b {
                (bid == id).then_some(fold.clone())
            } else {
                None
            }
        })
    }

    fn idx_of_thinking(s: &SessionStore, id: &str) -> usize {
        s.messages
            .get()
            .iter()
            .position(|b| matches!(b, TranscriptBlock::Thinking { id: bid, .. } if bid == id))
            .unwrap()
    }

    #[test]
    fn tool_call_duration_freezes_on_done_and_started_at_preserved() {
        let s = SessionStore::new();
        s.upsert_tool_call("t9", "bash", "", ToolPhase::Starting);
        s.upsert_tool_call("t9", "bash", "ls -la", ToolPhase::Running);
        s.upsert_tool_call("t9", "bash", "", ToolPhase::Done);
        let msgs = s.messages.get();
        let Some(TranscriptBlock::ToolCall {
            started_at,
            duration,
            phase,
            ..
        }) = msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::ToolCall { id, .. } if id == "t9"))
        else {
            panic!("tool call block missing");
        };
        assert_eq!(*phase, ToolPhase::Done);
        assert!(duration.is_some(), "终态必须固化耗时");
        // Done 之后同 id 再来事件（重放）不重算 duration。
        let frozen = *duration;
        s.upsert_tool_call("t9", "bash", "", ToolPhase::Done);
        let msgs = s.messages.get();
        let Some(TranscriptBlock::ToolCall {
            duration: after, ..
        }) = msgs
            .iter()
            .find(|b| matches!(b, TranscriptBlock::ToolCall { id, .. } if id == "t9"))
        else {
            panic!()
        };
        assert_eq!(*after, frozen);
        let _ = started_at;
    }

    #[test]
    fn format_elapsed_uses_compact_tiers() {
        use crate::store::types::format_elapsed;
        assert_eq!(format_elapsed(std::time::Duration::from_secs(45)), "45s");
        assert_eq!(format_elapsed(std::time::Duration::from_secs(151)), "2m31s");
        assert_eq!(
            format_elapsed(std::time::Duration::from_secs(3600 + 240)),
            "1h04m"
        );
    }

    #[test]
    fn finalized_stream_rejects_late_delta_and_snapshot() {
        let s = SessionStore::new();
        s.push_assistant_delta("m1", "final");
        s.finalize_stream_block("m", "m1");
        s.push_assistant_delta("m1", " late");
        s.apply_assistant_snapshot("m1", "rewritten");
        match &s.messages.get()[0] {
            TranscriptBlock::AssistantMsg { content, .. } => assert_eq!(content, "final"),
            _ => panic!("expected assistant message"),
        }
        s.push_thinking("r1", "reason");
        s.finalize_stream_block("r", "r1");
        s.push_thinking("r1", " late");
        match &s.messages.get()[1] {
            TranscriptBlock::Thinking { content, .. } => assert_eq!(content, "reason"),
            _ => panic!("expected thinking"),
        }
        s.mark_stream_segment_start("m", "m1");
        assert_eq!(
            s.stream_block_lifecycle("m", "m1"),
            StreamBlockLifecycle::Streaming
        );
        s.apply_assistant_snapshot("m1", " next");
        match s.messages.get().last().expect("reopened assistant") {
            TranscriptBlock::AssistantMsg { content, .. } => assert_eq!(content, " next"),
            _ => panic!("expected assistant message"),
        }
    }

    #[test]
    fn stream_lifecycle_transitions_are_segment_scoped() {
        let s = SessionStore::new();
        assert_eq!(
            s.stream_block_lifecycle("m", "x"),
            StreamBlockLifecycle::Streaming
        );
        s.finalize_stream_block("m", "x");
        assert_eq!(
            s.stream_block_lifecycle("m", "x"),
            StreamBlockLifecycle::Finalized
        );
        s.mark_stream_segment_start("m", "x");
        assert_eq!(
            s.stream_block_lifecycle("m", "x"),
            StreamBlockLifecycle::Streaming
        );
    }

    #[test]
    fn subagent_projection_rejects_cross_session_stale_and_equal_conflict() {
        let s = SessionStore::new();
        s.set_session_id("s1");
        let mk = |sid: &str, ts: Option<i64>, label: &str| agendao_api::SessionExecutionTopology {
            session_id: sid.into(),
            active_count: 1,
            done_count: 0,
            running_count: 1,
            waiting_count: 0,
            cancelling_count: 0,
            retry_count: 0,
            updated_at: ts,
            roots: vec![agendao_api::SessionExecutionNode {
                id: "sa".into(),
                kind: agendao_api::ExecutionKind::SchedulerNode,
                status: agendao_api::ExecutionStatus::Running,
                label: Some(label.into()),
                parent_id: None,
                waiting_on: None,
                recent_event: None,
                started_at: 0,
                updated_at: 0,
                metadata: Some(serde_json::json!({"subagent": true})),
                children: vec![],
            }],
        };
        assert!(!s.replace_subagent_projection(&mk("other", Some(1), "x")));
        assert!(s.replace_subagent_projection(&mk("s1", Some(2), "x")));
        assert!(!s.replace_subagent_projection(&mk("s1", Some(1), "y")));
        assert!(!s.replace_subagent_projection(&mk("s1", Some(2), "y")));
        assert!(s.replace_subagent_projection(&mk("s1", Some(2), "x")));
    }
}
