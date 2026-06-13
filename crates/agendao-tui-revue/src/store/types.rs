//! 土 — Shared types for the state layer.
//!
//! Every type here is consumed by exactly one Signal owner.
//! No type is shared across multiple write paths.

// ── Transcript blocks (金：TranscriptFeed 唯一消费者) ──

#[derive(Clone, Debug)]
pub enum TranscriptBlock {
    UserPrompt {
        id: String,
        content: String,
        folded: bool,
    },
    Thinking {
        id: String,
        content: String,
        folded: bool,
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
        folded: bool,
    },
    SkillActivated {
        id: String,
        name: String,
    },
    StageUpdate {
        id: String,
        name: String,
        status: String,
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
    pub total: u64,
    pub cache_read: u64,
    pub cache_miss: u64,
    pub cache_write: u64,
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
}

#[derive(Clone, Debug)]
pub enum ToastMsgVariant {
    Success,
    Error,
    Info,
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
