//! 土 — Shared types for the state layer.
//!
//! Every type here is consumed by exactly one Signal owner.
//! No type is shared across multiple write paths.

// ── Transcript blocks (金：TranscriptFeed 唯一消费者) ──

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// 金：Session header dir 点击弹出的全路径 tooltip（click-to-reveal，无 motion tracking）。
/// path=working_dir 全路径；x/y=toast 左上角屏幕坐标（点击时算好存入，render 只读）。
#[derive(Clone, Debug)]
pub struct DirTooltip {
    pub path: String,
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Debug)]
pub struct SessionListItem {
    pub id: String,
    pub title: String,
    pub run_status: Option<String>,
    /// Fork 父会话 id;`None` = 根会话。sidebar 导航树组树用(对齐 web parent_id)。
    pub parent_id: Option<String>,
    /// 会话所属工作目录(canonical);与 `AppStore.working_dir` 过滤对齐。
    pub directory: String,
    /// `time.updated` 毫秒时间戳;根/子节点排序用(最近优先)。
    pub updated: i64,
}

// ── Settings screen 状态(土：AppStore 唯一所有权) ──

/// Settings 左栏分类。已落地项见 [`Self::is_implemented`];无占位灰显项
/// (土律·第十条:不假装做了未做的功能)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCategory {
    General,
    ModelSettings,
    Skills,
    McpServers,
    Keybindings,
    About,
}

impl SettingsCategory {
    /// 6 分类的渲染顺序。
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::ModelSettings,
        Self::Skills,
        Self::McpServers,
        Self::Keybindings,
        Self::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::ModelSettings => "Model Settings",
            Self::Skills => "Skills",
            Self::McpServers => "MCP Servers",
            Self::Keybindings => "Keybindings",
            Self::About => "About",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "☯",
            Self::ModelSettings => "⚒",
            Self::Skills => "✧",
            Self::McpServers => "⚔",
            Self::Keybindings => "⌨",
            Self::About => "ℹ",
        }
    }

    /// 当前是否有具体实现。六项均已落地。
    pub fn is_implemented(self) -> bool {
        true
    }
}

/// Settings→Skills 分类的列表行(catalog 或 pending proposal)。
/// 单一枚举避免 catalog/proposal 双列表分裂成两套选中态(土律·第四条)。
#[derive(Clone, Debug)]
pub enum SettingsSkillRow {
    Catalog {
        name: String,
        description: String,
        location: String,
        category: Option<String>,
        writable: bool,
    },
    Proposal {
        id: String,
        title: String,
        status: String,
        kind: String,
    },
}

impl SettingsSkillRow {
    pub fn label(&self) -> &str {
        match self {
            Self::Catalog { name, .. } => name,
            Self::Proposal { title, .. } => title,
        }
    }

    pub fn is_proposal(&self) -> bool {
        matches!(self, Self::Proposal { .. })
    }
}

/// Settings→MCP 分类的一行(与 dialog::McpEntry 字段同构,store 侧权威副本)。
#[derive(Clone, Debug)]
pub struct SettingsMcpRow {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    pub error: Option<String>,
}

impl SettingsMcpRow {
    pub fn is_connected(&self) -> bool {
        self.status == "connected"
    }
}

/// General 分类的可交互行(木律·唯一输入权威)。每行对应一个已有 UI 偏好
/// signal;keymap 的 body 处理器把行动作**复用**到 `execute_slash_action` 的
/// 既有 toggle 权威(`ToggleThinking`/`ToggleAppearance`/...),不新增第二写路径
/// (土律·第四条·单点权威 + 木克土:输入变体复用同一权威)。
///
/// 渲染端(`screen::settings`)读同一批 signal 显示当前值,与 toggle 写入同源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneralRow {
    ShowThinking,
    ShowScrollbar,
    ShowHeader,
    ShowTips,
    CompactDensity,
    Theme,
}

impl GeneralRow {
    pub const ALL: [Self; 6] = [
        Self::ShowThinking,
        Self::ShowScrollbar,
        Self::ShowHeader,
        Self::ShowTips,
        Self::CompactDensity,
        Self::Theme,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ShowThinking => "Show thinking blocks",
            Self::ShowScrollbar => "Show scrollbar",
            Self::ShowHeader => "Show session header",
            Self::ShowTips => "Show input tips",
            Self::CompactDensity => "Compact density",
            Self::Theme => "Theme",
        }
    }

    /// 一行说明(渲染在 label 右侧或下方,帮助新用户理解每项作用)。
    pub fn description(self) -> &'static str {
        match self {
            Self::ShowThinking => "Reasoning/thinking segments in the transcript",
            Self::ShowScrollbar => "Scrollbar rail on the transcript",
            Self::ShowHeader => "Title/dir header row above the transcript",
            Self::ShowTips => "Hint line above the prompt input",
            Self::CompactDensity => "Remove blank lines between transcript blocks",
            Self::Theme => "Dark / light appearance",
        }
    }
}

/// Settings 三栏当前焦点(Tab 循环切换;影响 ↑/↓ 行为)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsFocusPane {
    Categories,
    Providers,
    Details,
}

impl SettingsFocusPane {
    pub fn next(self) -> Self {
        match self {
            Self::Categories => Self::Providers,
            Self::Providers => Self::Details,
            Self::Details => Self::Categories,
        }
    }
}
