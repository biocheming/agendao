//! 金 — Permission dialog: Allow/Deny tool execution.

use std::cell::Cell;
use revue::prelude::*;
use revue::event::Key;
use crate::theme::colors;
use crate::dialog::backdrop;

/// A clickable region in the permission dialog.
/// Updated on every render, consumed on mouse click.
#[derive(Clone, Debug)]
struct ClickTarget {
    /// Absolute row on screen (set during render).
    row: u16,
    /// Column range [start, end] inclusive.
    col_start: u16,
    col_end: u16,
    /// Which option index this target maps to (lifetime index, or n for deny).
    option_index: usize,
}

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
    /// Click targets recalculated each render. Cell allows interior
    /// mutation during render (which takes &self).
    click_targets: Cell<Vec<ClickTarget>>,
    /// Dialog position from last render, for hit-testing mouse clicks.
    dialog_origin: Cell<(u16, u16)>,
}

impl PermissionDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            requests: Vec::new(),
            selected_lifetime: 0,
            click_targets: Cell::new(Vec::new()),
            dialog_origin: Cell::new((0, 0)),
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

    /// Handle a mouse click at absolute screen coordinates (col, row).
    /// Returns the permission id + reply if a clickable option was hit.
    pub fn handle_click(&self, col: u16, row: u16) -> Option<(String, PermissionReply)> {
        if !self.visible || self.requests.is_empty() { return None; }
        let req = &self.requests[0];
        let n = req.supported_lifetimes.len();
        let targets = self.click_targets.take();
        let hit = targets.iter().find(|t| {
            row == t.row && col >= t.col_start && col <= t.col_end
        });
        let result = hit.and_then(|t| {
            if t.option_index >= n {
                Some((req.id.clone(), PermissionReply::Deny))
            } else {
                match req.supported_lifetimes.get(t.option_index) {
                    Some(PermissionLifetime::Once) => Some((req.id.clone(), PermissionReply::AllowOnce)),
                    Some(PermissionLifetime::Turn) => Some((req.id.clone(), PermissionReply::AllowTurn)),
                    Some(PermissionLifetime::Session) => Some((req.id.clone(), PermissionReply::AllowSession)),
                    None => Some((req.id.clone(), PermissionReply::Deny)),
                }
            }
        });
        // Restore targets for next event (render will overwrite anyway)
        self.click_targets.set(targets);
        result
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

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let Some(req) = self.requests.first() else { return; };

        // ── Queue position indicator ──
        let queue_hint = if self.requests.len() > 1 {
            format!(" ({}/{})", 1, self.requests.len())
        } else { String::new() };

        // ── Header: icon + type label + tool name ──
        let header_text = format!(
            "{} {} — {}{}",
            req.perm_type.icon(),
            req.tool,
            req.perm_type.label(),
            queue_hint,
        );
        let mut content = vstack().gap(0)
            .child_sized(
                Text::new(header_text).bold().fg(colors::ACCENT_YELLOW),
                1,
            )
            .child_sized(Text::new(""), 1);

        // ── Message (up to 2 lines) ──
        if !req.message.is_empty() {
            content = content.child_sized(
                Text::new(&req.message).fg(colors::FG_SECONDARY),
                2,
            );
        }

        // ── Detail field rows ──
        let mut field_rows: u16 = 0;
        if let Some(class) = req.class_label() {
            content = content.child_sized(
                hstack().gap(0)
                    .child_sized(Text::new(" Class:  ").fg(colors::FG_MUTED), 9)
                    .child_flex(Text::new(class).fg(colors::FG_PRIMARY), 1.0),
                1,
            );
            field_rows += 1;
        }
        if let Some(ref scope) = req.scope_label {
            content = content.child_sized(
                hstack().gap(0)
                    .child_sized(Text::new(" Scope:  ").fg(colors::FG_MUTED), 9)
                    .child_flex(Text::new(scope).fg(colors::FG_PRIMARY), 1.0),
                1,
            );
            field_rows += 1;
        }
        if !req.risk_tags.is_empty() {
            let tags = req.risk_tags.join(", ");
            content = content.child_sized(
                hstack().gap(0)
                    .child_sized(Text::new(" Risk:   ").fg(colors::FG_MUTED), 9)
                    .child_flex(Text::new(tags).fg(colors::ACCENT_RED), 1.0),
                1,
            );
            field_rows += 1;
        }
        if !req.resource.is_empty() {
            // Show the resource (command, path, URL) in muted italic
            let resource_preview = if req.resource.len() > 80 {
                format!("{}…", &req.resource.chars().take(77).collect::<String>())
            } else {
                req.resource.clone()
            };
            content = content.child_sized(
                hstack().gap(0)
                    .child_sized(Text::new(" Cmd:    ").fg(colors::FG_MUTED), 9)
                    .child_flex(Text::new(resource_preview).fg(colors::FG_SECONDARY).italic(), 1.0),
                1,
            );
            field_rows += 1;
        }

        // Spacer before action options
        content = content.child_sized(Text::new(""), 1);

        // ── Lifetime options ──
        let lifetimes = &req.supported_lifetimes;
        for (i, lt) in lifetimes.iter().enumerate() {
            let marker = if i == self.selected_lifetime { "▶" } else { " " };
            let (key, desc) = match lt {
                PermissionLifetime::Once => ("1", "Allow this request only"),
                PermissionLifetime::Turn => ("2", "Allow for this turn"),
                PermissionLifetime::Session => ("3", "Allow for this session"),
            };
            let color = if i == self.selected_lifetime { colors::ACCENT_CYAN } else { colors::FG_SECONDARY };
            content = content.child_sized(
                Text::new(format!("{} {}. {}", marker, key, desc)).fg(color),
                1,
            );
        }

        // ── Deny option ──
        let deny_idx = lifetimes.len();
        let deny_selected = self.selected_lifetime == deny_idx;
        let deny_marker = if deny_selected { "▶" } else { " " };
        let deny_color = if deny_selected { colors::ACCENT_RED } else { colors::FG_SECONDARY };
        content = content.child_sized(
            Text::new(format!("{} 0. Deny", deny_marker)).fg(deny_color),
            1,
        );

        // ── Compute total height ──
        // The backdrop's vstack has gap(1) between content and footer.
        // Total dialog height = top border(1) + content rows + gap(1) + footer(1) + bottom border(1)
        //                     = content_rows + 4
        let msg_rows: u16 = if req.message.is_empty() { 0 } else { 2 };
        let content_rows: u16 = 1 /* header */ + 1 /* spacer */ + msg_rows + field_rows
            + 1 /* spacer */ + lifetimes.len() as u16 + 1 /* deny */;
        let h = content_rows + 4;

        // ── Risk-based border color ──
        let border_color = if req.risk_tags.iter().any(|t| t.contains("dangerous") || t.contains("destructive")) {
            colors::ACCENT_RED
        } else if req.permission_class.as_deref() == Some("external_access") {
            colors::E_AMBER
        } else {
            colors::ACCENT_YELLOW
        };

        // ── Record click targets for mouse support ──
        let area = ctx.area;
        let w: u16 = 64.min(area.width.saturating_sub(4));
        let dialog_h: u16 = h.min(area.height.saturating_sub(4));
        let dialog_x = (area.width.saturating_sub(w)) / 2;
        let dialog_y = (area.height.saturating_sub(dialog_h)) / 2;
        // Content starts after top border (1 row)
        let content_y = dialog_y + 1;
        // Options start offset within content: header(1) + spacer(1) + msg + fields + spacer(1)
        let options_start = content_y + 1 + 1 + msg_rows + field_rows + 1;
        let inner_w = w.saturating_sub(2); // minus left+right border
        let mut targets = Vec::new();
        for (i, _) in lifetimes.iter().enumerate() {
            targets.push(ClickTarget {
                row: options_start + i as u16,
                col_start: dialog_x + 1,
                col_end: dialog_x + 1 + inner_w,
                option_index: i,
            });
        }
        // Deny option
        targets.push(ClickTarget {
            row: options_start + lifetimes.len() as u16,
            col_start: dialog_x + 1,
            col_end: dialog_x + 1 + inner_w,
            option_index: lifetimes.len(),
        });
        self.click_targets.set(targets);
        self.dialog_origin.set((dialog_x, dialog_y));

        backdrop::render_dialog(
            "Permission Required",
            border_color,
            content,
            "↑↓ navigate  Enter/y: allow  n/Esc: deny",
            ctx, 64, h,
        );
    }
}

#[derive(Clone, Debug)]
pub enum PermissionReply { AllowOnce, AllowTurn, AllowSession, Deny }
