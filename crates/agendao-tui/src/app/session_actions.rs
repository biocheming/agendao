use super::*;
use crate::context::MessagePart;
use agendao_command_render::terminal_tool_block_display::{
    build_file_items, build_image_items, summarize_block_items_inline,
};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TranscriptOptions {
    pub include_thinking: bool,
    pub include_tool_details: bool,
    pub include_metadata: bool,
}

impl App {
    pub(super) fn queue_delete_session(&mut self, session_id: String) {
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog.set_message("API unavailable.");
            self.open_alert_dialog();
            return;
        };

        let context = self.context.clone();
        std::thread::spawn(move || {
            let outcome = match client.delete_session(&session_id) {
                Ok(_) => crate::event::SessionDeleteOutcome::Succeeded,
                Err(error) => crate::event::SessionDeleteOutcome::Failed {
                    message: error.to_string(),
                },
            };
            let _ = context.emit_custom_event(crate::event::CustomEvent::SessionDeleteFinished {
                session_id,
                outcome,
            });
        });
    }

    pub(super) fn current_session_id(&self) -> Option<String> {
        self.context.current_route_session_id()
    }

    pub(super) fn handle_show_recovery_actions(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session for recovery actions.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog.set_message("API unavailable.");
            self.open_alert_dialog();
            return;
        };
        let recovery = match client.get_session_recovery(&session_id) {
            Ok(recovery) => recovery,
            Err(error) => {
                self.alert_dialog
                    .set_message(&format!("Failed to load recovery actions:\n{}", error));
                self.open_alert_dialog();
                return;
            }
        };
        let items = recovery_action_items(&recovery);
        if items.is_empty() {
            self.alert_dialog
                .set_message("No recovery actions are available for this session.");
            self.open_alert_dialog();
            return;
        }
        self.open_recovery_action_dialog_modal(items);
    }

    pub(super) fn handle_execute_recovery_action(&mut self, selector: &str) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session for recovery execution.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            self.alert_dialog.set_message("API unavailable.");
            self.open_alert_dialog();
            return;
        };
        let recovery = match client.get_session_recovery(&session_id) {
            Ok(recovery) => recovery,
            Err(error) => {
                self.alert_dialog
                    .set_message(&format!("Failed to load recovery actions:\n{}", error));
                self.open_alert_dialog();
                return;
            }
        };
        let Some(action) = resolve_recovery_action_selection(&recovery, selector) else {
            self.alert_dialog.set_message(
                "Unknown recovery action. Open /recover and select one from the list.",
            );
            self.open_alert_dialog();
            return;
        };
        match client.execute_session_recovery(
            &session_id,
            action.kind.clone(),
            action.target_id.clone(),
        ) {
            Ok(_) => {
                self.toast.show(
                    ToastVariant::Success,
                    &format!("Recovery action started: {}", action.label),
                    2500,
                );
                if self.status_dialog.is_open() {
                    self.refresh_active_status_dialog();
                }
            }
            Err(error) => {
                self.toast.show(
                    ToastVariant::Error,
                    &format!("Recovery action failed: {}", error),
                    3000,
                );
            }
        }
    }

    pub(super) fn open_session_rename_dialog(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to rename.");
            self.open_alert_dialog();
            return;
        };

        let title = self
            .context
            .session
            .read()
            .sessions
            .get(&session_id)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "New Session".to_string());
        self.open_session_rename_dialog_modal(session_id, title);
    }

    pub(super) fn open_session_export_dialog(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to export.");
            self.open_alert_dialog();
            return;
        };

        let title = self
            .context
            .session
            .read()
            .sessions
            .get(&session_id)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "New Session".to_string());
        let default_filename = default_export_filename(&title, &session_id);
        self.open_session_export_dialog_modal(session_id, default_filename);
    }

    pub(super) fn open_prompt_stash_dialog(&mut self) {
        let entries = self
            .prompt
            .stash_entries()
            .iter()
            .cloned()
            .map(|entry| StashItem {
                input: entry.input,
                created_at: entry.created_at,
            })
            .collect::<Vec<_>>();
        self.prompt_stash_dialog.set_entries(entries);
        self.open_prompt_stash_dialog_modal();
    }

    pub(super) fn open_skill_list_dialog(&mut self) {
        if let Err(err) = self.refresh_skill_list_dialog() {
            self.alert_dialog
                .set_message(&format!("Failed to refresh skills:\n{}", err));
            self.open_alert_dialog();
        }
        self.open_skill_list_dialog_modal();
        let _ = self.refresh_skill_list_detail();
    }

    pub(super) fn handle_share_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to share.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.share_session(&session_id) {
            Ok(response) => {
                let _ = Clipboard::write_text(&response.url);
                self.alert_dialog.set_message(&format!(
                    "Session shared. Link copied to clipboard:\n{}",
                    response.url
                ));
                self.open_alert_dialog();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to share session:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_unshare_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to unshare.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.unshare_session(&session_id) {
            Ok(_) => {
                self.alert_dialog
                    .set_message("Session sharing link revoked.");
                self.open_alert_dialog();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to unshare session:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_compact_session(&mut self, focus: Option<&str>) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session to compact.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.compact_session(&session_id, focus) {
            Ok(response) => {
                let _ = self.sync_session_from_server(&session_id);
                let response_message = response.message.trim();
                let message = if response_message.is_empty() {
                    if response.success {
                        focus
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(|value| format!("Session compacted around focus:\n{}", value))
                            .unwrap_or_else(|| "Session compacted successfully.".to_string())
                    } else {
                        "Nothing to compact yet.".to_string()
                    }
                } else {
                    response_message.to_string()
                };
                self.alert_dialog.set_message(&message);
                self.open_alert_dialog();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to compact session:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_undo(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session for undo.");
            self.open_alert_dialog();
            return;
        };
        let session_ctx = self.context.session.read();
        let messages = session_ctx.messages.get(&session_id);
        let last_user_msg = messages
            .and_then(|msgs| msgs.iter().rev().find(|m| m.role == MessageRole::User))
            .map(|m| (m.id.clone(), m.content.clone()));
        drop(session_ctx);

        let Some((msg_id, msg_content)) = last_user_msg else {
            self.alert_dialog.set_message("No user message to revert.");
            self.open_alert_dialog();
            return;
        };
        let Some(client) = self.context.get_api_client() else {
            return;
        };
        match client.revert_session(&session_id, &msg_id) {
            Ok(_) => {
                self.prompt.set_input(msg_content);
                let _ = self.sync_session_from_server(&session_id);
                self.alert_dialog
                    .set_message("Message reverted. Prompt restored.");
                self.open_alert_dialog();
            }
            Err(err) => {
                self.alert_dialog
                    .set_message(&format!("Failed to revert message:\n{}", err));
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_redo(&mut self) {
        let Some(_session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session for redo.");
            self.open_alert_dialog();
            return;
        };
        // Redo re-submits the current prompt content (which was restored by undo)
        let input = self.prompt.get_input().trim().to_string();
        if input.is_empty() {
            self.alert_dialog
                .set_message("Nothing to redo. Prompt is empty.");
            self.open_alert_dialog();
            return;
        }
        // Re-submit the prompt to effectively redo
        if let Err(err) = self.submit_prompt() {
            self.alert_dialog
                .set_message(&format!("Failed to redo:\n{}", err));
            self.open_alert_dialog();
        }
    }

    pub(super) fn handle_copy_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to copy.");
            self.open_alert_dialog();
            return;
        };
        match self.build_session_transcript(
            &session_id,
            TranscriptOptions {
                include_thinking: false,
                include_tool_details: true,
                include_metadata: false,
            },
        ) {
            Some(text) => {
                if let Err(err) = Clipboard::write_text(&text) {
                    self.alert_dialog
                        .set_message(&format!("Failed to copy transcript to clipboard:\n{}", err));
                    self.open_alert_dialog();
                } else {
                    self.alert_dialog
                        .set_message("Session transcript copied to clipboard.");
                    self.open_alert_dialog();
                }
            }
            None => {
                self.alert_dialog
                    .set_message("No transcript available for current session.");
                self.open_alert_dialog();
            }
        }
    }

    pub(super) fn handle_open_timeline(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog
                .set_message("No active session for timeline.");
            self.open_alert_dialog();
            return;
        };
        let session_ctx = self.context.session.read();
        let entries = session_ctx
            .messages
            .get(&session_id)
            .map(|msgs| {
                msgs.iter()
                    .map(|m| {
                        let role = match m.role {
                            MessageRole::User => "user",
                            MessageRole::Assistant => "assistant",
                            MessageRole::System => "system",
                            MessageRole::Tool => "tool",
                        };
                        let preview = m
                            .content
                            .chars()
                            .take(60)
                            .collect::<String>()
                            .replace('\n', " ");
                        TimelineEntry {
                            message_id: m.id.clone(),
                            role: role.to_string(),
                            preview,
                            timestamp: m.created_at.format("%H:%M:%S").to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(session_ctx);
        self.open_timeline_dialog_modal(entries);
    }

    pub(super) fn handle_fork_session(&mut self) {
        let Some(session_id) = self.current_session_id() else {
            self.alert_dialog.set_message("No active session to fork.");
            self.open_alert_dialog();
            return;
        };
        let session_ctx = self.context.session.read();
        let entries = session_ctx
            .messages
            .get(&session_id)
            .map(|msgs| {
                msgs.iter()
                    .map(|m| {
                        let role = match m.role {
                            MessageRole::User => "user",
                            MessageRole::Assistant => "assistant",
                            MessageRole::System => "system",
                            MessageRole::Tool => "tool",
                        };
                        let preview = m
                            .content
                            .chars()
                            .take(60)
                            .collect::<String>()
                            .replace('\n', " ");
                        ForkEntry {
                            message_id: m.id.clone(),
                            role: role.to_string(),
                            preview,
                            timestamp: m.created_at.format("%H:%M:%S").to_string(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        drop(session_ctx);
        self.open_fork_dialog_modal(session_id, entries);
    }

    pub(super) fn build_session_transcript(
        &self,
        session_id: &str,
        options: TranscriptOptions,
    ) -> Option<String> {
        let session_ctx = self.context.session.read();
        let session = session_ctx.sessions.get(session_id)?;
        let messages = session_ctx.messages.get(session_id)?;

        let mut output = String::new();
        output.push_str(&format!("# {}\n\n", session.title));
        output.push_str(&format!("Session ID: `{}`\n", session.id));
        output.push_str(&format!("Created: {}\n", session.created_at.to_rfc3339()));
        output.push_str(&format!("Updated: {}\n\n", session.updated_at.to_rfc3339()));

        if messages.is_empty() {
            output.push_str("_No messages_\n");
            return Some(output);
        }

        for message in messages {
            let role = match message.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
                MessageRole::Tool => "Tool",
            };
            output.push_str(&format!("## {}\n\n", role));
            let rendered = render_transcript_message(message, options);
            if rendered.trim().is_empty() {
                output.push_str("_Empty message_\n\n");
            } else {
                output.push_str(&rendered);
                output.push_str("\n\n");
            }
        }

        Some(output)
    }

    pub(super) fn export_session_to_file(
        &self,
        session_id: &str,
        filename: &str,
        options: TranscriptOptions,
    ) -> anyhow::Result<PathBuf> {
        let transcript = self
            .build_session_transcript(session_id, options)
            .ok_or_else(|| {
                anyhow::anyhow!("No transcript available for session `{}`", session_id)
            })?;

        let mut path = PathBuf::from(filename.trim());
        if path.as_os_str().is_empty() {
            anyhow::bail!("filename cannot be empty");
        }
        if path.is_relative() {
            let base_dir = {
                let directory = self.context.directory.read().clone();
                if directory.trim().is_empty() {
                    std::env::current_dir()?
                } else {
                    PathBuf::from(directory)
                }
            };
            path = base_dir.join(path);
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&path, transcript)?;
        Ok(path)
    }
}

fn render_transcript_message(message: &Message, options: TranscriptOptions) -> String {
    let mut parts = Vec::new();

    for part in &message.parts {
        match part {
            MessagePart::Text { text } => {
                if !text.trim().is_empty() {
                    parts.push(text.clone());
                }
            }
            MessagePart::Reasoning { text } if options.include_thinking => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(format!("[reasoning]\n{trimmed}"));
                }
            }
            MessagePart::ToolCall {
                name, arguments, ..
            } if options.include_tool_details => {
                let trimmed = arguments.trim();
                if trimmed.is_empty() {
                    parts.push(format!("[tool:{name}]"));
                } else {
                    parts.push(format!("[tool:{name}] {trimmed}"));
                }
            }
            MessagePart::ToolResult {
                result, is_error, ..
            } if options.include_tool_details => {
                let trimmed = result.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let label = if *is_error {
                    "[tool-error]"
                } else {
                    "[tool-result]"
                };
                parts.push(format!("{label} {trimmed}"));
            }
            MessagePart::File { path, mime } if options.include_tool_details => {
                parts.push(summarize_block_items_inline(&build_file_items(path, mime)));
            }
            MessagePart::Image { url } if options.include_tool_details => {
                parts.push(summarize_block_items_inline(&build_image_items(url)));
            }
            _ => {}
        }
    }

    if options.include_metadata {
        let mut metadata = Vec::new();
        if message.tokens.input > 0 {
            metadata.push(format!("input={}", message.tokens.input));
        }
        if message.tokens.output > 0 {
            metadata.push(format!("output={}", message.tokens.output));
        }
        if message.tokens.reasoning > 0 {
            metadata.push(format!("reasoning={}", message.tokens.reasoning));
        }
        if message.tokens.cache_read > 0 {
            metadata.push(format!("cache_read={}", message.tokens.cache_read));
        }
        if message.tokens.cache_write > 0 {
            metadata.push(format!("cache_write={}", message.tokens.cache_write));
        }
        if message.cost > 0.0 {
            metadata.push(format!("cost=${:.6}", message.cost));
        }
        if !metadata.is_empty() {
            parts.push(format!("[metadata] {}", metadata.join(" · ")));
        }
    }

    parts.join("\n")
}
