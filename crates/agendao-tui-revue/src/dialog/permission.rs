//! 金 — Permission dialog: Allow/Deny tool execution.
//!
//! 内联形态(Claude Code/Codex 风格):pending permission 不再浮出居中
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
}

impl PermissionDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            requests: Vec::new(),
            selected_lifetime: 0,
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
    /// 返回 `None` 当不可见。鼠标 hit-test 故意省略:内联块的屏幕位置随
    /// transcript 滚动而变,键盘是唯一可靠输入(土克水:编排可约束回流,
    /// 但内联位置不固定时鼠标语义不可靠)。
    ///
    /// 视觉风格(用户定调 2026-06-16):顶格 dot 式,像 ToolCall 块 ——
    /// permission 是流末尾的独立待决策块,语义中性,不暗示附属某 tool_call
    /// (agendao 的 permission 是 server 推的独立事件,无 tool_call 锚点)。
    pub fn render_inline(&self) -> Option<BlockLayout> {
        if !self.visible { return None; }
        let req = self.requests.first()?;

        // ── Queue position indicator ──
        let queue_hint = if self.requests.len() > 1 {
            format!(" ({}/{})", 1, self.requests.len())
        } else { String::new() };

        // ── Risk → header color (dangerous reads red, else amber) ──
        let header_color = if req.risk_tags.iter().any(|t| t.contains("dangerous") || t.contains("destructive")) {
            colors::ACCENT_RED
        } else {
            colors::E_AMBER
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
                Text::new(format!("   {}", req.message)).fg(colors::FG_SECONDARY),
                1,
            );
            height += 1;
        }

        // ── Resource: command / path / url (indent 3, muted italic) ──
        if !req.resource.is_empty() {
            let resource_preview = if req.resource.len() > 76 {
                format!("{}…", &req.resource.chars().take(73).collect::<String>())
            } else {
                req.resource.clone()
            };
            content = content.child_sized(
                Text::new(format!("   {}", resource_preview)).fg(colors::FG_MUTED).italic(),
                1,
            );
            height += 1;
        }

        // ── Risk tags (if any) ──
        if !req.risk_tags.is_empty() {
            content = content.child_sized(
                Text::new(format!("   ⚠ {}", req.risk_tags.join(", "))).fg(colors::ACCENT_RED),
                1,
            );
            height += 1;
        }

        // ── Spacer ──
        content = content.child_sized(Text::new(""), 1);
        height += 1;

        // ── Lifetime options (❯ pointer, Claude Code/Codex style) ──
        let lifetimes = &req.supported_lifetimes;
        for (i, lt) in lifetimes.iter().enumerate() {
            let marker = if i == self.selected_lifetime { "❯ " } else { "  " };
            let desc = match lt {
                PermissionLifetime::Once => "Allow this request only",
                PermissionLifetime::Turn => "Allow for this turn",
                PermissionLifetime::Session => "Allow for this session",
            };
            let color = if i == self.selected_lifetime { colors::ACCENT_CYAN } else { colors::FG_SECONDARY };
            content = content.child_sized(
                Text::new(format!("{}{}", marker, desc)).fg(color),
                1,
            );
            height += 1;
        }

        // ── Deny option ──
        let deny_selected = self.selected_lifetime == lifetimes.len();
        let deny_marker = if deny_selected { "❯ " } else { "  " };
        let deny_color = if deny_selected { colors::ACCENT_RED } else { colors::FG_SECONDARY };
        content = content.child_sized(
            Text::new(format!("{}Deny", deny_marker)).fg(deny_color),
            1,
        );
        height += 1;

        // ── Hint ──
        content = content.child_sized(
            Text::new(" ↑↓ navigate · ↵/y allow · 1-3 quick allow · 0/n/Esc deny").fg(colors::FG_MUTED),
            1,
        );
        height += 1;

        Some(BlockLayout { height, view: content })
    }
}

#[derive(Clone, Debug)]
pub enum PermissionReply { AllowOnce, AllowTurn, AllowSession, Deny }
