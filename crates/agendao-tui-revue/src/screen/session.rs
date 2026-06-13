//! Session Screen — 木+火+金 integration.
//!
//! Wires PromptBar (input) → API dispatch → status display.

use revue::prelude::*;
use revue::event::Key;

use crate::bridge::api::{ApiBridge, PromptResponse};
use crate::input::prompt_bar::{PromptAction, PromptBar};
use crate::store::session_store::SessionStore;
use crate::store::types::*;

pub struct SessionScreen {
    pub session_id: String,
    pub session: SessionStore,
    pub prompt_bar: PromptBar,
    pub api: Option<ApiBridge>,
}

impl SessionScreen {
    pub fn new(session_id: String, api: Option<ApiBridge>) -> Self {
        Self { session_id, session: SessionStore::new(), prompt_bar: PromptBar::new(), api }
    }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        if matches!(self.session.run_status.get(), RunStatus::Sending | RunStatus::Running) {
            if matches!(key, Key::Escape) {
                self.session.run_status.set(RunStatus::Idle);
                return true;
            }
            return false;
        }
        match self.prompt_bar.handle_key(key) {
            PromptAction::Submit(text) => { self.dispatch_prompt(text); true }
            PromptAction::None => true,
        }
    }

    fn dispatch_prompt(&mut self, text: String) {
        let msg_id = format!("user-{}", ts_now());
        self.session.push_user_message(&msg_id, &text);
        self.session.run_status.set(RunStatus::Sending);

        if let Some(ref api) = self.api {
            let sid = self.session.get_session_id().unwrap_or_else(|| {
                match api.create_session(None, None) {
                    Ok(info) => { self.session.set_session_id(&info.id); info.id }
                    Err(e) => {
                        self.session.run_status.set(RunStatus::Error(format!("Session: {}", e)));
                        String::new()
                    }
                }
            });
            if sid.is_empty() { return; }
            match api.send_prompt(&sid, text) {
                Ok(response) => {
                    let status_msg = format_status(&response);
                    let id = format!("r-{}", ts_now());
                    self.session.push_assistant_delta(&id, &status_msg);
                    self.session.run_status.set(RunStatus::Idle);
                }
                Err(e) => { self.session.run_status.set(RunStatus::Error(format!("API: {}", e))); }
            }
        } else {
            let id = format!("echo-{}", ts_now());
            self.session.push_assistant_delta(&id, &format!("[echo] {}", text));
            self.session.run_status.set(RunStatus::Idle);
        }
    }
}

fn format_status(r: &PromptResponse) -> String {
    match r.status.as_str() {
        "awaiting_user" => format!("⏳ Awaiting{}", r.pending_question_id.as_deref().map_or(String::new(), |id| format!(" ({})", id))),
        "queued" => format!("📨 Queued ({} ahead)", r.queued_count.unwrap_or(0)),
        _ => format!("✅ Sent ({})", r.status),
    }
}

impl View for SessionScreen {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;
        let transcript_bottom = area.height.saturating_sub(3);
        let msgs = self.session.messages.get();
        self.render_messages(ctx, &msgs, area.y + 1, transcript_bottom);

        // Divider
        let div_y = transcript_bottom;
        for x in 0..area.width { ctx.draw_text(x, div_y, "─", Color::rgb(59, 66, 97)); }

        // Status hint
        let status = self.session.run_status.get();
        let hint = self.prompt_bar.status_hint(&status);
        let status_color = match status {
            RunStatus::Idle => Color::rgb(86, 95, 137),
            RunStatus::Sending | RunStatus::WaitingUser => Color::rgb(224, 175, 104),
            RunStatus::Running => Color::rgb(125, 207, 255),
            RunStatus::Error(_) => Color::rgb(247, 118, 142),
        };
        ctx.draw_text(0, transcript_bottom + 1, &format!(" {}", hint), status_color);

        // Session ID
        if let Some(ref id) = self.session.get_session_id() {
            let x = area.width.saturating_sub(id.len() as u16 + 2);
            ctx.draw_text(x, transcript_bottom + 1, id, Color::rgb(86, 95, 137));
        }
    }
}

impl SessionScreen {
    fn render_messages(&self, ctx: &mut RenderContext, messages: &[TranscriptBlock], start_y: u16, max_y: u16) {
        let mut y = start_y;
        if messages.is_empty() && y < max_y {
            ctx.draw_text(ctx.area.x + 2, y, "(no messages)", Color::rgb(86, 95, 137));
            return;
        }
        for block in messages {
            if y >= max_y { break; }
            match block {
                TranscriptBlock::UserPrompt { content, .. } => {
                    y = self.render_block_lines(ctx, "> ", content, Color::rgb(125, 207, 255), y, max_y);
                }
                TranscriptBlock::AssistantMsg { content, .. } => {
                    y = self.render_block_lines(ctx, "  ", content, Color::rgb(169, 177, 214), y, max_y);
                }
                TranscriptBlock::Thinking { content, folded, .. } => {
                    let prefix = if *folded { "▶ " } else { "▼ " };
                    y = self.render_block_lines(ctx, prefix, content, Color::rgb(86, 95, 137), y, max_y);
                }
                TranscriptBlock::ToolCall { name, params, phase, .. } => {
                    let icon = match phase { ToolPhase::Starting => "○", ToolPhase::Running => "◉", ToolPhase::Done => "●" };
                    let line = format!("{} {} {}", icon, name, params);
                    ctx.draw_text(ctx.area.x + 1, y, &line, Color::rgb(224, 175, 104));
                    y += 1;
                }
                TranscriptBlock::ToolResult { name, result, is_error, .. } => {
                    let color = if *is_error { Color::rgb(247, 118, 142) } else { Color::rgb(158, 206, 106) };
                    y = self.render_block_lines(ctx, &format!("{}: ", name), result, color, y, max_y);
                }
                TranscriptBlock::SkillActivated { name, .. } => {
                    ctx.draw_text(ctx.area.x + 1, y, &format!("⚡ Skill: {}", name), Color::rgb(187, 154, 247));
                    y += 1;
                }
                TranscriptBlock::StageUpdate { name, status, .. } => {
                    ctx.draw_text(ctx.area.x + 1, y, &format!("⚙ {} — {}", name, status), Color::rgb(224, 175, 104));
                    y += 1;
                }
                TranscriptBlock::CompactionHint { before_tokens, after_tokens, .. } => {
                    ctx.draw_text(ctx.area.x + 1, y, &format!("📦 Compacted: {}→{} tokens", before_tokens, after_tokens), Color::rgb(86, 95, 137));
                    y += 1;
                }
                TranscriptBlock::ImageRef { mime, .. } => {
                    ctx.draw_text(ctx.area.x + 1, y, &format!("🖼 [{}]", mime), Color::rgb(86, 95, 137));
                    y += 1;
                }
                TranscriptBlock::SystemNotice { text, .. } => {
                    y = self.render_block_lines(ctx, "ℹ ", text, Color::rgb(86, 95, 137), y, max_y);
                }
            }
            y += 1; // gap between blocks
        }
    }

    fn render_block_lines(&self, ctx: &mut RenderContext, prefix: &str, content: &str, color: Color, mut y: u16, max_y: u16) -> u16 {
        for line in content.lines() {
            if y >= max_y { break; }
            ctx.draw_text(ctx.area.x + 1, y, &format!("{}{}", prefix, line), color);
            y += 1;
        }
        y
    }
}

fn ts_now() -> String {
    use std::time::SystemTime;
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_millis())).unwrap_or_default()
}
