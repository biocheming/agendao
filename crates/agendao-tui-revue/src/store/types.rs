//! 土 — Shared types for the state layer.
//!
//! Every type here is consumed by exactly one Signal owner.
//! No type is shared across multiple write paths.

// ── Transcript blocks (金：TranscriptFeed 唯一消费者) ──

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// Metadata for the running task list header.
#[derive(Clone, Debug, Default)]
pub struct TodoSummary {
    pub duration: String, // e.g. "19m 49s"
    pub tokens: String,   // e.g. "50.4k"
    pub phase: String,    // e.g. "still thinking"
}

/// Stable, read-only runtime facts used by the M7 prompt summary. `None`
/// means no topology snapshot has been observed for this session; it must not
/// be rendered as a fabricated zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TopologySummary {
    pub running_tools: usize,
    pub subagents: Option<usize>,
}

/// M8 bounded lifecycle for a wire stream segment. This is intentionally not
/// a logical turn-final authority; a later `start` may reopen the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamBlockLifecycle {
    Streaming,
    Finalized,
}

/// Deterministic prompt summary formatter. Missing facts are omitted.
pub fn format_details_summary(
    todo: Option<(usize, usize)>,
    running_tools: usize,
    subagents: Option<usize>,
) -> String {
    let mut parts = Vec::new();
    if let Some((done, total)) = todo {
        parts.push(format!("Todo ({done}/{total})"));
    }
    if running_tools > 0 {
        parts.push(format!(
            "{running_tools} tool{} running",
            if running_tools == 1 { "" } else { "s" }
        ));
    }
    if let Some(count) = subagents {
        if count > 0 {
            parts.push(format!(
                "{count} subagent{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }
    parts.join(" · ")
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

/// Unified diff preview attached to a tool result (edit/write/apply_patch).
/// Carried by the wire block's `display.preview = {kind:"diff", text, truncated}`
/// (block projection) - the TUI renders it with +/- line coloring instead of
/// the plain `detail` text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffPreview {
    pub text: String,
    /// Server already truncated the diff text (preview cap) — the UI must
    /// say so, otherwise users read a partial diff as complete.
    pub truncated: bool,
}

/// One file's diff stat from `FrontendEvent::DiffReplaced` (session-level
/// summary, replace semantics — each event carries the full current set).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Clone, Debug)]
pub enum TranscriptBlock {
    UserPrompt {
        id: String,
        content: String,
        fold: FoldState,
        /// Reasonix msg--user-failed 口径：发送失败的消息**原地打标保留**
        /// （✕ 引导符 + 红），不再回收——用户不丢上下文，Ctrl+R 重试。
        failed: bool,
    },
    Thinking {
        id: String,
        content: String,
        lifecycle: StreamBlockLifecycle,
        fold: FoldState,
        duration_ms: u64,
        /// Reasonix userOverridden 口径：用户手动折叠/展开过该块后，
        /// 自动跟随（流结束自动收起）不再作用于此块。
        user_overridden: bool,
    },
    ToolCall {
        id: String,
        name: String,
        params: String,
        phase: ToolPhase,
        /// 首次出现时间：Done 时固化为 `duration`（Reasonix ToolCard
        /// duration 徽标口径），Running 期间配合 params 长度显示接收量。
        started_at: std::time::Instant,
        /// 终态耗时（Done/error 固化）；重放/旧块可能为 None（不显示）。
        duration: Option<std::time::Duration>,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
        fold: FoldState,
        /// edit/write/apply_patch 的 unified diff 预览（`display.preview`，
        /// kind=="diff"）。Some 时渲染层以 diff 文本为 body（± 行着色），
        /// `result`（detail 摘要）退居序列化/复制用途。
        diff: Option<DiffPreview>,
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
        lifecycle: StreamBlockLifecycle,
        /// 长回答默认 Truncated（3 行预览 + hint），Space/点击展开。
        fold: FoldState,
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
            TranscriptBlock::Thinking { content, fold, .. } => match fold {
                FoldState::Folded => 1,
                FoldState::Truncated => {
                    let total = content.lines().count();
                    let body = FOLD_PREVIEW_LINES.min(total) as u16;
                    let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                    1 + body + extra
                }
                FoldState::Expanded => 1 + content.lines().count().max(1) as u16,
            },
            TranscriptBlock::ToolCall { params, .. } => {
                if params.is_empty() {
                    1
                } else {
                    2
                }
            }
            TranscriptBlock::ToolResult {
                result, fold, diff, ..
            } => {
                // body 口径与 screen/session.rs 的 ToolResult 分支同源：diff 预览
                // 存在时 body = diff 文本；server-truncated 恒多一行截断标注。
                let total = diff
                    .as_ref()
                    .map(|d| d.text.lines().count())
                    .unwrap_or_else(|| result.lines().count());
                let server_truncated = diff.as_ref().map(|d| d.truncated).unwrap_or(false);
                match fold {
                    FoldState::Folded => 1,
                    FoldState::Truncated => {
                        let body = FOLD_PREVIEW_LINES.min(total) as u16;
                        let extra = if total > FOLD_PREVIEW_LINES || server_truncated {
                            1
                        } else {
                            0
                        };
                        1 + body + extra
                    }
                    FoldState::Expanded => {
                        let lines = total.clamp(1, 20) as u16;
                        let extra = if total > 20 || server_truncated { 1 } else { 0 };
                        1 + lines + extra
                    }
                }
            }
            TranscriptBlock::StageUpdate { metadata, .. } => {
                let extra = metadata
                    .as_ref()
                    .map(|m| m.lines().count() as u16)
                    .unwrap_or(0);
                3 + extra
            }
            TranscriptBlock::TodoList { items, fold, .. } => match fold {
                FoldState::Folded => 1, // header only
                FoldState::Truncated => {
                    let body = FOLD_PREVIEW_LINES.min(items.len()) as u16;
                    let extra = if items.len() > FOLD_PREVIEW_LINES {
                        1
                    } else {
                        0
                    };
                    1 + body + extra
                }
                FoldState::Expanded => 1 + items.len().max(1) as u16,
            },
            TranscriptBlock::SkillActivated { .. }
            | TranscriptBlock::CompactionHint { .. }
            | TranscriptBlock::SystemNotice { .. }
            | TranscriptBlock::ImageRef { .. } => 1,
            TranscriptBlock::AssistantMsg { content, fold, .. } => {
                // Rough estimate: role label + body lines. The renderer's
                // exact height (which walks markdown segments + tables)
                // is close enough for auto-scroll math — a row or two of
                // difference won't change the "is cursor visible" answer.
                let total = content.lines().count();
                match fold {
                    FoldState::Folded => 1,
                    FoldState::Truncated => {
                        let body = FOLD_PREVIEW_LINES.min(total) as u16;
                        let extra = if total > FOLD_PREVIEW_LINES { 1 } else { 0 };
                        (body + extra).max(1)
                    }
                    FoldState::Expanded => {
                        if content.is_empty() {
                            2
                        } else {
                            total.max(1) as u16 + 1
                        }
                    }
                }
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
    /// U9：压缩相位独立成态——此前归并 Running，用户无法区分"模型在
    /// 生成"与"上下文在压缩"（压缩期无 token 流出，易被误判为挂死）。
    /// 行为口径与 Running 一致（spinner 转、Esc 可打断、stale 判定同闸），
    /// 仅展示层分流。
    Compacting,
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
    /// 首次进入该工具的时间点（phase 推进不重置）——状态栏/头部
    /// 工具耗时 chip 的锚点（Reasonix ToolCard elapsed 模式）。
    pub started_at: std::time::Instant,
}

/// 紧凑耗时：`45s` / `2m31s` / `1h04m`（Reasonix format.ts 分级口径）。
pub fn format_elapsed(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// 紧凑字符量：`87` / `2.3k` / `1.2M`（接收中指示用，Reasonix "receiving
/// args (Xk chars)" 口径——长参数流式接收时用户能看到进度，不误判卡死）。
pub fn format_chars(n: usize) -> String {
    if n < 1000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

// ── 木：输入类型 ──

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
}

#[derive(Clone, Debug)]
pub enum AttachmentKind {
    File {
        path: String,
        lines: usize,
    },
    Image {
        mime: String,
        width: u32,
        height: u32,
    },
}

// ── 金：Toast ──

/// ⑥ Reasonix toast actionLabel 口径：toast 可携带一个动作（TUI 用枚举
/// 数据驱动而非闭包——点击命中后由 keymap 单点执行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastActionKind {
    /// 重发最近失败的 prompt（与 Ctrl+R 同源，Failed 回执留存原文）。
    RetryLastPrompt,
}

impl ToastActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RetryLastPrompt => "↻ Retry",
        }
    }
}

/// ⑥ Reasonix inboxError 错误码→文案表口径：常见错误翻译成可行动的
/// 中文提示（保留原文尾部，不丢技术细节）。集中单点，UI 各处共用。
pub fn friendly_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let hint = if lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("身份验证失败")
    {
        "API key 无效或过期——Settings 里检查该 provider 的 key"
    } else if lower.contains("404") || lower.contains("not found") {
        "端点路径不存在——协议与 Base URL 组合不对（如 Anthropic 协议会拼 /messages）"
    } else if lower.contains("429") || lower.contains("余额不足") || lower.contains("rate limit")
    {
        "限流或余额不足——稍后重试或检查账户额度"
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("超时") {
        "网络超时——检查代理/镜像或稍后重试"
    } else if lower.contains("connection refused") || lower.contains("dns") {
        "无法连接服务端——检查网络/代理设置"
    } else {
        return raw.to_string();
    };
    format!("{}（{}）", hint, raw)
}

#[derive(Clone, Debug)]
pub struct ToastMsg {
    /// 单调递增 id（AppStore 计数器分配）——dismiss/命中测试按 id 定位，
    /// 不因过期 GC 移位而误删（土律：删除以身份不以位置）。
    pub id: u64,
    pub text: String,
    pub variant: ToastMsgVariant,
    /// 可选动作（⑥）：渲染层行尾 chip，点击执行（keymap 单点）。
    pub action: Option<ToastActionKind>,
    /// Wall-clock deadline (millis since UNIX epoch) after which the
    /// toast should be considered expired. The renderer reads
    /// `expires_at` and skips rendering if the deadline passed —
    /// without it toasts pile up forever and obscure the prompt area.
    pub expires_at: u64,
    /// 入队时刻（millis since UNIX epoch）——通知中心（U7③）按时间
    /// 排序/显示「x 分钟前」；渲染层不读。
    pub created_at: u64,
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
    Tools,
    Plugins,
    McpServers,
    Keybindings,
    About,
}

impl SettingsCategory {
    /// 8 分类的渲染顺序。
    pub const ALL: [Self; 8] = [
        Self::General,
        Self::ModelSettings,
        Self::Skills,
        Self::Tools,
        Self::Plugins,
        Self::McpServers,
        Self::Keybindings,
        Self::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::ModelSettings => "Model Settings",
            Self::Skills => "Skills",
            Self::Tools => "Tools",
            Self::Plugins => "Plugins",
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
            Self::Tools => "⛏",
            Self::Plugins => "⧉",
            Self::McpServers => "⚔",
            Self::Keybindings => "⌨",
            Self::About => "ℹ",
        }
    }

    /// 当前是否有具体实现。八项均已落地。
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
        /// 是否命中 `skills.disabled`（精确名或 `category/*` 通配）——
        /// server `include_disabled` 查询打标，UI 据此渲染开关态。
        disabled: bool,
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

    /// 启停态：catalog 行读 server 打标；proposal 恒 false（不参与 disabled）。
    pub fn is_disabled(&self) -> bool {
        matches!(self, Self::Catalog { disabled, .. } if *disabled)
    }

    /// 树状分组归属：proposals 恒入 `SKILLS_PROPOSALS_GROUP` 伪类目；
    /// catalog 按 `category` 字段，缺省入 `SKILLS_UNCATEGORIZED_GROUP`。
    pub fn group_name(&self) -> &str {
        match self {
            Self::Proposal { .. } => SKILLS_PROPOSALS_GROUP,
            Self::Catalog { category, .. } => category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(SKILLS_UNCATEGORIZED_GROUP),
        }
    }
}

/// pending proposals 的固定分组名（catalog category 之外的伪类目，恒排最前）。
pub const SKILLS_PROPOSALS_GROUP: &str = "Pending Proposals";
/// 无 category 的 catalog skill 归入的分组名。
pub const SKILLS_UNCATEGORIZED_GROUP: &str = "Uncategorized";

/// Settings→Skills 列表的可见行（类目头 或 源数据行）。
/// 由 [`flatten_settings_skill_rows`] 从源数据 + 折叠集派生——渲染、键盘导航、
/// 鼠标命中三方共用同一份展开结果（金律·渲染/命中同源）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsSkillLine {
    /// 类目头（可折叠）。`count` = 组内数据行数（折叠时也计入）；
    /// `disabled_count` = 组内 disabled 行数（开关态渲染用）。
    Category {
        name: String,
        count: usize,
        collapsed: bool,
        disabled_count: usize,
    },
    /// 数据行：值为 `settings_skills` 源 Vec 的下标。
    Row(usize),
}

/// 把 proposals + catalog 源行按 category 聚合成「类目头 + 组内行」的可见序列。
///
/// 排序：`SKILLS_PROPOSALS_GROUP` 恒最前；其余类目按名字典序（忽略大小写）；
/// 组内行按 label 字典序。`collapsed` 命中的类目只出类目头（数据行隐藏）。
/// 与 session tree 折叠（telemetry/session_tree.rs）同范式：折叠态独立持有，
/// 源数据不因折叠改变。
pub fn flatten_settings_skill_rows(
    rows: &[SettingsSkillRow],
    collapsed: &std::collections::HashSet<String>,
) -> Vec<SettingsSkillLine> {
    use std::collections::BTreeMap;

    // key = 小写类目名（排序/去重/折叠集匹配口径统一，避免 "A"/"a" 拆两组）；
    // value = (首次出现的原始类目名（展示用）, 组内源行下标)。
    let mut groups: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let display = row.group_name();
        groups
            .entry(display.to_ascii_lowercase())
            .or_insert_with(|| (display.to_string(), Vec::new()))
            .1
            .push(i);
    }

    let mut lines = Vec::new();
    let emit_group =
        |key: &str, display: &str, indices: &mut Vec<usize>, lines: &mut Vec<SettingsSkillLine>| {
            indices.sort_by(|&a, &b| {
                rows[a]
                    .label()
                    .to_ascii_lowercase()
                    .cmp(&rows[b].label().to_ascii_lowercase())
            });
            let is_collapsed = collapsed.contains(key);
            let disabled_count = indices.iter().filter(|&&i| rows[i].is_disabled()).count();
            lines.push(SettingsSkillLine::Category {
                name: display.to_string(),
                count: indices.len(),
                collapsed: is_collapsed,
                disabled_count,
            });
            if !is_collapsed {
                lines.extend(indices.iter().copied().map(SettingsSkillLine::Row));
            }
        };

    // proposals 伪类目恒最前（待处理优先，与旧版"proposals 排在前"语义一致）。
    let proposals_key = SKILLS_PROPOSALS_GROUP.to_ascii_lowercase();
    if let Some((display, mut indices)) = groups.remove(&proposals_key) {
        emit_group(&proposals_key, &display, &mut indices, &mut lines);
    }
    for (key, (display, mut indices)) in groups {
        emit_group(&key, &display, &mut indices, &mut lines);
    }
    lines
}

/// Settings→Tools 分类的列表行（来自 `local_list_tools` / GET `/tool/catalog`）。
/// 与 skills 的差异：无 proposal 变体；`protected` 行（facade/bridge 工具）
/// 开关锁定——禁掉它们会切断模型对其它一切工具的触达（registry 过滤也豁免）。
#[derive(Clone, Debug)]
pub struct SettingsToolRow {
    pub id: String,
    pub description: String,
    pub family: Option<String>,
    pub protected: bool,
    pub disabled: bool,
}

impl SettingsToolRow {
    pub fn label(&self) -> &str {
        &self.id
    }

    /// 树状分组归属：按 catalog metadata 的 `family`；无 family 归入
    /// `TOOLS_UNCATEGORIZED_GROUP`（此类目头不支持 `family/*` 通配启停）。
    pub fn group_name(&self) -> &str {
        self.family
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(TOOLS_UNCATEGORIZED_GROUP)
    }
}

/// 无 family 的 tool 归入的分组名。
pub const TOOLS_UNCATEGORIZED_GROUP: &str = "Uncategorized";

/// Settings→Tools 列表的可见行（类目头 或 源数据行），与 skills 树同范式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsToolLine {
    /// 类目头（可折叠）。`count` = 组内行数；`disabled_count` = 组内 disabled 行数。
    Category {
        name: String,
        count: usize,
        collapsed: bool,
        disabled_count: usize,
    },
    /// 数据行：值为 `settings_tools` 源 Vec 的下标。
    Row(usize),
}

/// 把 tool 源行按 family 聚合成「类目头 + 组内行」的可见序列。
/// 类目按名字典序（忽略大小写），组内按 id 字典序；无 proposals 伪类目。
pub fn flatten_settings_tool_rows(
    rows: &[SettingsToolRow],
    collapsed: &std::collections::HashSet<String>,
) -> Vec<SettingsToolLine> {
    use std::collections::BTreeMap;

    let mut groups: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        let display = row.group_name();
        groups
            .entry(display.to_ascii_lowercase())
            .or_insert_with(|| (display.to_string(), Vec::new()))
            .1
            .push(i);
    }

    let mut lines = Vec::new();
    for (key, (display, mut indices)) in groups {
        indices.sort_by(|&a, &b| {
            rows[a]
                .label()
                .to_ascii_lowercase()
                .cmp(&rows[b].label().to_ascii_lowercase())
        });
        let is_collapsed = collapsed.contains(&key);
        let disabled_count = indices.iter().filter(|&&i| rows[i].disabled).count();
        lines.push(SettingsToolLine::Category {
            name: display,
            count: indices.len(),
            collapsed: is_collapsed,
            disabled_count,
        });
        if !is_collapsed {
            lines.extend(indices.iter().copied().map(SettingsToolLine::Row));
        }
    }
    lines
}

/// Settings→MCP 分类的一行(与 dialog::McpEntry 字段同构,store 侧权威副本)。
/// 连接状态来自 `/mcp` 运行时;transport/command/url/enabled 来自 config.mcp
/// (refresh 时两源合并——status map 无配置字段,土律·第十条·可观测性)。
#[derive(Clone, Debug)]
pub struct SettingsMcpRow {
    pub name: String,
    pub status: String,
    pub tools: usize,
    pub resources: usize,
    pub error: Option<String>,
    /// transport 类型:`local`(command) / `remote`(url) / `unknown`(config 缺条目)。
    pub transport: String,
    pub command: Option<String>,
    pub url: Option<String>,
    /// config 侧启停(McpServerConfig::Enabled / Full.enabled);缺省 true。
    pub enabled: bool,
}

impl SettingsMcpRow {
    pub fn is_connected(&self) -> bool {
        self.status == "connected"
    }
}

/// Settings→Plugins 分类的一行（`local_list_plugins` / GET `/config/plugins`）。
#[derive(Clone, Debug)]
pub struct SettingsPluginRow {
    pub name: String,
    pub plugin_type: String,
    /// `true` = config 声明（可 DELETE /config/plugin/{key}）；
    /// `false` = 目录扫描发现（删除要去对应目录）。
    pub managed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    /// 安装途径标签（server `build_plugin_list_entries` 单点权威）。
    pub origin: String,
    /// 命中顶层 `disabled_plugins`（精确名或 `前缀/*` 通配）。
    pub disabled: bool,
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
            Self::Theme => "Cycle color themes with ← / →",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_summary_omits_missing_and_zero_facts() {
        assert_eq!(format_details_summary(None, 0, None), "");
        assert_eq!(format_details_summary(None, 0, Some(0)), "");
        assert_eq!(
            format_details_summary(Some((2, 5)), 1, Some(1)),
            "Todo (2/5) · 1 tool running · 1 subagent"
        );
        assert_eq!(
            format_details_summary(None, 2, Some(3)),
            "2 tools running · 3 subagents"
        );
    }

    fn catalog(name: &str, category: Option<&str>) -> SettingsSkillRow {
        SettingsSkillRow::Catalog {
            name: name.to_string(),
            description: String::new(),
            location: format!("/p/.agendao/skills/{}/SKILL.md", name),
            category: category.map(str::to_string),
            writable: true,
            disabled: false,
        }
    }

    fn proposal(title: &str) -> SettingsSkillRow {
        SettingsSkillRow::Proposal {
            id: title.to_string(),
            title: title.to_string(),
            status: "pending".to_string(),
            kind: "create".to_string(),
        }
    }

    #[test]
    fn friendly_error_maps_common_codes_and_passes_through_unknown() {
        assert!(friendly_error("HTTP 401 Unauthorized").contains("API key"));
        assert!(friendly_error("404 Not Found: nope").contains("协议"));
        assert!(friendly_error("429 too many requests").contains("限流"));
        assert!(friendly_error("request timeout after 30s").contains("超时"));
        assert_eq!(friendly_error("weird failure xyz"), "weird failure xyz");
        assert!(friendly_error("HTTP 401 Unauthorized").contains("401"));
    }

    #[test]
    fn format_chars_uses_compact_tiers() {
        assert_eq!(format_chars(87), "87");
        assert_eq!(format_chars(2_340), "2.3k");
        assert_eq!(format_chars(1_234_567), "1.2M");
    }
    #[test]
    fn flatten_groups_by_category_with_proposals_first() {
        let rows = vec![
            catalog("zeta", Some("chem")),
            catalog("alpha", Some("chem")),
            catalog("plain", None),
            proposal("p1"),
        ];
        let lines = flatten_settings_skill_rows(&rows, &Default::default());
        // Pending Proposals 恒最前，随后类目字典序：chem < uncategorized。
        let names: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                SettingsSkillLine::Category { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec![SKILLS_PROPOSALS_GROUP, "chem", SKILLS_UNCATEGORIZED_GROUP]
        );
        // 组内按 label 排序：alpha 在 zeta 前。
        let row_labels: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                SettingsSkillLine::Row(i) => Some(rows[*i].label()),
                _ => None,
            })
            .collect();
        assert_eq!(row_labels, vec!["p1", "alpha", "zeta", "plain"]);
    }

    #[test]
    fn flatten_hides_rows_of_collapsed_category() {
        let rows = vec![catalog("a", Some("chem")), catalog("b", Some("chem"))];
        let collapsed = std::collections::HashSet::from(["chem".to_string()]);
        let lines = flatten_settings_skill_rows(&rows, &collapsed);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            SettingsSkillLine::Category {
                name: "chem".to_string(),
                count: 2,
                collapsed: true,
                disabled_count: 0,
            }
        );
    }

    #[test]
    fn flatten_skill_category_counts_disabled_rows() {
        let mut rows = vec![catalog("a", Some("chem")), catalog("b", Some("chem"))];
        if let SettingsSkillRow::Catalog { disabled, .. } = &mut rows[1] {
            *disabled = true;
        }
        let lines = flatten_settings_skill_rows(&rows, &Default::default());
        match &lines[0] {
            SettingsSkillLine::Category {
                count,
                disabled_count,
                ..
            } => {
                assert_eq!(*count, 2);
                assert_eq!(*disabled_count, 1);
            }
            _ => panic!("首行应为类目头"),
        }
    }

    fn tool(id: &str, family: Option<&str>) -> SettingsToolRow {
        SettingsToolRow {
            id: id.to_string(),
            description: String::new(),
            family: family.map(str::to_string),
            protected: false,
            disabled: false,
        }
    }

    #[test]
    fn flatten_tools_groups_by_family_and_counts_disabled() {
        let mut rows = vec![
            tool("write", Some("filesystem_edit")),
            tool("read", Some("filesystem_edit")),
            tool("skill", None),
        ];
        rows[2].disabled = true;
        let lines = flatten_settings_tool_rows(&rows, &Default::default());
        // 类目字典序：filesystem_edit < uncategorized；组内 read < write。
        let labels: Vec<String> = lines
            .iter()
            .map(|l| match l {
                SettingsToolLine::Category { name, .. } => name.clone(),
                SettingsToolLine::Row(i) => rows[*i].label().to_string(),
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                "filesystem_edit",
                "read",
                "write",
                TOOLS_UNCATEGORIZED_GROUP,
                "skill"
            ]
        );
        match &lines[3] {
            SettingsToolLine::Category {
                count,
                disabled_count,
                ..
            } => {
                assert_eq!(*count, 1);
                assert_eq!(*disabled_count, 1);
            }
            _ => panic!("第 4 行应为类目头"),
        }
    }

    #[test]
    fn flatten_tools_hides_rows_of_collapsed_family() {
        let rows = vec![tool("a", Some("fam")), tool("b", Some("fam"))];
        let collapsed = std::collections::HashSet::from(["fam".to_string()]);
        let lines = flatten_settings_tool_rows(&rows, &collapsed);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0],
            SettingsToolLine::Category {
                name: "fam".to_string(),
                count: 2,
                collapsed: true,
                disabled_count: 0,
            }
        );
    }

    #[test]
    fn flatten_case_variant_categories_share_one_group() {
        let rows = vec![catalog("a", Some("Chem")), catalog("b", Some("chem"))];
        let lines = flatten_settings_skill_rows(&rows, &Default::default());
        let categories = lines
            .iter()
            .filter(|l| matches!(l, SettingsSkillLine::Category { .. }))
            .count();
        assert_eq!(categories, 1, "大小写变体应并入同组");
        // 展示名取首次出现的原始写法。
        match &lines[0] {
            SettingsSkillLine::Category { name, count, .. } => {
                assert_eq!(name, "Chem");
                assert_eq!(*count, 2);
            }
            _ => panic!("首行应为类目头"),
        }
    }
}
