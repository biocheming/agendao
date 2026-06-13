//! Session Screen — 木+火+金+水 integration.

use revue::prelude::*;
use revue::event::Key;

use crate::bridge::api::{ApiBridge, PromptResponse};
use crate::input::{PromptAction, PromptInput};
use crate::output::TranscriptFeed;
use crate::screen::render_session_header;
use crate::store::session_store::SessionStore;
use crate::store::types::*;
use crate::telemetry::SessionSidebar;

pub struct SessionScreen {
    pub session_id: String,
    pub session: SessionStore,
    pub prompt: PromptInput,
    pub sidebar: SessionSidebar,
    pub api: Option<ApiBridge>,
}

impl SessionScreen {
    pub fn new(session_id: String, api: Option<ApiBridge>) -> Self {
        Self { session_id, session: SessionStore::new(), prompt: PromptInput::new(), sidebar: SessionSidebar::new(), api }
    }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        // Ctrl+B → toggle sidebar
        if matches!(key, Key::Char('b') | Key::Char('B')) {
            self.sidebar.toggle(); return true;
        }
        if matches!(self.session.run_status.get(), RunStatus::Sending | RunStatus::Running) {
            if matches!(key, Key::Escape) {
                self.session.run_status.set(RunStatus::Idle);
                return true;
            }
            return false;
        }
        match self.prompt.handle_key(key) {
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
        // Header (2 rows)
        render_session_header(
            &self.session.working_dir.get(),
            &self.session.title.get(),
            None, // agent — from AppStore in future
            None, // model — from AppStore in future
            ctx,
        );

        // Transcript + sidebar below header
        let msgs = self.session.messages.get();
        let blocks = TranscriptFeed::render_blocks(&msgs);

        let is_running = matches!(self.session.run_status.get(), RunStatus::Sending | RunStatus::Running);
        let hint = self.prompt.status_hint(is_running);
        let status_color = match self.session.run_status.get() {
            RunStatus::Idle => Color::rgb(86, 95, 137),
            RunStatus::Sending | RunStatus::WaitingUser => Color::rgb(224, 175, 104),
            RunStatus::Running => Color::rgb(125, 207, 255),
            RunStatus::Error(_) => Color::rgb(247, 118, 142),
        };

        // Sidebar content
        let sidebar_tree = if self.sidebar.visible {
            let token = self.session.token_usage.get();
            let cache = self.session.cache_stats.get();
            let price = self.session.pricing.get();
            let ctx_pct = self.session.context_pct.get();
            let trees = self.session.sidebar_trees.get();
            let mcp = self.session.mcp_lsp.get();
            let tools = self.session.active_tools.get();
            Some(SessionSidebar::build(&token, &cache, &price, ctx_pct, &trees, &mcp, &tools))
        } else { None };

        let mut main = hstack().gap(0);
        main = main.child(blocks);
        if let Some(tree) = sidebar_tree {
            main = main.child(tree);
        }

        let layout = vstack()
            .child(main)
            .child(Text::new(format!(" {} | Ctrl+B: sidebar", hint)).fg(status_color));
        layout.render(ctx);
    }
}

fn ts_now() -> String {
    use std::time::SystemTime;
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_millis())).unwrap_or_default()
}
