//! Session Screen — 木+火+金 integration.
//!
//! Wires PromptBar (input) → API dispatch → status display.

use revue::prelude::*;
use revue::event::Key;

use crate::bridge::api::{ApiBridge, PromptResponse};
use crate::input::prompt_bar::{PromptAction, PromptBar};
use crate::store::session_store::{MessageRole, RunStatus, SessionStore, TranscriptMessage};

/// Session screen: the main interactive view for a conversation.
pub struct SessionScreen {
    pub session_id: String,
    pub session: SessionStore,
    pub prompt_bar: PromptBar,
    pub api: Option<ApiBridge>,
}

impl SessionScreen {
    pub fn new(session_id: String, api: Option<ApiBridge>) -> Self {
        Self {
            session_id,
            session: SessionStore::new(),
            prompt_bar: PromptBar::new(),
            api,
        }
    }

    /// Handle key events. Returns true if state changed.
    pub fn handle_key(&mut self, key: &Key) -> bool {
        if matches!(self.session.run_status.get(), RunStatus::Sending | RunStatus::Running) {
            if matches!(key, Key::Escape) {
                self.session.run_status.set(RunStatus::Idle);
                return true;
            }
            return false;
        }

        match self.prompt_bar.handle_key(key) {
            PromptAction::Submit(text) => {
                self.dispatch_prompt(text);
                true
            }
            PromptAction::None => true,
        }
    }

    fn dispatch_prompt(&mut self, text: String) {
        let msg_id = format!("user-{}", ts_now());
        self.session.add_user_message(&text, &msg_id);
        self.session.run_status.set(RunStatus::Sending);

        if let Some(ref api) = self.api {
            let sid = self.session.get_session_id().unwrap_or_else(|| {
                match api.create_session(None, None) {
                    Ok(info) => {
                        self.session.set_session_id(&info.id);
                        info.id
                    }
                    Err(e) => {
                        self.session.run_status
                            .set(RunStatus::Error(format!("Session: {}", e)));
                        return String::new();
                    }
                }
            });

            if sid.is_empty() {
                return;
            }

            match api.send_prompt(&sid, text) {
                Ok(response) => {
                    let status_msg = format_status(&response);
                    let id = format!("r-{}", ts_now());
                    self.session.append_message_text(&id, &status_msg);
                    self.session.finalize_message(&id);
                    self.session.run_status.set(RunStatus::Idle);
                }
                Err(e) => {
                    self.session.run_status
                        .set(RunStatus::Error(format!("API: {}", e)));
                }
            }
        } else {
            // No API — echo mode for dev
            let id = format!("echo-{}", ts_now());
            self.session.append_message_text(&id, &format!("[echo] {}", text));
            self.session.finalize_message(&id);
            self.session.run_status.set(RunStatus::Idle);
        }
    }
}

fn format_status(r: &PromptResponse) -> String {
    match r.status.as_str() {
        "awaiting_user" => format!(
            "⏳ Awaiting user input{}",
            r.pending_question_id.as_deref().map_or(String::new(), |id| format!(" ({})", id))
        ),
        "queued" => format!("📨 Queued ({} ahead)", r.queued_count.unwrap_or(0)),
        _ => format!("✅ Sent (status: {})", r.status),
    }
}

impl View for SessionScreen {
    fn render(&self, ctx: &mut RenderContext) {
        let area = ctx.area;

        // ── Transcript (top area) ──
        let transcript_bottom = area.height.saturating_sub(3);
        let msgs = self.session.messages.get();
        self.render_messages(ctx, &msgs, area.y + 1, transcript_bottom);

        // ── Divider ──
        let div_y = transcript_bottom;
        for x in 0..area.width {
            ctx.draw_text(x, div_y, "─", Color::rgb(59, 66, 97));
        }

        // ── Status hint ──
        let status = self.session.run_status.get();
        let hint = self.prompt_bar.status_hint(&status);
        let status_color = match status {
            RunStatus::Idle => Color::rgb(86, 95, 137),
            RunStatus::Sending => Color::rgb(224, 175, 104),
            RunStatus::Running => Color::rgb(125, 207, 255),
            RunStatus::WaitingUser => Color::rgb(224, 175, 104),
            RunStatus::Error(_) => Color::rgb(247, 118, 142),
        };
        ctx.draw_text(0, transcript_bottom + 1, &format!(" {}", hint), status_color);

        // ── Sidebar hint ──
        let sid = self.session.get_session_id();
        if let Some(ref id) = sid {
            ctx.draw_text(area.width.saturating_sub(id.len() as u16 + 2),
                transcript_bottom + 1, id, Color::rgb(86, 95, 137));
        }
    }
}

impl SessionScreen {
    fn render_messages(
        &self,
        ctx: &mut RenderContext,
        messages: &[TranscriptMessage],
        start_y: u16,
        max_y: u16,
    ) {
        let mut y = start_y;
        if messages.is_empty() && y < max_y {
            ctx.draw_text(ctx.area.x + 2, y, "(no messages)", Color::rgb(86, 95, 137));
            return;
        }
        for msg in messages {
            if y >= max_y {
                break;
            }
            let (prefix, color) = match msg.role {
                MessageRole::User => ("> ", Color::rgb(125, 207, 255)),
                MessageRole::Assistant => ("  ", Color::rgb(169, 177, 214)),
                MessageRole::Thinking => ("💭 ", Color::rgb(86, 95, 137)),
                MessageRole::Stage => ("⚙  ", Color::rgb(224, 175, 104)),
                MessageRole::System => ("  ", Color::rgb(86, 95, 137)),
            };
            for line in msg.content.lines() {
                if y >= max_y {
                    break;
                }
                let display = format!("{}{}", prefix, line);
                ctx.draw_text(ctx.area.x + 1, y, &display, color);
                y += 1;
            }
            y += 1;
        }
    }
}

fn ts_now() -> String {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_millis()))
        .unwrap_or_default()
}
