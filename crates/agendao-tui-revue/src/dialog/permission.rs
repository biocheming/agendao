//! 金 — Permission dialog: Allow/Deny tool execution.
//!
//! 内联形态(终端内联 CLI 风格):pending permission 不再浮出居中
//! modal,而是作为 transcript 流末尾的一个顶格块渲染(像 ToolCall)。
//! 状态所有权(土)不变 —— 仍是 `PermissionDialog` 持有 pending 队列;
//! 只是把成形(金)从「浮动 modal」改成「内联 BlockLayout」。

use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::screen::BlockLayout;

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionType {
    ReadFile, WriteFile, Edit, ExecuteCommand, Bash,
    NetworkRequest, Glob, Grep, List, Task,
    WebFetch, WebSearch, CodeSearch, ExternalDirectory,
}

impl PermissionType {
    pub fn icon(&self) -> &'static str { match self {
        Self::ReadFile => "[R]", Self::WriteFile => "[W]", Self::Edit => "[E]",
        Self::ExecuteCommand => "[X]", Self::Bash => "[!]", Self::NetworkRequest => "[N]",
        Self::Glob => "[G]", Self::Grep => "[S]", Self::List => "[L]", Self::Task => "[T]",
        Self::WebFetch => "[F]", Self::WebSearch => "[Q]", Self::CodeSearch => "[C]",
        Self::ExternalDirectory => "[D]",
    }}
    pub fn label(&self) -> &'static str { match self {
        Self::ReadFile => "Read file", Self::WriteFile => "Write file", Self::Edit => "Edit file",
        Self::ExecuteCommand => "Execute command", Self::Bash => "Run shell command",
        Self::NetworkRequest => "Network request", Self::Glob => "Glob search",
        Self::Grep => "Grep search", Self::List => "List directory", Self::Task => "Task operation",
        Self::WebFetch => "Fetch web content", Self::WebSearch => "Web search",
        Self::CodeSearch => "Code search", Self::ExternalDirectory => "External directory access",
    }}
}

#[derive(Clone, Debug, PartialEq)]
pub enum PermissionLifetime { Once, Turn, Session }

/// 资源区折叠预览行数（与 transcript FoldState 的 FOLD_PREVIEW_LINES 同口径）。
const RESOURCE_PREVIEW_LINES: usize = 3;
/// 资源区缩进（"   "，3 列）。
const RESOURCE_INDENT: usize = 3;

/// 从 permission.upsert 的 input 提取可读资源（真实命令/路径/URL）。
///
/// 服务端 `permission_request_info`（agendao-server routes/permission.rs）
/// 把 input 包成元信息封套 `{permission, scope_key, patterns: [...], metadata: {...}}`，
/// 真实命令在 `metadata.command`（bash 工具 `with_metadata("command", ...)` 写入），
/// 顶层不再有 command/path —— 旧的顶层直查因此永远落空。优先级：
/// `metadata.command` → `metadata.path` → `metadata.url` → `patterns[0]`
/// （注意复数 key、数组）→ 顶层 command/path/url/pattern/query/directory
/// （兼容直传原始 input 的 server）→ 空串。
pub(crate) fn extract_resource(input: &serde_json::Value) -> String {
    let obj = match input.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    fn str_at(v: Option<&serde_json::Value>) -> Option<&str> {
        v.and_then(|v| v.as_str()).filter(|s| !s.is_empty())
    }
    if let Some(meta) = obj.get("metadata").and_then(|m| m.as_object()) {
        for key in ["command", "path", "url"] {
            if let Some(s) = str_at(meta.get(key)) {
                return s.to_string();
            }
        }
    }
    if let Some(first) = obj
        .get("patterns")
        .and_then(|p| p.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return first.to_string();
    }
    for key in ["command", "path", "url", "pattern", "query", "directory"] {
        if let Some(s) = str_at(obj.get(key)) {
            return s.to_string();
        }
    }
    String::new()
}

/// 按宽度把长资源文本（命令行）折成多行：先按空格词组贪心换行，
/// 单个无空格超长 token 按宽度硬断，绝不溢出。
pub(crate) fn wrap_resource_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let mut cur = String::new();
        for word in raw.split(' ').filter(|w| !w.is_empty()) {
            let chars: Vec<char> = word.chars().collect();
            let mut rest: &[char] = &chars;
            loop {
                let cur_w = cur.chars().count();
                let sep = if cur_w > 0 { 1 } else { 0 };
                if cur_w + sep + rest.len() <= width {
                    if sep == 1 {
                        cur.push(' ');
                    }
                    cur.extend(rest.iter());
                    break;
                }
                if cur_w == 0 {
                    // 空行仍放不下 —— 超长 token 按宽度硬断。
                    cur.extend(rest[..width].iter());
                    out.push(std::mem::take(&mut cur));
                    rest = &rest[width..];
                } else {
                    // 当前行放不下整个词 —— 换行后重试。
                    out.push(std::mem::take(&mut cur));
                }
            }
        }
        out.push(std::mem::take(&mut cur));
    }
    out
}

/// 内联 permission 块的屏幕命中矩形（render 端发布，keymap 鼠标消费）。
/// 内联块位置随 transcript 滚动而变，几何只能在渲染时确定——故由 render
/// 计算绝对 y 并发布，鼠标命中直接消费，与 dir/sidebar 命中同模式。
#[derive(Clone, Copy, Debug)]
pub struct PermissionBlockHit {
    /// 块首行的绝对屏幕 y。
    pub y_start: u16,
    /// 块总行数。
    pub height: u16,
    /// 资源区在块内的行范围（rel 起点, 行数），含折叠/展开 hint 行。
    pub resource_rows: Option<(u16, u16)>,
}

#[derive(Clone)]
pub struct PermissionRequest {
    pub id: String,
    pub tool: String,
    pub message: String,
    pub perm_type: PermissionType,
    pub supported_lifetimes: Vec<PermissionLifetime>,
    // ── Extended fields from server ──
    pub permission_class: Option<String>,
    pub scope_label: Option<String>,
    pub risk_tags: Vec<String>,
    /// The resource being requested (command text, file path, URL, etc.)
    /// Extracted from `input` JSON or `message` fallback.
    pub resource: String,
}

impl PermissionRequest {
    /// Derive a human-readable permission class label.
    pub fn class_label(&self) -> Option<&str> {
        self.permission_class.as_deref().map(|c| match c {
            "inspect_read" => "Inspect read",
            "workspace_write" => "Workspace write",
            "external_access" => "External access",
            "dangerous_exec" => "Dangerous execution",
            other => other,
        })
    }
}

pub struct PermissionDialog {
    pub visible: bool,
    requests: Vec<PermissionRequest>,
    selected_lifetime: usize,
    /// 资源区展开态按 head request id 记录 —— 队列头切换（allow/deny/新请求
    /// 入队）后自动回到折叠，无需在每个 remove 点手动重置。
    resource_expanded_for: Option<String>,
}

impl Default for PermissionDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            requests: Vec::new(),
            selected_lifetime: 0,
            resource_expanded_for: None,
        }
    }

    pub fn add_request(&mut self, req: PermissionRequest) {
        // Deduplicate: if a request with the same id already exists, skip
        if self.requests.iter().any(|r| r.id == req.id) { return; }
        self.requests.push(req); self.selected_lifetime = 0; self.visible = true;
    }

    /// Remove a request by id (e.g. when server sends PermissionRemoved).
    pub fn remove_by_id(&mut self, id: &str) {
        self.requests.retain(|r| r.id != id);
        if self.requests.is_empty() {
            self.visible = false;
            self.selected_lifetime = 0;
        } else if self.selected_lifetime >= self.requests[0].supported_lifetimes.len() {
            self.selected_lifetime = 0;
        }
    }

    /// Close the dialog without clearing pending requests.
    /// Use this for Escape / panel dismiss — the requests stay queued
    /// and the dialog re-opens on the next add_request or re-surface.
    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn pending_count(&self) -> usize { self.requests.len() }

    /// head request 的资源区当前是否展开。
    fn resource_expanded(&self) -> bool {
        self.requests.first().is_some_and(|r| {
            self.resource_expanded_for.as_deref() == Some(r.id.as_str())
        })
    }

    /// 展开/收起 head request 的资源区（Space 键与鼠标点击共用）。
    pub fn toggle_resource_fold(&mut self) {
        if let Some(head) = self.requests.first() {
            let id = head.id.clone();
            self.resource_expanded_for =
                if self.resource_expanded_for.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
        }
    }

    /// 资源区渲染计划（render 与鼠标命中同口径单点）：
    /// `Some((待显示行, hint))`；资源为空或宽度内 ≤3 行时 hint 为 None。
    fn resource_render_plan(&self, width: u16) -> Option<(Vec<String>, Option<String>)> {
        let req = self.requests.first()?;
        if req.resource.is_empty() {
            return None;
        }
        let wrap_w = (width as usize).saturating_sub(RESOURCE_INDENT);
        let lines = wrap_resource_lines(&req.resource, wrap_w);
        let total = lines.len();
        if total <= RESOURCE_PREVIEW_LINES {
            return Some((lines, None));
        }
        if self.resource_expanded() {
            Some((lines, Some("… Space/click to collapse".to_string())))
        } else {
            let shown: Vec<String> = lines.into_iter().take(RESOURCE_PREVIEW_LINES).collect();
            Some((
                shown,
                Some(format!(
                    "… +{} more lines · Space/click to expand",
                    total - RESOURCE_PREVIEW_LINES
                )),
            ))
        }
    }

    /// 资源区在块内的行范围（rel 起点, 行数），供 render 发布命中矩形。
    /// 起点 = header(1) + message(0/1)。
    pub(crate) fn resource_row_range(&self, width: u16) -> Option<(u16, u16)> {
        let req = self.requests.first()?;
        let (lines, hint) = self.resource_render_plan(width)?;
        let start = 1 + u16::from(!req.message.is_empty());
        let count = lines.len() as u16 + u16::from(hint.is_some());
        Some((start, count))
    }

    /// Handle a key. On allow/deny, return both the request id and the
    /// reply so the caller can route it back to the correct pending
    /// permission on the server. Returning only the reply leaves the
    /// caller passing `id=""`, which the server can't match to anything
    /// — the prompt loop then hangs waiting for an answer that never
    /// reaches it.
    pub fn handle_key(&mut self, key: &Key) -> Option<(String, PermissionReply)> {
        if !self.visible || self.requests.is_empty() { return None; }
        let req = &self.requests[0];
        let n = req.supported_lifetimes.len();
        // Total selectable items: lifetime options + deny option
        let total_options = n + 1;
        match key {
            Key::Up => { self.selected_lifetime = self.selected_lifetime.saturating_sub(1); None }
            Key::Down => { self.selected_lifetime = (self.selected_lifetime + 1).min(total_options.saturating_sub(1)); None }
            Key::Enter => {
                // If selected index is beyond lifetimes, it's the deny option
                if self.selected_lifetime >= n {
                    let id = req.id.clone();
                    self.requests.remove(0);
                    if self.requests.is_empty() { self.visible = false; }
                    return Some((id, PermissionReply::Deny));
                }
                let reply = match req.supported_lifetimes.get(self.selected_lifetime) {
                    Some(PermissionLifetime::Once) => PermissionReply::AllowOnce,
                    Some(PermissionLifetime::Turn) => PermissionReply::AllowTurn,
                    Some(PermissionLifetime::Session) => PermissionReply::AllowSession,
                    None => PermissionReply::Deny,
                };
                let id = req.id.clone();
                self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                Some((id, reply))
            }
            Key::Escape | Key::Char('d') | Key::Char('n') => {
                let id = req.id.clone();
                self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                Some((id, PermissionReply::Deny))
            }
            // Number keys jump to a specific lifetime + accept in one stroke
            Key::Char('0') => {
                // Deny shortcut
                let id = req.id.clone();
                self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                Some((id, PermissionReply::Deny))
            }
            Key::Char('1') if n >= 1 => {
                self.selected_lifetime = 0;
                self.synth_enter()
            }
            Key::Char('2') if n >= 2 => {
                self.selected_lifetime = 1;
                self.synth_enter()
            }
            Key::Char('3') if n >= 3 => {
                self.selected_lifetime = 2;
                self.synth_enter()
            }
            Key::Char('a') | Key::Char('y') => {
                // Quick "allow once" alias — common keymap in coding TUIs.
                self.selected_lifetime = 0;
                self.synth_enter()
            }
            // Space 展开/收起资源区（长命令 3 行折叠 ↔ 全量）。dialog 无
            // 行级 cursor（Enter 始终作用于 lifetime/deny 选项），故 Space
            // 是唯一折叠键。
            Key::Char(' ') => {
                self.toggle_resource_fold();
                None
            }
            _ => None,
        }
    }

    /// Internal helper: pretend the user pressed Enter at the current
    /// selection. Used by digit/'a' shortcuts so we can hard-code the
    /// reply mapping in one place.
    fn synth_enter(&mut self) -> Option<(String, PermissionReply)> {
        let req = self.requests.first()?;
        let reply = match req.supported_lifetimes.get(self.selected_lifetime) {
            Some(PermissionLifetime::Once) => PermissionReply::AllowOnce,
            Some(PermissionLifetime::Turn) => PermissionReply::AllowTurn,
            Some(PermissionLifetime::Session) => PermissionReply::AllowSession,
            None => return None,
        };
        let id = req.id.clone();
        self.requests.remove(0);
        if self.requests.is_empty() { self.visible = false; }
        Some((id, reply))
    }

    /// 内联成形:把 pending permission 渲染成 transcript 流末尾的一个顶格
    /// 块(`⏺ tool (label)` header + detail + ❯ allow/deny 选项),而非
    /// 居中浮层。
    ///
    /// 返回 `None` 当不可见。`width` = transcript 可用宽（内联块不走 PAD/
    /// glyph 包装，顶格全宽）。资源区按宽度换行、折叠到 3 行预览 + hint
    /// 行；鼠标命中不再省略——render 端发布 `PermissionBlockHit` 屏幕矩形，
    /// keymap 点击资源区 toggle 折叠（位置随滚动变，几何只能渲染时确定）。
    ///
    /// 视觉风格(用户定调 2026-06-16):顶格 dot 式,像 ToolCall 块 ——
    /// permission 是流末尾的独立待决策块,语义中性,不暗示附属某 tool_call
    /// (agendao 的 permission 是 server 推的独立事件,无 tool_call 锚点)。
    pub fn render_inline(&self, width: u16) -> Option<BlockLayout> {
        if !self.visible { return None; }
        let req = self.requests.first()?;

        // ── Queue position indicator ──
        let queue_hint = if self.requests.len() > 1 {
            format!(" ({}/{})", 1, self.requests.len())
        } else { String::new() };

        // ── Risk → header color (dangerous reads red, else amber) ──
        let header_color = if req.risk_tags.iter().any(|t| t.contains("dangerous") || t.contains("destructive")) {
            colors::ACCENT_RED()
        } else {
            colors::E_AMBER()
        };

        // ── Header: ⏺ tool (label) — top-level, like a ToolCall block ──
        let mut content = vstack().gap(0)
            .child_sized(
                Text::new(format!(" ⏺ {} ({}){}", req.tool, req.perm_type.label(), queue_hint))
                    .bold()
                    .fg(header_color),
                1,
            );
        let mut height: u16 = 1;

        // ── Message (indent 3) ──
        if !req.message.is_empty() {
            content = content.child_sized(
                Text::new(format!("   {}", req.message)).fg(colors::FG_SECONDARY()),
                1,
            );
            height += 1;
        }

        // ── Resource: command / path / url (indent 3, muted italic) ──
        // 按宽度换行；超 3 行折叠为预览 + "+N more lines" hint，Space/点击展开。
        if let Some((lines, hint)) = self.resource_render_plan(width) {
            for line in &lines {
                content = content.child_sized(
                    Text::new(format!("   {}", line)).fg(colors::FG_MUTED()).italic(),
                    1,
                );
                height += 1;
            }
            if let Some(hint) = hint {
                content = content.child_sized(
                    Text::new(format!("   {}", hint)).fg(colors::FG_TRACE()),
                    1,
                );
                height += 1;
            }
        }

        // ── Risk tags (if any) ──
        if !req.risk_tags.is_empty() {
            content = content.child_sized(
                Text::new(format!("   ⚠ {}", req.risk_tags.join(", "))).fg(colors::ACCENT_RED()),
                1,
            );
            height += 1;
        }

        // ── Spacer ──
        content = content.child_sized(Text::new(""), 1);
        height += 1;

        // ── Lifetime options (❯ pointer, inline CLI style) ──
        let lifetimes = &req.supported_lifetimes;
        for (i, lt) in lifetimes.iter().enumerate() {
            let marker = if i == self.selected_lifetime { "❯ " } else { "  " };
            let desc = match lt {
                PermissionLifetime::Once => "Allow this request only",
                PermissionLifetime::Turn => "Allow for this turn",
                PermissionLifetime::Session => "Allow for this session",
            };
            let color = if i == self.selected_lifetime { colors::ACCENT_CYAN() } else { colors::FG_SECONDARY() };
            content = content.child_sized(
                Text::new(format!("{}{}", marker, desc)).fg(color),
                1,
            );
            height += 1;
        }

        // ── Deny option ──
        let deny_selected = self.selected_lifetime == lifetimes.len();
        let deny_marker = if deny_selected { "❯ " } else { "  " };
        let deny_color = if deny_selected { colors::ACCENT_RED() } else { colors::FG_SECONDARY() };
        content = content.child_sized(
            Text::new(format!("{}Deny", deny_marker)).fg(deny_color),
            1,
        );
        height += 1;

        // ── Hint ──
        content = content.child_sized(
            Text::new(" ↑↓ navigate · ↵/y allow · 1-3 quick allow · 0/n/Esc deny").fg(colors::FG_MUTED()),
            1,
        );
        height += 1;

        Some(BlockLayout { height, view: content })
    }
}

#[derive(Clone, Debug)]
pub enum PermissionReply { AllowOnce, AllowTurn, AllowSession, Deny }


#[cfg(test)]
mod tests {
    use super::*;

    fn req(resource: &str) -> PermissionRequest {
        PermissionRequest {
            id: "p1".into(),
            tool: "bash".into(),
            message: String::new(),
            perm_type: PermissionType::Bash,
            supported_lifetimes: vec![PermissionLifetime::Once],
            permission_class: None,
            scope_label: None,
            risk_tags: vec![],
            resource: resource.into(),
        }
    }

    fn dialog_with(resource: &str) -> PermissionDialog {
        let mut d = PermissionDialog::new();
        d.add_request(req(resource));
        d
    }

    // ── extract_resource 优先级链 ──

    #[test]
    fn extract_metadata_command_wins() {
        let input = serde_json::json!({
            "permission": "bash",
            "patterns": ["cargo test"],
            "metadata": {"command": "cargo test -p agendao -- --nocapture"}
        });
        assert_eq!(
            extract_resource(&input),
            "cargo test -p agendao -- --nocapture"
        );
    }

    #[test]
    fn extract_metadata_path_then_url() {
        let input = serde_json::json!({"metadata": {"path": "/tmp/a.rs", "url": "http://x"}});
        assert_eq!(extract_resource(&input), "/tmp/a.rs");
        let input = serde_json::json!({"metadata": {"url": "http://x"}});
        assert_eq!(extract_resource(&input), "http://x");
    }

    #[test]
    fn extract_patterns_first_fallback() {
        // 复数 key、数组（非字符串）。
        let input = serde_json::json!({"patterns": ["src/**/*.rs", "docs/**"]});
        assert_eq!(extract_resource(&input), "src/**/*.rs");
    }

    #[test]
    fn extract_top_level_legacy() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(extract_resource(&input), "ls -la");
        let input = serde_json::json!({"directory": "/tmp"});
        assert_eq!(extract_resource(&input), "/tmp");
    }

    #[test]
    fn extract_empty_when_nothing_matches() {
        assert_eq!(extract_resource(&serde_json::json!({})), "");
        assert_eq!(
            extract_resource(&serde_json::json!({"permission": "bash", "patterns": []})),
            ""
        );
        assert_eq!(extract_resource(&serde_json::json!(null)), "");
        // 空字符串不算命中。
        assert_eq!(extract_resource(&serde_json::json!({"metadata": {"command": ""}})), "");
    }

    // ── wrap_resource_lines ──

    #[test]
    fn wrap_hard_breaks_single_long_token() {
        let token = "a".repeat(25);
        let lines = wrap_resource_lines(&token, 10);
        assert_eq!(lines, vec!["a".repeat(10), "a".repeat(10), "a".repeat(5)]);
        assert!(lines.iter().all(|l| l.chars().count() <= 10));
    }

    #[test]
    fn wrap_word_wraps_and_preserves_short_lines() {
        // 贪心词组换行 + 超长 token "agendao-tui-revue"(17) 按 15 硬断。
        let lines = wrap_resource_lines("cargo test -p agendao-tui-revue --lib", 15);
        assert_eq!(lines, vec!["cargo test -p", "agendao-tui-rev", "ue --lib"]);
        assert!(wrap_resource_lines("short", 76) == vec!["short"]);
    }

    // ── 折叠 / 展开 ──

    // width 12（wrap 宽 12-3=9）→ 折成 5 行：
    // ["git", "commit -m", "abcdef", "ghijkl", "mnopqr"]
    const LONG_CMD: &str = "git commit -m abcdef ghijkl mnopqr";

    #[test]
    fn collapsed_plan_shows_preview_plus_hint() {
        let d = dialog_with(LONG_CMD);
        let (lines, hint) = d.resource_render_plan(12).unwrap();
        assert_eq!(lines.len(), RESOURCE_PREVIEW_LINES);
        assert_eq!(hint.as_deref(), Some("… +2 more lines · Space/click to expand"));
    }

    #[test]
    fn short_resource_has_no_hint() {
        let d = dialog_with("ls -la");
        let (lines, hint) = d.resource_render_plan(80).unwrap();
        assert_eq!(lines, vec!["ls -la"]);
        assert_eq!(hint, None);
    }

    #[test]
    fn toggle_expands_and_collapses_back() {
        let mut d = dialog_with(LONG_CMD);
        let collapsed_h = d.render_inline(12).unwrap().height;
        d.toggle_resource_fold();
        let (lines, hint) = d.resource_render_plan(12).unwrap();
        assert_eq!(lines.len(), 5, "展开态显示全部折行");
        assert_eq!(hint.as_deref(), Some("… Space/click to collapse"));
        let expanded_h = d.render_inline(12).unwrap().height;
        assert!(expanded_h > collapsed_h, "展开态块更高");
        d.toggle_resource_fold();
        let (lines, _) = d.resource_render_plan(12).unwrap();
        assert_eq!(lines.len(), RESOURCE_PREVIEW_LINES, "再点收回 3 行预览");
        assert_eq!(d.render_inline(12).unwrap().height, collapsed_h);
    }

    #[test]
    fn space_key_toggles_fold_without_reply() {
        let mut d = dialog_with(LONG_CMD);
        assert!(d.handle_key(&Key::Char(' ')).is_none());
        assert!(d.resource_expanded());
        assert!(d.handle_key(&Key::Char(' ')).is_none());
        assert!(!d.resource_expanded());
        // 折叠切换不消费队列。
        assert_eq!(d.pending_count(), 1);
    }

    #[test]
    fn expansion_resets_when_head_changes() {
        let mut d = dialog_with(LONG_CMD);
        d.toggle_resource_fold();
        assert!(d.resource_expanded());
        // deny head → 下一请求（或空队列）自动回折叠。
        let _ = d.handle_key(&Key::Char('0'));
        assert!(!d.resource_expanded());
    }

    #[test]
    fn resource_row_range_offsets_past_header_and_message() {
        let mut d = dialog_with(LONG_CMD);
        // 无 message：起点 = header(1)。折叠 3 行 + hint 1 行 = 4。
        assert_eq!(d.resource_row_range(12), Some((1, 4)));
        // 有 message：起点后移 1。
        d.requests[0].message = "Allow?".into();
        assert_eq!(d.resource_row_range(12), Some((2, 4)));
        // 展开：5 行 + hint 1 行 = 6。
        d.toggle_resource_fold();
        assert_eq!(d.resource_row_range(12), Some((2, 6)));
        // 无资源：无命中区。
        let d2 = dialog_with("");
        assert_eq!(d2.resource_row_range(12), None);
    }
}
