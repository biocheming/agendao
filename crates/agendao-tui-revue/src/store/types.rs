//! 土 — Shared types for the state layer.
//!
//! Every type here is consumed by exactly one Signal owner.
//! No type is shared across multiple write paths.

// ── Transcript blocks (金：TranscriptFeed 唯一消费者) ──

#[derive(Clone, Debug, PartialEq)]
pub enum TodoStatus { Pending, InProgress, Completed, Cancelled }

#[derive(Clone, Debug)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// Metadata for the running task list header.
#[derive(Clone, Debug, Default)]
pub struct TodoSummary {
    pub duration: String,    // e.g. "19m 49s"
    pub tokens: String,      // e.g. "50.4k"
    pub phase: String,       // e.g. "still thinking"
}

/// Three-state fold for transcript blocks.
///
/// - `Folded`   — role label + one-line summary (current "closed" state)
/// - `Truncated` — role label + first N lines + "… +M more" hint (DEFAULT)
/// - `Expanded`  — full content, no truncation
#[derive(Clone, Debug, PartialEq)]
pub enum FoldState {
    Folded,
    Truncated,
    Expanded,
}

impl FoldState {
    pub fn next(&self) -> Self {
        match self {
            Self::Folded => Self::Truncated,
            Self::Truncated => Self::Expanded,
            Self::Expanded => Self::Folded,
        }
    }
}

#[derive(Clone, Debug)]
pub enum TranscriptBlock {
    UserPrompt {
        id: String,
        content: String,
        fold: FoldState,
    },
    Thinking {
        id: String,
        content: String,
        fold: FoldState,
        duration_ms: u64,
    },
    ToolCall {
        id: String,
        name: String,
        params: String,
        phase: ToolPhase,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
        fold: FoldState,
    },
    SkillActivated {
        id: String,
        name: String,
    },
    /// Task/todo list emitted during execution.
    TodoList {
        id: String,
        /// Individual todo items with status.
        items: Vec<TodoItem>,
        fold: FoldState,
        /// Running header summary: duration, token count, phase.
        summary: Option<TodoSummary>,
    },
    StageUpdate {
        id: String,
        name: String,
        status: String,
        /// Optional JSON metadata rendered via JsonViewer
        metadata: Option<String>,
    },
    AssistantMsg {
        id: String,
        content: String,
    },
    ImageRef {
        id: String,
        mime: String,
    },
    CompactionHint {
        id: String,
        before_tokens: u64,
        after_tokens: u64,
    },
    SystemNotice {
        id: String,
        text: String,
    },
}

impl TranscriptBlock {
    /// Rough row estimate for **auto-scroll math only** (e.g. keeping the
    /// cursor block in view). NOT a mirror of screen-layer height — the
    /// screen layer's `layout_block` is the precise truth used for render
    /// layout; this is a deliberately coarse approximation (e.g.
    /// AssistantMsg uses raw line count, not markdown line count) because
    /// scroll only needs "is cursor in viewport", where a row or two of
    /// difference is irrelevant. Kept in the store layer because store
    /// cannot depend upward on screen.
    pub fn height(&self) -> u16 {
        const FOLD_PREVIEW_LINES: usize = 3;
        match self {
            TranscriptBlock::UserPrompt { content, fold, .. } => {
                let total = content.lines().count();
                match fold {
                    FoldState::Folded => 1, // role label only (inline summary)
                    FoldState::Truncated => {
                        let body = FOLD_PREVIEW_LINES.min(total) as u16;
                        let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                        1 + body + extra
                    }
                    FoldState::Expanded => total.max(1) as u16 + 1,
                }
            }
            TranscriptBlock::Thinking { content, fold, .. } => {
                match fold {
                    FoldState::Folded => 1,
                    FoldState::Truncated => {
                        let total = content.lines().count();
                        let body = FOLD_PREVIEW_LINES.min(total) as u16;
                        let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                        1 + body + extra
                    }
                    FoldState::Expanded => 1 + content.lines().count().max(1) as u16,
                }
            }
            TranscriptBlock::ToolCall { params, .. } => {
                if params.is_empty() { 1 } else { 2 }
            }
            TranscriptBlock::ToolResult { result, fold, .. } => {
                match fold {
                    FoldState::Folded => 1,
                    FoldState::Truncated => {
                        let total = result.lines().count();
                        let body = FOLD_PREVIEW_LINES.min(total) as u16;
                        let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                        1 + body + extra
                    }
                    FoldState::Expanded => {
                        let lines = result.lines().count().min(20).max(1) as u16;
                        let extra = if result.lines().count() > 20 { 1 } else { 0 };
                        1 + lines + extra
                    }
                }
            }
            TranscriptBlock::StageUpdate { metadata, .. } => {
                let extra = metadata.as_ref().map(|m| m.lines().count() as u16).unwrap_or(0);
                3 + extra
            }
            TranscriptBlock::TodoList { items, fold, .. } => match fold {
                FoldState::Folded => 1, // header only
                FoldState::Truncated => {
                    let body = FOLD_PREVIEW_LINES.min(items.len()) as u16;
                    let extra = if items.len() > FOLD_PREVIEW_LINES { 1 } else { 0 };
                    1 + body + extra
                }
                FoldState::Expanded => 1 + items.len().max(1) as u16,
            },
            TranscriptBlock::SkillActivated { .. }
            | TranscriptBlock::CompactionHint { .. }
            | TranscriptBlock::SystemNotice { .. }
            | TranscriptBlock::ImageRef { .. } => 1,
            TranscriptBlock::AssistantMsg { content, .. } => {
                // Rough estimate: role label + body lines. The renderer's
                // exact height (which walks markdown segments + tables)
                // is close enough for auto-scroll math — a row or two of
                // difference won't change the "is cursor visible" answer.
                if content.is_empty() { 2 } else { content.lines().count().max(1) as u16 + 1 }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolPhase {
    Starting,
    Running,
    Done,
}

// ── 运行状态 ──

#[derive(Clone, Debug, PartialEq)]
pub enum RunStatus {
    Idle,
    Sending,
    Running,
    WaitingUser,
    Error(String),
}

// ── 水：遥测类型（Sidebar 各面板独立消费） ──

#[derive(Clone, Debug, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub total: u64,
    pub cache_read: u64,
    pub cache_miss: u64,
    pub cache_write: u64,
    /// Latest turn context tokens (non-cumulative, for meter bar)
    pub context_tokens: u64,
    /// Cumulative total cost in USD
    pub total_cost: f64,
}

#[derive(Clone, Debug, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Pricing {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    pub total: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SidebarTrees {
    pub session_nodes: Vec<TreeNode>,
    pub workspace_nodes: Vec<TreeNode>,
}

#[derive(Clone, Debug)]
pub struct TreeNode {
    pub label: String,
    pub depth: u8,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
    pub intent: Option<TreeIntent>,
}

#[derive(Clone, Debug)]
pub enum TreeIntent {
    NavigateSession(String),
    OpenFile(String),
}

#[derive(Clone, Debug, Default)]
pub struct McpLspInfo {
    pub mcp_connected: usize,
    pub mcp_total: usize,
    pub lsp_active: Vec<String>,
}

// ── 火：运行时类型 ──

#[derive(Clone, Debug)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
    pub phase: ToolPhase,
}

// ── 木：输入类型 ──

#[derive(Clone, Debug, PartialEq)]
pub enum PromptMode {
    Normal,
    Shell,
    Slash(String), // slash query text
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
}

#[derive(Clone, Debug)]
pub enum AttachmentKind {
    File { path: String, lines: usize },
    Image { mime: String, width: u32, height: u32 },
}

// ── 金：Dialog 类型 ──

#[derive(Clone, Debug, PartialEq)]
pub enum DialogKind {
    Alert,
    Help,
    ModelSelect,
    AgentSelect,
    SessionList,
    Permission,
    Question,
    SlashPopup,
    ThemeList,
    ProviderManager,
    Timeline,
}

// ── 金：Toast ──

#[derive(Clone, Debug)]
pub struct ToastMsg {
    pub text: String,
    pub variant: ToastMsgVariant,
    /// Wall-clock deadline (millis since UNIX epoch) after which the
    /// toast should be considered expired. The renderer reads
    /// `expires_at` and skips rendering if the deadline passed —
    /// without it toasts pile up forever and obscure the prompt area.
    pub expires_at: u64,
}

#[derive(Clone, Debug)]
pub enum ToastMsgVariant {
    Success,
    Error,
    Info,
    /// Soft warning — used for "this didn't fail but you should know why"
    /// signals like "Provider not connected — selection blocked". Renders
    /// in the same accent_yellow band as Sending status.
    Warning,
}

// ── 模型/Agent 信息 ──

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub variants: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AgentInfo {
    pub name: String,
    pub display_name: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct SessionListItem {
    pub id: String,
    pub title: String,
    pub run_status: Option<String>,
}
