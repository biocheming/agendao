//! 金 — Permission dialog: Allow/Deny tool execution.
//!
//! Mirrors old TUI's permission.rs: PermissionType (12 icons),
//! PermissionLifetime (Once/Turn/Session), supported_lifetimes hint.

use revue::prelude::*;
use revue::event::Key;

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
    pub id: String, pub tool: String, pub message: String,
    pub perm_type: PermissionType,
    pub supported_lifetimes: Vec<PermissionLifetime>,
}

pub struct PermissionDialog {
    pub visible: bool,
    requests: Vec<PermissionRequest>,
    selected_lifetime: usize,
}

impl PermissionDialog {
    pub fn new() -> Self { Self { visible: false, requests: Vec::new(), selected_lifetime: 0 } }

    pub fn add_request(&mut self, req: PermissionRequest) {
        self.requests.push(req); self.selected_lifetime = 0; self.visible = true;
    }

    pub fn pending_count(&self) -> usize { self.requests.len() }

    pub fn handle_key(&mut self, key: &Key) -> Option<PermissionReply> {
        if !self.visible || self.requests.is_empty() { return None; }
        let req = &self.requests[0];
        let n = req.supported_lifetimes.len();
        match key {
            Key::Up => { self.selected_lifetime = self.selected_lifetime.saturating_sub(1); None }
            Key::Down => { self.selected_lifetime = (self.selected_lifetime + 1).min(n.saturating_sub(1)); None }
            Key::Enter => {
                let reply = match req.supported_lifetimes.get(self.selected_lifetime) {
                    Some(PermissionLifetime::Once) => PermissionReply::AllowOnce,
                    Some(PermissionLifetime::Turn) => PermissionReply::AllowTurn,
                    Some(PermissionLifetime::Session) => PermissionReply::AllowSession,
                    None => PermissionReply::Deny,
                };
                let _id = req.id.clone(); self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                Some(reply)
            }
            Key::Escape | Key::Char('d') => {
                let _id = req.id.clone(); self.requests.remove(0);
                if self.requests.is_empty() { self.visible = false; }
                Some(PermissionReply::Deny)
            }
            _ => None,
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible { return; }
        let Some(req) = self.requests.first() else { return; };
        let queue_hint = if self.requests.len() > 1 { format!(" ({} more)", self.requests.len() - 1) } else { String::new() };

        let mut content = vstack().gap(1)
            .child(Text::new(format!("{} {} {}", req.perm_type.icon(), req.tool, queue_hint)).bold().fg(Color::rgb(224, 175, 104)))
            .child(Text::new(&req.message).class("DialogBody"));

        let lifetimes = &req.supported_lifetimes;
        for (i, lt) in lifetimes.iter().enumerate() {
            let marker = if i == self.selected_lifetime { "▶" } else { " " };
            let (key, desc) = match lt {
                PermissionLifetime::Once => ("1", "this request only"),
                PermissionLifetime::Turn => ("2", "this turn"),
                PermissionLifetime::Session => ("3", "this session"),
            };
            let color = if i == self.selected_lifetime { Color::rgb(125, 207, 255) } else { Color::rgb(169, 177, 214) };
            content = content.child(Text::new(format!("{} {}. {} — {}", marker, key, desc, desc)).fg(color));
        }
        content = content.child(Text::new("d/Esc: deny").fg(Color::rgb(86, 95, 137)));

        let dialog = Border::rounded()
            .title(" Permission Required ")
            .fg(Color::rgb(224, 175, 104))
            .child(content);

        let w = 54u16.min(ctx.area.width - 4);
        let h = (lifetimes.len() as u16 + 6).min(ctx.area.height - 4);
        let x = (ctx.area.width - w) / 2;
        let y = (ctx.area.height - h) / 2;
        revue::widget::positioned(dialog).x(x as i16).y(y as i16).width(w).height(h).render(ctx);
    }
}

#[derive(Clone, Debug)]
pub enum PermissionReply { AllowOnce, AllowTurn, AllowSession, Deny }
